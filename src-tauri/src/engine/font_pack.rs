//! 由使用者字體檔建立 Minecraft 字體資源包（人性化、固定安全路徑）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use super::apply_record;
use super::jar_scan::resolve_minecraft_dir;
use super::options_txt;
use super::paths::long_path;
use super::out_layout::{ensure_font_result_layout, FONT_RESULT_DIR_NAME};
use super::pack_out::{pack_format_for_version, pack_mcmeta_value};
use super::security::{check_font_file, ensure_under_base, sanitize_folder_name};

/// 空名稱時的字體包預設顯示名（勿依賴 sanitize 的翻譯包預設「繁體中文翻譯」）。
pub(crate) const DEFAULT_FONT_PACK_NAME: &str = "繁體中文遊戲字體";
/// 未指定版本／format 時的保底（約對應 1.21）。
const DEFAULT_FONT_PACK_FORMAT: u32 = 34;
const ASCII_SKIP: &str = " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontPackResult {
    pub pack_path: String,
    pub player_summary: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontPackApplyResult {
    pub copied_path: String,
    pub backup_path: Option<String>,
    pub player_summary: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FontPackOptions {
    pub size: f32,
    pub weight: f32,
    pub shift_x: f32,
    pub shift_y: f32,
    pub oversample: f32,
}

impl Default for FontPackOptions {
    fn default() -> Self {
        Self {
            size: 11.0,
            weight: 400.0,
            shift_x: 0.0,
            shift_y: 0.5,
            oversample: 4.0,
        }
    }
}

impl FontPackOptions {
    fn normalized(&self) -> Self {
        let finite_or = |value: f32, fallback: f32| {
            if value.is_finite() {
                value
            } else {
                fallback
            }
        };
        Self {
            size: finite_or(self.size, 11.0).clamp(8.0, 24.0),
            weight: finite_or(self.weight, 400.0).clamp(100.0, 900.0),
            shift_x: finite_or(self.shift_x, 0.0).clamp(-3.0, 3.0),
            shift_y: finite_or(self.shift_y, 0.5).clamp(-3.0, 3.0),
            oversample: finite_or(self.oversample, 4.0).clamp(1.0, 8.0),
        }
    }

    fn provider_size(&self) -> f32 {
        let weight_adjustment = (self.weight - 400.0) / 400.0 * 0.9;
        (self.size + weight_adjustment).clamp(8.0, 24.0)
    }
}

/// 依來源副檔名決定資源包內字體檔名；拒絕 `.ttc`。
pub(crate) fn cjk_font_file_name(font_file: &Path) -> Result<&'static str, String> {
    let ext = font_file
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "ttf" => Ok("cjk_font.ttf"),
        "otf" => Ok("cjk_font.otf"),
        "ttc" => Err(
            "不支援 .ttc（TrueType Collection 集合字體）。請改用單一字形的 .ttf 或 .otf 檔。"
                .into(),
        ),
        _ => Err("請使用 .ttf 或 .otf 字體檔。".into()),
    }
}

fn resolve_font_pack_format(pack_format: Option<u16>, target_version: Option<&str>) -> u32 {
    if let Some(f) = pack_format {
        return u32::from(f);
    }
    if let Some(v) = target_version {
        if let Some(f) = pack_format_for_version(v) {
            return f;
        }
    }
    DEFAULT_FONT_PACK_FORMAT
}

fn font_pack_display_name(pack_name: &str) -> Result<String, String> {
    let raw = pack_name.trim();
    let for_sanitize = if raw.is_empty() {
        DEFAULT_FONT_PACK_NAME
    } else {
        raw
    };
    sanitize_folder_name(for_sanitize)
}

pub fn build_font_resource_pack_with_options(
    font_file: &Path,
    output_dir: &Path,
    pack_name: &str,
    pack_desc: &str,
    options: &FontPackOptions,
    pack_format: Option<u16>,
    target_version: Option<&str>,
) -> Result<FontPackResult, String> {
    let font_name = cjk_font_file_name(font_file)?;
    check_font_file(font_file)?;
    let name = font_pack_display_name(pack_name)?;
    let options = options.normalized();
    let desc = if pack_desc.trim().is_empty() {
        "自訂遊戲字體資源包".to_string()
    } else {
        pack_desc.trim().chars().take(120).collect()
    };

    // 字體包進專用「字體結果/resourcepacks」，不與翻譯結果混用
    let work = if output_dir
        .file_name()
        .and_then(|s| s.to_str())
        == Some(FONT_RESULT_DIR_NAME)
    {
        output_dir.to_path_buf()
    } else {
        ensure_font_result_layout(output_dir)?
    };
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let pack_root = work.join("resourcepacks").join(&name);
    ensure_under_base(&work, &pack_root)?;

    if pack_root.exists() {
        fs::remove_dir_all(&pack_root).map_err(|e| e.to_string())?;
    }

    let font_dir = pack_root
        .join("assets")
        .join("minecraft")
        .join("font")
        .join("include");
    fs::create_dir_all(&font_dir).map_err(|e| e.to_string())?;

    let dest_font = font_dir.join(font_name);
    fs::copy(font_file, &dest_font).map_err(|e| format!("複製字體失敗：{e}"))?;

    let provider_file = format!("minecraft:include/{font_name}");
    let space_advance = (options.provider_size() * 0.36).clamp(3.0, 8.0);
    let prov = serde_json::json!({
        "providers": [
            {
                "type": "space",
                "advances": {
                    " ": space_advance
                }
            },
            {
                "type": "ttf",
                "file": provider_file,
                "shift": [options.shift_x, options.shift_y],
                "size": options.provider_size(),
                "oversample": options.oversample,
                "skip": ASCII_SKIP
            }
        ]
    });
    let font_json = serde_json::to_string_pretty(&prov).unwrap() + "\n";
    let font_base = pack_root.join("assets").join("minecraft").join("font");
    fs::write(font_base.join("default.json"), &font_json).map_err(|e| e.to_string())?;
    fs::write(font_base.join("uniform.json"), &font_json).map_err(|e| e.to_string())?;

    let fmt = resolve_font_pack_format(pack_format, target_version);
    let meta = pack_mcmeta_value(target_version, fmt, &desc);
    fs::write(
        pack_root.join("pack.mcmeta"),
        serde_json::to_string_pretty(&meta).unwrap() + "\n",
    )
    .map_err(|e| e.to_string())?;

    let readme = pack_root.join("使用說明.txt");
    fs::write(
        readme,
        format!(
            "【自訂字體資源包】\n\
1. 把整個「{}」資料夾放到遊戲的 resourcepacks。\n\
2. 設定 → 資源包 → 啟用它，並把字體包拖到列表「最上方」（優先於 GUI／材質包）。\n\
3. 關閉「強制使用 Unicode 字型」（Force Unicode）後重開遊戲。\n\
4. 若字變成□方框：多半是缺字形 → 換支援繁中的字體檔再建一次。\n\
5. 若字變成 Ã/å/æ 之類怪碼：不是字體問題 → 還原套用後用本工具重產，並檢查是否有第三方語言包蓋過。\n\
6. 若字太糊或太細，可換另一個字體檔再產生一次。\n",
            name
        ),
    )
    .ok();

    Ok(FontPackResult {
        pack_path: pack_root.display().to_string(),
        player_summary: format!(
            "字體資源包已建立！\n\
• 名稱：{}\n\
• 位置：\n{}\n\n\
• 設定：大小 {:.1}、字重感 {:.0}、位移 ({:.1}, {:.1})、清晰度 {:.1}\n\n\
【怎麼用】\n\
1. 複製到遊戲 resourcepacks\n\
2. 啟用並置頂（高於 GUI／其他材質包）\n\
3. 關閉「強制使用 Unicode 字型」後重開遊戲\n\
4. □方框＝換字體；Ã/å/æ＝還原後重產翻譯／檢查第三方語言包",
            name,
            pack_root.display(),
            options.size,
            options.weight,
            options.shift_x,
            options.shift_y,
            options.oversample
        ),
    })
}

pub fn build_font_pack_str_with_options(
    font_path: &str,
    output_dir: &str,
    pack_name: &str,
    pack_desc: &str,
    options: &FontPackOptions,
    pack_format: Option<u16>,
    target_version: Option<&str>,
) -> Result<FontPackResult, String> {
    let font = PathBuf::from(font_path.trim().trim_matches('"'));
    let out = PathBuf::from(output_dir.trim().trim_matches('"'));
    build_font_resource_pack_with_options(
        &font,
        &out,
        pack_name,
        pack_desc,
        options,
        pack_format,
        target_version,
    )
}

pub fn apply_font_pack_to_instance(
    instance_path: &Path,
    font_pack_path: &Path,
) -> Result<FontPackApplyResult, String> {
    let running = super::game_process::is_game_running(instance_path);
    apply_font_pack_with_game_state(instance_path, font_pack_path, running)
}

/// 套用字體包本體。前置條件：遊戲沒開著（會刪、會寫遊戲資料夾）。
pub fn apply_font_pack_with_game_state(
    instance_path: &Path,
    font_pack_path: &Path,
    running: super::game_process::GameRunning,
) -> Result<FontPackApplyResult, String> {
    if let super::game_process::GameRunning::Yes { detail } = &running {
        return Err(format!(
            "Minecraft 正在使用這個模組整合包，現在換字體包可能讓檔案被鎖住或只換一半。\n\
請完全關閉遊戲後再套用字體包。（{detail}）"
        ));
    }
    use super::apply_guard;
    use super::mcpl_marker as mk;
    if !font_pack_path.exists() {
        return Err("找不到要套用的字體資源包。請先建立字體包。".into());
    }
    if !font_pack_path.is_dir() && !font_pack_path.is_file() {
        return Err("字體資源包路徑必須是資料夾或 .zip 檔。".into());
    }
    let mc = resolve_minecraft_dir(instance_path)?;
    let name = font_pack_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "字體資源包名稱無法辨識。".to_string())?
        .to_string();
    let resourcepacks = mc.join("resourcepacks");
    let dest = resourcepacks.join(&name);
    if same_path(font_pack_path, &dest) {
        return Ok(FontPackApplyResult {
            copied_path: dest.display().to_string(),
            backup_path: None,
            player_summary: format!("字體資源包已經在這個遊戲的資源包資料夾裡：\n{}", dest.display()),
        });
    }

    // 前置條件：紀錄讀得到（`.mcpl` 被刪時先認回原本的識別碼），才確認或建立識別碼
    let (ctx, mut knowledge) = super::apply_identity::load_then_begin(&mc, None)?;
    let record = &mut knowledge.record;

    // 新字體包的每個檔：先算好標記（相對路徑、內容指紋）
    let new_files: Vec<(PathBuf, String)> = if font_pack_path.is_dir() {
        walkdir::WalkDir::new(font_pack_path)
            .sort_by_file_name()
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| {
                let rel = e.path().strip_prefix(font_pack_path).ok()?;
                Some((e.path().to_path_buf(), apply_record::rel_key(&mc, &dest.join(rel))))
            })
            .collect()
    } else {
        vec![(font_pack_path.to_path_buf(), apply_record::rel_key(&mc, &dest))]
    };

    // 同名的舊字體包：工具寫的可以直接換掉；不是工具寫的，先移入隔離區（附標記）才能動
    let mut quarantined = Vec::new();
    let mut to_quarantine = Vec::new();
    let mut unconfirmed = Vec::new();
    if long_path(&dest).exists() {
        // 列舉舊字體包的檔案：任何一個讀不到就中止，不能在不清楚裡面有什麼時刪整個資料夾
        let old_files: Vec<PathBuf> = if dest.is_dir() {
            let mut files = Vec::new();
            for entry in walkdir::WalkDir::new(long_path(&dest)) {
                let entry = entry.map_err(|e| format!("讀不到舊字體包的內容，已停止，沒有動任何檔案：{e}"))?;
                if entry.file_type().is_file() {
                    let rel = entry.path().strip_prefix(long_path(&dest)).unwrap_or(entry.path());
                    files.push(dest.join(rel));
                }
            }
            files
        } else {
            vec![dest.clone()]
        };
        for old in old_files {
            let rel = apply_record::rel_key(&mc, &old);
            let current = apply_record::file_sha256(&old);
            if record.files.contains_key(&rel) && record.is_tool_version(&rel, current.as_deref()) {
                // 前置條件：覆蓋工具版本前，遊戲檔標記要能確認（缺了從紀錄重建；重建不了就不覆蓋）
                if let apply_guard::Check::Broken(why) = apply_guard::require_game_marker(&ctx, record, &rel) {
                    crate::dev_log!("font", "{rel}：{why}");
                    unconfirmed.push(rel);
                }
                continue;
            }
            to_quarantine.push((old, rel));
        }
        if !unconfirmed.is_empty() {
            return Err(unconfirmed_message(&unconfirmed));
        }
        for (old, rel) in to_quarantine {
            let marker_id = record
                .files
                .get(&rel)
                .map(|e| e.marker_id.clone())
                .filter(|id| !id.is_empty())
                .unwrap_or_else(mk::new_marker_id);
            let q = apply_guard::require_quarantined(&ctx, &old, &rel, &marker_id)?;
            quarantined.push((rel, marker_id, q));
        }
    }

    let mut markers = Vec::new();
    for (src, rel) in &new_files {
        let sha = apply_record::file_sha256(src).ok_or_else(|| format!("讀不到字體包檔案：{}", src.display()))?;
        let previous = record.files.get(rel).cloned();
        let quarantine = quarantined.iter().find(|(r, _, _)| r == rel);
        let marker_id = quarantine
            .map(|(_, id, _)| id.clone())
            .or_else(|| previous.as_ref().map(|p| p.marker_id.clone()).filter(|id| !id.is_empty()))
            .unwrap_or_else(mk::new_marker_id);
        // 原本不是工具的檔：來源不明（原本內容在隔離區）；其餘是工具新增
        let (kind, origin) = if quarantine.is_some() {
            (apply_record::FileKind::Overwritten, apply_record::Origin::Unknown)
        } else if let Some(p) = &previous {
            (p.kind, p.origin)
        } else {
            (apply_record::FileKind::Added, apply_record::Origin::Known)
        };
        record.set_entry(rel, kind, origin, None, sha.clone());
        let mut links = vec![mk::link(&ctx.batch_id, mk::REL_BATCH)];
        if let Some((_, _, q)) = quarantine {
            links.push(mk::link(q, mk::REL_QUARANTINE));
        }
        if let Some(entry) = record.files.get_mut(rel) {
            entry.marker_id = marker_id.clone();
            entry.quarantine_id = quarantine.map(|(_, _, q)| q.clone()).unwrap_or_default();
            // 字體包自己一批：「移除翻譯」不動它，由「移除字體包」處理
            entry.owner = apply_record::OWNER_FONT.into();
        }
        markers.push(mk::FileMarker {
            id: marker_id,
            instance_id: ctx.instance_id.clone(),
            rel: rel.clone(),
            role: if kind == apply_record::FileKind::Added { "added".into() } else { "overwritten".into() },
            tool_sha256: sha,
            original_sha256: String::new(),
            backup: String::new(),
            tool_version: env!("CARGO_PKG_VERSION").into(),
            written_at: mk::now_secs(),
            origin: if origin == apply_record::Origin::Unknown { "unknown".into() } else { "known".into() },
            links,
            tool_history: previous.as_ref().map(|p| {
                let mut h = p.history.clone();
                h.push(p.sha256.clone());
                h
            }).unwrap_or_default(),
            state: "pending".into(),
        });
    }

    // 設定檔：先算好、準備清單項目與另存檔的標記
    let font_options = plan_font_options(&ctx, &mc, &name, record, &mut markers)?;

    // 先存紀錄與全部標記，才動遊戲檔
    let extra: Vec<String> = font_options.entry_id.iter().cloned().collect();
    apply_guard::require_record_and_markers(&ctx, record, &markers, font_options.marker.as_ref(), &extra)?;

    fs::create_dir_all(long_path(&resourcepacks)).map_err(|e| format!("無法建立遊戲的資源包資料夾：{e}"))?;
    if long_path(&dest).is_dir() {
        fs::remove_dir_all(long_path(&dest)).map_err(|e| format!("移除舊字體資源包失敗：{e}"))?;
    } else if long_path(&dest).is_file() {
        fs::remove_file(long_path(&dest)).map_err(|e| format!("移除舊字體資源包失敗：{e}"))?;
    }
    if font_pack_path.is_dir() {
        copy_dir_recursive(font_pack_path, &dest)?;
    } else {
        apply_record::copy_atomic(font_pack_path, &dest).map_err(|e| format!("複製字體資源包失敗：{e}"))?;
    }
    for (_, rel) in &new_files {
        mk::mark_written(&mc, rel)?;
    }
    let enabled = write_font_options(&mc, &name, font_options.plan.as_ref())?;

    let backup_path = (!quarantined.is_empty())
        .then(|| apply_record::quarantine_dir(&mc).join(&ctx.batch_id).display().to_string());
    // 識別碼認回的說明（部分檔案被整合包更新改過）放在最前面
    let notice_prefix = knowledge.notice.as_deref().map(|n| format!("{n}\n\n")).unwrap_or_default();
    let backup_line = backup_path
        .as_deref()
        .map(|p| format!("\n• 同名的舊字體包不是工具做的，已先移到隔離區保存：\n{p}"))
        .unwrap_or_default();
    Ok(FontPackApplyResult {
        copied_path: dest.display().to_string(),
        backup_path,
        player_summary: if enabled {
            format!(
                "{notice_prefix}字體資源包已放進遊戲，並已啟用、排在資源包清單最上面（最高優先）。\n• 位置：\n{}{}",
                dest.display(),
                backup_line
            )
        } else {
            format!(
                "{notice_prefix}字體資源包已放進遊戲的資源包資料夾，但這個遊戲資料夾還沒有啟動過遊戲，無法自動啟用。\n\
請先用啟動器開一次遊戲再關掉，然後重新按一次套用字體。\n• 位置：\n{}{}",
                dest.display(),
                backup_line
            )
        },
    })
}

/// 舊字體包裡有工具的檔，但它的標記對不上又補不回：無法確認，整個字體包都不換。
fn unconfirmed_message(rels: &[String]) -> String {
    let shown: Vec<String> = rels.iter().take(10).map(|r| format!("  - {r}")).collect();
    format!(
        "沒有換上新的字體包：遊戲裡舊字體包的以下檔案，工具的標記對不上又補不回，無法確認是不是工具放的。\n\
為了不蓋錯，這次一個檔都沒有動。可以先按「移除字體包」，或在遊戲的資源包資料夾裡確認後再套用一次：\n{}",
        shown.join("\n")
    )
}

struct FontOptions {
    plan: Option<(String, options_txt::OptionsEdit, String)>,
    marker: Option<super::mcpl_marker::OptionsMarker>,
    entry_id: Option<String>,
}

/// 字體包的設定檔修改：先算好、把清單項目標記指向字體包的第一個檔（該檔標記也指回來）、
/// 設定檔旁另存檔也先有標記。沒有 options.txt（還沒啟動過遊戲）時不自己建。
fn plan_font_options(
    ctx: &super::apply_guard::Ctx,
    mc: &Path,
    pack_name: &str,
    record: &mut apply_record::ApplyRecord,
    markers: &mut Vec<super::mcpl_marker::FileMarker>,
) -> Result<FontOptions, String> {
    use super::mcpl_marker as mk;
    let options = mc.join("options.txt");
    if !long_path(&options).is_file() {
        return Ok(FontOptions { plan: None, marker: None, entry_id: None });
    }
    let original =
        fs::read_to_string(long_path(&options)).map_err(|e| format!("讀取遊戲設定檔失敗：{e}"))?;
    let entry = options_txt::pack_entry(pack_name);
    let edit = options_txt::enable_pack_last(&original, Some(&entry), false);
    // 字體包的清單項目記在自己的欄位（舊紀錄記在翻譯欄位的也搬過來）
    let was_in_translation = record.options.packs_added.contains(&entry);
    record.options.packs_added.retain(|e| e != &entry);
    if (edit.pack_added || was_in_translation) && !record.options.font_packs_added.contains(&entry) {
        record.options.font_packs_added.push(entry.clone());
    }
    let mut marker = mk::read_options_marker(mc);
    let mut entry_id = None;
    if record.options.font_packs_added.contains(&entry) {
        if let Some(first) = markers.first_mut() {
            let id = marker
                .entries
                .iter()
                .find(|e| e.entry == entry)
                .map(|e| e.id.clone())
                .unwrap_or_else(mk::new_marker_id);
            marker.entries.retain(|e| e.entry != entry);
            marker.entries.push(mk::EntryMarker {
                id: id.clone(),
                entry: entry.clone(),
                links: vec![mk::link(&first.id, mk::REL_GAME_FILE), mk::link(&ctx.batch_id, mk::REL_BATCH)],
            });
            mk::set_link(&mut first.links, mk::REL_PACK_ENTRY, &id);
            entry_id = Some(id);
        }
    }
    markers.extend(super::pack_repair::options_bak_marker(ctx, record, &original, apply_record::OWNER_FONT));
    Ok(FontOptions { plan: Some((original, edit, entry)), marker: Some(marker), entry_id })
}

/// 紀錄與標記存好之後才寫設定檔：另存一份（只在第一次）、字體包排到最後（最高優先）、
/// 寫完重讀驗證，不對就寫回原檔並回報。沒有設定檔回 `Ok(false)`。
fn write_font_options(
    mc: &Path,
    pack_name: &str,
    plan: Option<&(String, options_txt::OptionsEdit, String)>,
) -> Result<bool, String> {
    let Some((original, edit, entry)) = plan else {
        return Ok(false);
    };
    let options = mc.join("options.txt");
    options_txt::backup_beside_once(&options, original)
        .map_err(|e| format!("另存遊戲設定檔失敗，已停止啟用字體包：{e}"))?;
    apply_record::write_atomic(&options, edit.text.as_bytes())
        .map_err(|e| format!("寫入遊戲設定檔失敗：{e}"))?;
    let after = fs::read_to_string(long_path(&options)).unwrap_or_default();
    if let Err(problem) = options_txt::verify_enabled(original, &after, Some(entry), false) {
        let _ = apply_record::write_atomic(&options, original.as_bytes());
        return Err(format!(
            "啟用字體包時發現問題（{problem}），已把遊戲設定還原成原本的樣子。\
請在遊戲的資源包畫面手動啟用「{pack_name}」並拉到最上面。"
        ));
    }
    Ok(true)
}

/// 測試用：只做「設定檔啟用字體包」這一步（不經紀錄）。
#[cfg(test)]
fn enable_font_pack_at_top(
    mc: &Path,
    pack_name: &str,
    _record: &mut apply_record::ApplyRecord,
) -> Result<bool, String> {
    let options = mc.join("options.txt");
    if !long_path(&options).is_file() {
        return Ok(false);
    }
    let original = fs::read_to_string(long_path(&options)).map_err(|e| e.to_string())?;
    let entry = options_txt::pack_entry(pack_name);
    let edit = options_txt::enable_pack_last(&original, Some(&entry), false);
    write_font_options(mc, pack_name, Some(&(original, edit, entry)))
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// 供前端 FontFace 預覽：回傳 base64（不含 data: 前綴）。
pub fn read_font_preview_base64(font_path: &str) -> Result<String, String> {
    let font = PathBuf::from(font_path.trim().trim_matches('"'));
    cjk_font_file_name(&font)?;
    check_font_file(&font)?;
    let bytes = fs::read(&font).map_err(|e| format!("讀取字體檔失敗：{e}"))?;
    Ok(base64_encode(&bytes))
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in walkdir::WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path.strip_prefix(src).unwrap_or(path);
        let target = dst.join(rel);
        if path.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else if path.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(path, &target)
                .map_err(|e| format!("複製字體資源包檔案失敗 {}：{e}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        apply_font_pack_to_instance, cjk_font_file_name, enable_font_pack_at_top, font_pack_display_name,
        resolve_font_pack_format, FontPackOptions, DEFAULT_FONT_PACK_FORMAT,
        DEFAULT_FONT_PACK_NAME,
    };
    use std::{fs, path::Path};

    #[test]
    fn font_options_are_clamped_to_safe_renderer_ranges() {
        let options = FontPackOptions {
            size: 99.0,
            weight: -10.0,
            shift_x: f32::NAN,
            shift_y: -99.0,
            oversample: 99.0,
        };
        let normalized = options.normalized();

        assert_eq!(normalized.size, 24.0);
        assert_eq!(normalized.weight, 100.0);
        assert_eq!(normalized.shift_x, 0.0);
        assert_eq!(normalized.shift_y, -3.0);
        assert_eq!(normalized.oversample, 8.0);
    }

    #[test]
    fn cjk_font_preserves_ttf_and_otf_extensions() {
        assert_eq!(
            cjk_font_file_name(Path::new(r"C:\fonts\NotoSans.ttf")).unwrap(),
            "cjk_font.ttf"
        );
        assert_eq!(
            cjk_font_file_name(Path::new("/tmp/SourceHan.OTF")).unwrap(),
            "cjk_font.otf"
        );
    }

    #[test]
    fn cjk_font_rejects_ttc_with_clear_message() {
        let err = cjk_font_file_name(Path::new("mingliu.ttc")).unwrap_err();
        assert!(err.contains(".ttc"), "{err}");
        assert!(err.contains("ttf") || err.contains("otf"), "{err}");
    }

    #[test]
    fn empty_pack_name_uses_font_specific_default() {
        assert_eq!(
            font_pack_display_name("").unwrap(),
            DEFAULT_FONT_PACK_NAME
        );
        assert_eq!(
            font_pack_display_name("   ").unwrap(),
            DEFAULT_FONT_PACK_NAME
        );
        assert_eq!(font_pack_display_name("我的字體").unwrap(), "我的字體");
    }

    #[test]
    fn pack_format_prefers_explicit_then_version_then_default() {
        assert_eq!(resolve_font_pack_format(Some(15), Some("1.21.1")), 15);
        assert_eq!(resolve_font_pack_format(None, Some("1.21.1")), 34);
        assert_eq!(resolve_font_pack_format(None, Some("1.20.1")), 15);
        assert_eq!(
            resolve_font_pack_format(None, None),
            DEFAULT_FONT_PACK_FORMAT
        );
    }

    #[test]
    fn font_pack_goes_last_with_backup_and_keeps_other_packs() {
        let root = std::env::temp_dir().join(format!("font_top_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        fs::create_dir_all(mc.join("mods")).unwrap();
        let original = "lang:zh_tw\nresourcePacks:[\"vanilla\",\"file/Gui.zip\",\"file/繁體中文翻譯.zip\"]\n";
        fs::write(mc.join("options.txt"), original).unwrap();
        assert!(enable_font_pack_at_top(&mc, "繁體中文遊戲字體", &mut super::apply_record::ApplyRecord::default()).unwrap());
        let text = fs::read_to_string(mc.join("options.txt")).unwrap();
        let packs = super::super::resource_pack_guard::parse_pack_list(&text);
        assert_eq!(packs.last().map(String::as_str), Some("file/繁體中文遊戲字體"), "{packs:?}");
        assert!(packs.contains(&"file/Gui.zip".to_string()));
        assert_eq!(fs::read_to_string(mc.join("options.txt.mcpl-bak")).unwrap(), original);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn font_pack_does_not_create_options_txt() {
        let root = std::env::temp_dir().join(format!("font_noopt_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        fs::create_dir_all(mc.join("mods")).unwrap();
        assert!(!enable_font_pack_at_top(&mc, "字體", &mut super::apply_record::ApplyRecord::default()).unwrap());
        assert!(!mc.join("options.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_font_pack_backs_up_same_name_pack() {
        let root = std::env::temp_dir().join(format!("font_apply_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let generated = root.join("翻譯結果").join("resourcepacks").join("繁體中文遊戲字體");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(mc.join("resourcepacks/繁體中文遊戲字體")).unwrap();
        fs::create_dir_all(generated.join("assets/minecraft/font")).unwrap();
        fs::write(mc.join("resourcepacks/繁體中文遊戲字體/old.txt"), "old").unwrap();
        fs::write(generated.join("assets/minecraft/font/default.json"), "{}").unwrap();

        let result = apply_font_pack_to_instance(&mc, &generated).unwrap();

        assert!(result.backup_path.is_some());
        assert!(mc
            .join("resourcepacks/繁體中文遊戲字體/assets/minecraft/font/default.json")
            .is_file());
        assert!(Path::new(result.backup_path.as_deref().unwrap()).is_file()
            || Path::new(result.backup_path.as_deref().unwrap()).is_dir());
        let _ = fs::remove_dir_all(root);
    }
}
