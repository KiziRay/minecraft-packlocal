//! 遊戲內可讀文字覆寫（非 jar lang）：patchouli / openloader / kubejs / datapacks / fancymenu。
//!
//! **搜尋原則（對齊手翻、泛化到任意模組包）**：
//! - 多根目錄：不同模組常把文字塞在不同資料夾（openloader／global_packs／datapacks…）。
//! - 路徑提示 + 內容嗅探：不假設單一固定樹；路徑像顯示內容、或 JSON 含 `loc_name` 等鍵才收。
//! - 資料包 gameplay JSON 只翻「顯示欄位白名單」（`loc_*`／`flavor_text`／`effect_tip`…），
//!   不掃全部字串，避免把 particle type、guid、id 翻壞。
//!
//! 只輸出到 work 目錄，不直接改實例。

use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::convert::convert_s2tw_batch;
use super::deepseek::translate_plain_strings_with_scope;
use super::translation_scope::TranslationScope;

const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// 單次送給翻譯引擎的上限；全部候選文字會分批處理。
const AI_BATCH_SIZE: usize = 8000;
const MAX_WALK_DEPTH: usize = 24;
/// 內容嗅探最多讀前 N bytes（判斷是否含顯示欄位鍵）
const SNIFF_BYTES: usize = 256 * 1024;

/// 資料包／模組自訂 JSON 的顯示欄位（Mine and Slash 系 `loc_*`、通用 desc/title…）。
/// 裸 `name` 故意不收：常是 guid／機制 id，翻了會壞。
const DISPLAY_FIELD_KEYS: &[&str] = &[
    "loc_name",
    "loc_desc",
    "loc_description",
    "loc_desc1",
    "loc_desc2",
    "loc_desc3",
    "loc_title",
    "loc_tooltip",
    "loc_text",
    "effect_tip",
    "flavor_text",
    "flavour_text",
    "description",
    "desc",
    "title",
    "subtitle",
    "tooltip",
    "lore",
    "text",
    "message",
    "header",
    "tip",
    "hover",
    "label",
    "caption",
    "wiki",
];

/// 只在已判定為玩家顯示內容的資料夾啟用，避免把一般 JSON 的 name/id 當成文字。
const AGGRESSIVE_DISPLAY_FIELD_KEYS: &[&str] = &[
    "display_name",
    "displayname",
    "display",
    "body",
    "content",
    "page",
    "pages",
    "category",
    "help",
];

/// `strings_translated` 目前只進 `note`，保留欄位供排查回報用。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct OverlayTranslateResult {
    pub files_written: usize,
    pub strings_translated: usize,
    pub note: String,
}

/// 掃描並翻譯 minecraft_dir 內文字覆寫，寫入 output_dir（保留相對路徑）。
pub fn translate_text_overlays<F>(
    minecraft_dir: &Path,
    output_dir: &Path,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    mut on_progress: F,
) -> Result<OverlayTranslateResult, String>
where
    F: FnMut(u8, &str),
{
    if !minecraft_dir.is_dir() {
        return Err(format!(
            "找不到 minecraft 目錄：{}",
            minecraft_dir.display()
        ));
    }

    on_progress(
        3,
        "覆寫文字：掃描多根目錄（openloader／datapacks／global_packs／kubejs…）…",
    );
    let files = collect_overlay_files(minecraft_dir);
    if files.is_empty() {
        return Ok(OverlayTranslateResult {
            files_written: 0,
            strings_translated: 0,
            note:
                "未找到可處理的文字覆寫檔（多根：openloader／datapacks／global_packs／kubejs／fancymenu 等）。"
                    .into(),
        });
    }

    on_progress(
        12,
        &format!("覆寫文字：候補 {} 個檔，擷取字串…", files.len()),
    );

    // path → 原始內容 + 類型
    let mut file_payloads: Vec<FilePayload> = Vec::new();
    let mut unique: Vec<String> = Vec::new();
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut parse_failures: Vec<String> = Vec::new();

    for path in &files {
        match load_payload(path) {
            Ok(payload) => {
                for s in payload.collect_strings() {
                    if should_translate_overlay_string(&s) && seen.insert(s.clone(), ()).is_none() {
                        unique.push(s);
                    }
                }
                file_payloads.push(payload);
            }
            // 解析失敗不再靜默：記下來，讓使用者分得清是檔案壞了還是工具漏翻。
            Err(e) => parse_failures.push(e),
        }
    }
    if !parse_failures.is_empty() {
        on_progress(
            21,
            &format!(
                "覆寫文字：{} 個檔解析失敗已略過（詳見錯誤日誌）",
                parse_failures.len()
            ),
        );
        write_overlay_parse_errors(output_dir, &parse_failures);
    }

    on_progress(
        22,
        &format!(
            "覆寫文字：{} 檔可讀、唯一可譯字串 {}",
            file_payloads.len(),
            unique.len()
        ),
    );

    if unique.is_empty() {
        return Ok(OverlayTranslateResult {
            files_written: 0,
            strings_translated: 0,
            note: format!(
                "掃描 {} 檔，但沒有符合條件的可譯字串（可能已是繁中或僅 id／路徑）。",
                files.len()
            ),
        });
    }

    // original → translated
    let mut map: HashMap<String, String> = HashMap::new();

    // 1) OpenCC：看起來有中文的一律 s2twp
    on_progress(28, "覆寫文字：簡體→台灣繁體（OpenCC）…");
    let chinese_idx: Vec<usize> = unique
        .iter()
        .enumerate()
        .filter(|(_, s)| looks_chinese(s))
        .map(|(i, _)| i)
        .collect();
    if !chinese_idx.is_empty() {
        let batch: Vec<String> = chinese_idx.iter().map(|&i| unique[i].clone()).collect();
        let conv = convert_s2tw_batch(&batch);
        for (j, &i) in chinese_idx.iter().enumerate() {
            let orig = &unique[i];
            if let Some(c) = conv.get(j) {
                if c != orig && !c.trim().is_empty() {
                    map.insert(orig.clone(), c.clone());
                }
            }
        }
    }

    // 2) AI：剩餘非中文（或未進 map 的拉丁文）
    if use_ai {
        let mut need_ai: Vec<String> = Vec::new();
        for s in &unique {
            if map.contains_key(s) {
                continue;
            }
            // 已是純中文且轉換無差 → 略過 AI
            if looks_chinese(s) && !has_latin_letter(s) {
                continue;
            }
            need_ai.push(s.clone());
        }

        if !need_ai.is_empty() {
            on_progress(
                40,
                &format!(
                    "覆寫文字：AI 分批翻譯 {} 條，完整處理中…",
                    need_ai.len()
                ),
            );
            let total_batches = need_ai.len().div_ceil(AI_BATCH_SIZE);
            for (batch_index, batch) in need_ai.chunks(AI_BATCH_SIZE).enumerate() {
                super::cancel::check()?;
                let batch_start = batch_index * AI_BATCH_SIZE;
                let translated = translate_plain_strings_with_scope(batch, scope, |pct, msg| {
                    let completed = batch_start + batch.len() * pct as usize / 100;
                    let mapped = 40 + ((completed * 38) / need_ai.len().max(1)) as u8;
                    on_progress(mapped.min(78), msg);
                })?;
                for (i, en) in batch.iter().enumerate() {
                    if let Some(zh) = translated.get(i) {
                        let t = zh.trim();
                        if !t.is_empty() && t != en {
                            map.insert(en.clone(), t.to_string());
                        }
                    }
                }
                on_progress(
                    40 + (((batch_index + 1) * 38) / total_batches.max(1)) as u8,
                    &format!(
                        "覆寫文字：已完成第 {}/{} 批（{} 條）",
                        batch_index + 1,
                        total_batches,
                        need_ai.len()
                    ),
                );
            }
            // AI 結果再跑一次 s2twp
            if !map.is_empty() {
                on_progress(82, "覆寫文字：AI 結果再轉台灣繁…");
                let keys: Vec<String> = map.keys().cloned().collect();
                let vals: Vec<String> = keys.iter().map(|k| map[k].clone()).collect();
                let conv = convert_s2tw_batch(&vals);
                for (i, k) in keys.iter().enumerate() {
                    if let Some(v) = conv.get(i) {
                        map.insert(k.clone(), v.clone());
                    }
                }
            }
        } else {
            on_progress(70, "覆寫文字：無需 AI（皆已中文／OpenCC 處理）");
        }
    } else {
        on_progress(50, "覆寫文字：未勾 AI，僅 OpenCC 處理既有中文");
    }

    if map.is_empty() {
        return Ok(OverlayTranslateResult {
            files_written: 0,
            strings_translated: 0,
            note: if use_ai {
                format!(
                    "掃描 {} 檔、唯一字串 {}；OpenCC／AI 皆無變更（可能已是繁中）。",
                    file_payloads.len(),
                    unique.len()
                )
            } else {
                format!(
                    "掃描 {} 檔、唯一字串 {}；本機轉換沒有新增內容（可能已是繁中）。",
                    file_payloads.len(),
                    unique.len()
                )
            },
        });
    }

    // 3) 寫出
    on_progress(88, "覆寫文字：寫出檔案…");
    let mut written = 0usize;
    for payload in &file_payloads {
        if let Some(new_bytes) = payload.apply(&map)? {
            let rel = payload
                .path
                .strip_prefix(minecraft_dir)
                .unwrap_or(payload.path.as_path());
            let out_path = output_dir.join(rel);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&out_path, new_bytes).map_err(|e| format!("{}: {e}", out_path.display()))?;
            written += 1;
        }
    }

    let note = format!(
        "覆寫文字：掃描 {} 檔、唯一字串 {}、翻譯表 {} 條、寫出 {} 檔（patchouli／openloader／kubejs／datapacks／fancymenu／顯示型 config）。流程完成時會直接套用到遊戲{}",
        file_payloads.len(),
        unique.len(),
        map.len(),
        written,
        "。"
    );
    on_progress(100, "覆寫文字完成");
    Ok(OverlayTranslateResult {
        files_written: written,
        strings_translated: map.len(),
        note,
    })
}

/// 把覆寫檔的解析失敗追加到結果目錄的錯誤日誌，讓使用者查得到（不再靜默）。
fn write_overlay_parse_errors(output_dir: &Path, failures: &[String]) {
    if failures.is_empty() {
        return;
    }
    let p = output_dir.join("翻譯錯誤日誌.txt");
    let mut body = String::new();
    if let Ok(old) = fs::read_to_string(&p) {
        body.push_str(&old);
    } else {
        body.push_str("【模組包繁中翻譯 — 錯誤／警告日誌】\n有問題時請把本檔內容一併提供。\n");
    }
    body.push_str("\n======== 覆寫文字解析失敗 ========\n");
    for f in failures {
        body.push_str("• ");
        body.push_str(f);
        body.push('\n');
    }
    let _ = fs::write(p, body);
}

// ─── 檔案收集 ───────────────────────────────────────────────

fn collect_overlay_files(mc: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.iter().any(|x| x == &p) {
            out.push(p);
        }
    };

    // patchouli_books/**/*.json
    let pb = mc.join("patchouli_books");
    if pb.is_dir() {
        for p in walk_files(&pb, &["json"]) {
            push(p);
        }
    }

    // 資料包根：手翻會依模組去不同資料夾找；工具同樣掃多根
    // - config/openloader：最常見巢狀 data/resources
    // - datapacks / global_packs / paxi / defaultconfigs / 頂層 data
    let data_pack_roots = [
        mc.join("config").join("openloader"),
        mc.join("datapacks"),
        mc.join("global_packs"),
        mc.join("paxi").join("datapacks"),
        mc.join("resourcepacks"),
        mc.join("defaultconfigs"),
        mc.join("data"),
        mc.join("assets"),
        mc.join("kubejs").join("data"),
    ];
    for root in &data_pack_roots {
        if !root.is_dir() {
            continue;
        }
        for p in walk_files(root, &["json", "json5", "properties"]) {
            if is_pack_text_json(&p) {
                push(p);
            }
        }
    }

    // 部分整合包把玩家看得到的文字放在 config，而不是 datapack。
    // 只依顯示路徑／欄位白名單收取，避免把整個 config 當成語言檔。
    let config_root = mc.join("config");
    if config_root.is_dir() {
        for p in walk_files(&config_root, &["json", "json5", "txt", "properties"]) {
            let is_json = p
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("json") || s.eq_ignore_ascii_case("json5"))
                .unwrap_or(false);
            let is_known_text_root = is_config_text_path(&p);
            let is_properties = p
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("properties"));
            if (is_json && is_pack_text_json(&p))
                || ((is_known_text_root || is_properties) && looks_text_file(&p))
            {
                push(p);
            }
        }
    }

    // kubejs/**/lang/*.json（腳本旁語言檔；data 已在上列）
    let kjs = mc.join("kubejs");
    if kjs.is_dir() {
        for p in walk_files(&kjs, &["json"]) {
            if path_has_segment(&p, "lang") {
                push(p);
            }
        }
    }

    // config/fancymenu + defaultconfigs/fancymenu
    for fm in [
        mc.join("config").join("fancymenu"),
        mc.join("defaultconfigs").join("fancymenu"),
    ] {
        if fm.is_dir() {
            for p in walk_files(&fm, &["txt", "json", "properties"]) {
                if looks_text_file(&p) {
                    push(p);
                }
            }
        }
    }

    // GuideME／自訂手冊常用 Markdown 或純文字，不假設固定模組名稱。
    for root in [
        mc.join("guideme"),
        mc.join("config").join("guideme"),
        mc.join("guidebook"),
    ] {
        if root.is_dir() {
            for p in walk_files(&root, &["json", "md", "txt", "properties"]) {
                if looks_text_file(&p) {
                    push(p);
                }
            }
        }
    }

    out
}

fn walk_files(root: &Path, exts: &[&str]) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for e in WalkDir::new(root)
        .max_depth(MAX_WALK_DEPTH)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        if is_skipped_binary_ext(p) {
            continue;
        }
        let ext_ok = p
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| {
                let sl = s.to_ascii_lowercase();
                exts.iter().any(|e| *e == sl)
            })
            .unwrap_or(false);
        if !ext_ok {
            continue;
        }
        if let Ok(meta) = p.metadata() {
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
        }
        v.push(p.to_path_buf());
    }
    v
}

/// 是否為資料包內「可讀文字」JSON（手翻會開的那類，不限單一模組資料夾名）。
fn is_pack_text_json(path: &Path) -> bool {
    // 1) 標準文字路徑
    if path_has_segment(path, "lang")
        || path_has_segment(path, "advancements")
        || path_has_segment(path, "patchouli_books")
        || path_has_segment(path, "patchouli")
    {
        return true;
    }
    // 純機制路徑（配方／loot／tags…）跳過，省 I/O
    if is_mechanism_only_path(path) {
        return false;
    }
    // 2) 路徑片段提示：跨模組常見顯示內容資料夾名（不綁死單一包）
    if is_display_content_path(path) {
        return true;
    }
    // 3) 內容嗅探：JSON 含顯示欄位鍵才收（技能／詞綴等自訂 schema）
    file_sniffs_display_field(path)
}

/// 幾乎不會有玩家可讀字串的資料包子路徑（跨模組通用）。
fn is_mechanism_only_path(path: &Path) -> bool {
    let lower = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    const SKIP: &[&str] = &[
        "/recipes/",
        "/loot_tables/",
        "/tags/",
        "/structures/",
        "/worldgen/",
        "/functions/",
        "/predicates/",
        "/item_modifiers/",
        "/dimension/",
        "/dimension_type/",
        "/biome/",
        "/noise_settings/",
        "/template_pool/",
        "/processor_list/",
        "/configured_feature/",
        "/placed_feature/",
        "/chat_type/",
        "/damage_type/",
        // Mine and Slash 純數值／條件表（顯示名在別處）
        "/mmorpg_value_calc/",
        "/mmorpg_stat_condition/",
        "/mmorpg_stat_effect/",
        "/mmorpg_stat_compat/",
        "/mmorpg_auto_item/",
        "/mmorpg_base_stats/",
        "/mmorpg_game_balance/",
        "/mmorpg_atlas_layout/",
    ];
    SKIP.iter().any(|s| lower.contains(s))
}

/// 路徑像「給玩家看的內容」——不同模組資料夾名不同，用通用片段比對。
fn is_display_content_path(path: &Path) -> bool {
    let lower = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    const HINTS: &[&str] = &[
        "/spells/",
        "/spell/",
        "/spell_",
        "/affix",
        "/unique",
        "/perk",
        "/talent",
        "/aura",
        "/omen",
        "/relic",
        "/profession",
        "/support_gem",
        "/exile_effect",
        "/quest",
        "/dialog",
        "/dialogue",
        "/books/",
        "/book/",
        "/lore/",
        "/tips/",
        "/wiki/",
        "/powers/",
        "/origins/",
        "/origin_layers/",
        // Mine and Slash / Library of Exile 系（CTE 等）常見前綴；其他包若同名也涵蓋
        "/mmorpg_",
        "/library_of_exile",
        "/armorsets",
        "/starterkit",
        "/minecolonies",
        "/custom_gui",
        "/gui/",
        "/menus/",
        "/handbook",
        "/manual",
        "/codex",
        "/guide/",
        "/guides/",
        "/skills/",
        "/abilities/",
        "/items/",
        "/affixes/",
        "/professions/",
        "/shop/",
        "/challenges/",
        "/milestones/",
        "/missions/",
        "/chapters/",
        "/entries/",
        "/categories/",
    ];
    HINTS.iter().any(|h| lower.contains(h))
}

/// CTE2 類整合包常見的設定型文字來源；只允許明確的顯示／說明資料夾。
fn is_config_text_path(path: &Path) -> bool {
    [
        "fancymenu",
        "starterkit",
        "armorsets",
        "minecolonies",
        "profession_shop",
        "custom_gui",
        "guidebook",
        "quests",
        "questing",
        "handbook",
        "manual",
        "codex",
        "guide",
        "guides",
        "skills",
        "abilities",
        "professions",
        "shop",
        "challenges",
        "milestones",
        "missions",
        "chapters",
        "entries",
        "categories",
    ]
    .iter()
    .any(|segment| path_has_segment(path, segment))
}

/// 讀檔前段，是否出現顯示欄位鍵（避免把純數值表整包掃進來）。
fn file_sniffs_display_field(path: &Path) -> bool {
    let Ok(mut f) = fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    let mut buf = vec![0u8; SNIFF_BYTES];
    let n = f.read(&mut buf).unwrap_or(0);
    if n == 0 {
        return false;
    }
    let Ok(s) = std::str::from_utf8(&buf[..n]) else {
        return false;
    };
    // 鍵名用引號包住比對，減少誤中
    s.contains("\"loc_name\"")
        || s.contains("\"loc_desc\"")
        || s.contains("\"loc_description\"")
        || s.contains("\"loc_title\"")
        || s.contains("\"loc_tooltip\"")
        || s.contains("\"loc_text\"")
        || s.contains("\"effect_tip\"")
        || s.contains("\"flavor_text\"")
        || s.contains("\"flavour_text\"")
}

fn path_has_segment(path: &Path, seg: &str) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| s.eq_ignore_ascii_case(seg))
            .unwrap_or(false)
    })
}

fn is_skipped_binary_ext(path: &Path) -> bool {
    const SKIP: &[&str] = &[
        "png", "jpg", "jpeg", "webp", "gif", "ogg", "wav", "mp3", "ttf", "otf", "zip", "jar",
        "class", "nbt", "bin",
    ];
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| SKIP.contains(&s.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn looks_text_file(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    if bytes.is_empty() {
        return false;
    }
    // 含 NUL 當二進位
    if bytes.iter().take(4096).any(|&b| b == 0) {
        return false;
    }
    true
}

// ─── 載入／套用 ─────────────────────────────────────────────

enum FileKind {
    Json,
    /// FancyMenu 等：引號字串替換
    QuotedText,
    Markdown,
    Properties,
}

/// 字串擷取模式：lang/書本可掃全部；gameplay 資料包只碰顯示欄位。
#[derive(Clone, Copy)]
enum StringMode {
    All,
    DisplayFieldsOnly,
    AggressiveDisplayFields,
}

struct FilePayload {
    path: PathBuf,
    kind: FileKind,
    mode: StringMode,
    raw: String,
}

impl FilePayload {
    fn collect_strings(&self) -> Vec<String> {
        match self.kind {
            FileKind::Json => {
                let Ok(v) = super::lenient_json::parse(&self.raw) else {
                    return vec![];
                };
                let mut out = Vec::new();
                match self.mode {
                    StringMode::All => collect_json_strings(&v, &mut out),
                    StringMode::DisplayFieldsOnly => {
                        collect_display_field_strings(&v, &mut out, false)
                    }
                    StringMode::AggressiveDisplayFields => {
                        collect_display_field_strings(&v, &mut out, true)
                    }
                }
                out
            }
            FileKind::QuotedText => {
                let Ok(re) = Regex::new(r#""((?:\\.|[^"\\])*)""#) else {
                    return vec![];
                };
                re.captures_iter(&self.raw)
                    .filter_map(|c| c.get(1).map(|m| unescape_json_str(m.as_str())))
                    .collect()
            }
            FileKind::Markdown => self
                .raw
                .lines()
                .filter(|line| !line.trim().is_empty() && line.trim() != "```")
                .map(str::to_string)
                .collect(),
            FileKind::Properties => self
                .raw
                .lines()
                .filter_map(property_value)
                .map(str::to_string)
                .collect(),
        }
    }

    fn apply(&self, map: &HashMap<String, String>) -> Result<Option<Vec<u8>>, String> {
        match self.kind {
            FileKind::Json => {
                let mut v: Value = super::lenient_json::parse(&self.raw)
                    .map_err(|e| format!("{}: {e}", self.path.display()))?;
                let changed = match self.mode {
                    StringMode::All => apply_json_strings(&mut v, map),
                    StringMode::DisplayFieldsOnly => {
                        apply_display_field_strings(&mut v, map, false)
                    }
                    StringMode::AggressiveDisplayFields => {
                        apply_display_field_strings(&mut v, map, true)
                    }
                };
                if !changed {
                    return Ok(None);
                }
                let s = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
                Ok(Some(s.into_bytes()))
            }
            FileKind::QuotedText => {
                let mut new_text = self.raw.clone();
                let mut pairs: Vec<_> = map.iter().collect();
                pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
                let mut any = false;
                for (en, zh) in pairs {
                    let from = format!("\"{}\"", escape_json_str(en));
                    let to = format!("\"{}\"", escape_json_str(zh));
                    if new_text.contains(&from) {
                        new_text = new_text.replace(&from, &to);
                        any = true;
                    }
                }
                if any && new_text != self.raw {
                    Ok(Some(new_text.into_bytes()))
                } else {
                    Ok(None)
                }
            }
            FileKind::Markdown => {
                let mut changed = false;
                let mut output = String::with_capacity(self.raw.len());
                for line in self.raw.split_inclusive('\n') {
                    let has_newline = line.ends_with('\n');
                    let content = line.strip_suffix('\n').unwrap_or(line);
                    let replacement = map.get(content).map(String::as_str).unwrap_or(content);
                    if replacement != content {
                        changed = true;
                    }
                    output.push_str(replacement);
                    if has_newline {
                        output.push('\n');
                    }
                }
                if changed {
                    Ok(Some(output.into_bytes()))
                } else {
                    Ok(None)
                }
            }
            FileKind::Properties => {
                let mut changed = false;
                let mut output = String::with_capacity(self.raw.len());
                for line in self.raw.split_inclusive('\n') {
                    let has_newline = line.ends_with('\n');
                    let content = line.strip_suffix('\n').unwrap_or(line);
                    let replacement = property_replaced_line(content, map);
                    if replacement != content {
                        changed = true;
                    }
                    output.push_str(&replacement);
                    if has_newline {
                        output.push('\n');
                    }
                }
                if changed {
                    Ok(Some(output.into_bytes()))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

fn string_mode_for_path(path: &Path) -> StringMode {
    // lang／進度／書本：玩家文案多，沿用全字串
    if path_has_segment(path, "lang")
        || path_has_segment(path, "advancements")
        || path_has_segment(path, "patchouli_books")
        || path_has_segment(path, "patchouli")
    {
        return StringMode::All;
    }
    // 已知玩家顯示內容路徑採積極欄位模式，會多翻 name/display/body/page 等欄位。
    if is_display_content_path(path) || is_config_text_path(path) {
        return StringMode::AggressiveDisplayFields;
    }
    // 其餘資料包 JSON：只翻保守顯示欄位。
    StringMode::DisplayFieldsOnly
}

fn is_display_field_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    if DISPLAY_FIELD_KEYS.contains(&k.as_str()) {
        return true;
    }
    // 泛化：任何 loc_* 當顯示文字（Mine and Slash 系等）
    k.starts_with("loc_")
}

fn is_display_field_key_for_mode(key: &str, aggressive: bool) -> bool {
    let lower = key.to_ascii_lowercase();
    is_display_field_key(key)
        || (aggressive && AGGRESSIVE_DISPLAY_FIELD_KEYS.contains(&lower.as_str()))
}

fn load_payload(path: &Path) -> Result<FilePayload, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "json" || ext == "json5" {
        // 寬鬆驗證（容忍註解／尾逗號）。真的壞掉回描述性錯誤，由呼叫端記錄，不靜默。
        super::lenient_json::parse(&raw)
            .map_err(|e| format!("{} 解析失敗：{e}", path.display()))?;
        return Ok(FilePayload {
            path: path.to_path_buf(),
            kind: FileKind::Json,
            mode: string_mode_for_path(path),
            raw,
        });
    }

    if ext == "md" {
        return Ok(FilePayload {
            path: path.to_path_buf(),
            kind: FileKind::Markdown,
            mode: StringMode::All,
            raw,
        });
    }

    if ext == "properties" {
        return Ok(FilePayload {
            path: path.to_path_buf(),
            kind: FileKind::Properties,
            mode: StringMode::All,
            raw,
        });
    }

    // .txt：若是 JSON 當 JSON，否則引號字串
    if super::lenient_json::parse(&raw).is_ok() {
        return Ok(FilePayload {
            path: path.to_path_buf(),
            kind: FileKind::Json,
            mode: string_mode_for_path(path),
            raw,
        });
    }
    Ok(FilePayload {
        path: path.to_path_buf(),
        kind: FileKind::QuotedText,
        mode: StringMode::All,
        raw,
    })
}

fn collect_json_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => {
            for x in a {
                collect_json_strings(x, out);
            }
        }
        Value::Object(m) => {
            for (_k, x) in m {
                collect_json_strings(x, out);
            }
        }
        _ => {}
    }
}

fn apply_json_strings(v: &mut Value, map: &HashMap<String, String>) -> bool {
    match v {
        Value::String(s) => {
            if let Some(t) = map.get(s) {
                if t != s {
                    *s = t.clone();
                    return true;
                }
            }
            false
        }
        Value::Array(a) => {
            let mut any = false;
            for x in a {
                if apply_json_strings(x, map) {
                    any = true;
                }
            }
            any
        }
        Value::Object(m) => {
            let mut any = false;
            for (_k, x) in m.iter_mut() {
                if apply_json_strings(x, map) {
                    any = true;
                }
            }
            any
        }
        _ => false,
    }
}

/// 只擷取顯示欄位下的字串（與 apply 對稱）。
fn collect_display_field_strings(v: &Value, out: &mut Vec<String>, aggressive: bool) {
    match v {
        Value::Object(m) => {
            for (k, child) in m {
                if is_display_field_key_for_mode(k, aggressive) {
                    push_display_value(child, out, aggressive);
                } else {
                    collect_display_field_strings(child, out, aggressive);
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                collect_display_field_strings(x, out, aggressive);
            }
        }
        _ => {}
    }
}

fn push_display_value(v: &Value, out: &mut Vec<String>, aggressive: bool) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => {
            for x in a {
                push_display_value(x, out, aggressive);
            }
        }
        // 文字元件：只拿 text / extra
        Value::Object(m) => {
            for (k, child) in m {
                let kl = k.to_ascii_lowercase();
                if kl == "text" || kl == "extra" || is_display_field_key_for_mode(k, aggressive) {
                    push_display_value(child, out, aggressive);
                }
            }
        }
        _ => {}
    }
}

fn apply_display_field_strings(
    v: &mut Value,
    map: &HashMap<String, String>,
    aggressive: bool,
) -> bool {
    match v {
        Value::Object(m) => {
            let mut any = false;
            // 先收集鍵，避免邊改邊借
            let keys: Vec<String> = m.keys().cloned().collect();
            for k in keys {
                let is_disp = is_display_field_key_for_mode(&k, aggressive);
                if let Some(child) = m.get_mut(&k) {
                    if is_disp {
                        if apply_display_value(child, map, aggressive) {
                            any = true;
                        }
                    } else if apply_display_field_strings(child, map, aggressive) {
                        any = true;
                    }
                }
            }
            any
        }
        Value::Array(a) => {
            let mut any = false;
            for x in a {
                if apply_display_field_strings(x, map, aggressive) {
                    any = true;
                }
            }
            any
        }
        _ => false,
    }
}

fn apply_display_value(v: &mut Value, map: &HashMap<String, String>, aggressive: bool) -> bool {
    match v {
        Value::String(s) => {
            if let Some(t) = map.get(s) {
                if t != s {
                    *s = t.clone();
                    return true;
                }
            }
            false
        }
        Value::Array(a) => {
            let mut any = false;
            for x in a {
                if apply_display_value(x, map, aggressive) {
                    any = true;
                }
            }
            any
        }
        Value::Object(m) => {
            let mut any = false;
            let keys: Vec<String> = m.keys().cloned().collect();
            for k in keys {
                let kl = k.to_ascii_lowercase();
                let touch = kl == "text"
                    || kl == "extra"
                    || is_display_field_key_for_mode(&k, aggressive);
                if touch {
                    if let Some(child) = m.get_mut(&k) {
                        if apply_display_value(child, map, aggressive) {
                            any = true;
                        }
                    }
                }
            }
            any
        }
        _ => false,
    }
}

// ─── 字串過濾 ───────────────────────────────────────────────

fn should_translate_overlay_string(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 2 {
        return false;
    }
    // 資源路徑／圖片／音效
    let lower = t.to_ascii_lowercase();
    for suf in [
        ".png", ".jpg", ".jpeg", ".webp", ".gif", ".ogg", ".wav", ".mp3", ".json", ".mcmeta",
        ".ttf", ".otf", ".nbt", ".zip",
    ] {
        if lower.ends_with(suf) {
            return false;
        }
    }
    if t.starts_with("http://") || t.starts_with("https://") {
        return false;
    }
    // 純色碼 / 格式碼
    if is_color_or_format_only(t) {
        return false;
    }
    // 純 item id namespace:path
    if t.contains(':') && !t.contains(' ') && t.is_ascii() && !t.contains('\n') {
        // 允許 "Hello: world" 類；純 id 略過
        if t.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '/' || c == '.' || c == '-'
        }) {
            return false;
        }
    }
    // 長 hex id
    if t.chars().all(|c| c.is_ascii_hexdigit()) && t.len() >= 8 {
        return false;
    }
    // 看起來像程式碼／JSON 片段
    if looks_mostly_code(t) {
        return false;
    }
    // 要有字母或中文
    let has_alpha = t.chars().any(|c| c.is_ascii_alphabetic());
    let has_cjk = looks_chinese(t);
    has_alpha || has_cjk
}

fn is_color_or_format_only(s: &str) -> bool {
    // 僅 §x / &x / #RRGGBB / 空白
    let stripped: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if stripped.is_empty() {
        return true;
    }
    // 全是 § 後接一字 或 & 色碼
    let mut chars = stripped.chars().peekable();
    let mut only_codes = true;
    while let Some(c) = chars.next() {
        if c == '§' || c == '&' {
            let _ = chars.next();
            continue;
        }
        if c == '#' {
            for _ in 0..6 {
                match chars.next() {
                    Some(x) if x.is_ascii_hexdigit() => {}
                    _ => {
                        only_codes = false;
                        break;
                    }
                }
            }
            continue;
        }
        only_codes = false;
        break;
    }
    only_codes
}

fn looks_chinese(s: &str) -> bool {
    s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

fn has_latin_letter(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_alphabetic())
}

fn looks_mostly_code(s: &str) -> bool {
    if s.starts_with('{') && s.contains('}') {
        return true;
    }
    if s.starts_with('[') && s.ends_with(']') && s.contains('{') {
        return true;
    }
    if s.contains("\"id\":") || s.contains("Count:") {
        return true;
    }
    // 座標／純數字列表
    if s.chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == ',' || c == '-' || c == ' ')
    {
        return true;
    }
    false
}

fn unescape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('u') => {
                    let mut hex = String::new();
                    for _ in 0..4 {
                        if let Some(h) = chars.next() {
                            hex.push(h);
                        }
                    }
                    if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            out.push(ch);
                        }
                    }
                }
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn escape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

fn property_value(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
        return None;
    }
    let separator = trimmed.find('=').or_else(|| trimmed.find(':'))?;
    let value = trimmed.get(separator + 1..)?.trim();
    (!value.is_empty()).then_some(value)
}

fn property_replaced_line(line: &str, map: &HashMap<String, String>) -> String {
    let Some(separator) = line.find('=').or_else(|| line.find(':')) else {
        return line.to_string();
    };
    let after = &line[separator + 1..];
    let value = after.trim();
    let Some(replacement) = map.get(value) else {
        return line.to_string();
    };
    let leading_len = after.len() - after.trim_start().len();
    let trailing_len = after.len() - after.trim_end().len();
    let leading = &after[..leading_len];
    let trailing = if trailing_len == 0 {
        ""
    } else {
        &after[after.len() - trailing_len..]
    };
    format!("{}{leading}{replacement}{trailing}", &line[..=separator])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_cte_style_config_text_roots() {
        assert!(is_config_text_path(Path::new(
            "minecraft/config/starterkit/descriptions/start.txt"
        )));
        assert!(is_config_text_path(Path::new(
            "minecraft/config/armorsets/oath_of_mahj.json"
        )));
        assert!(is_config_text_path(Path::new(
            "minecraft/config/minecolonies/client.json"
        )));
        assert!(!is_config_text_path(Path::new(
            "minecraft/config/jei/jei-world-style.json"
        )));
    }

    #[test]
    fn recognizes_display_content_path_without_translating_mechanics() {
        assert!(is_display_content_path(Path::new(
            "minecraft/config/armorsets/oath_of_mahj.json"
        )));
        assert!(is_display_content_path(Path::new(
            "minecraft/data/example/minecolonies/guide.json"
        )));
        assert!(!is_display_content_path(Path::new(
            "minecraft/data/example/recipes/iron.json"
        )));
    }

    #[test]
    fn properties_replace_only_values_and_keep_formatting() {
        let mut map = HashMap::new();
        map.insert("Welcome to the pack".to_string(), "歡迎來到整合包".to_string());
        assert_eq!(
            property_replaced_line("welcome =  Welcome to the pack  ", &map),
            "welcome =  歡迎來到整合包  "
        );
        assert_eq!(property_value("# welcome = Hello"), None);
    }

    #[test]
    fn display_paths_enable_aggressive_fields_but_generic_paths_do_not() {
        assert!(matches!(
            string_mode_for_path(Path::new("data/example/quests/chapter.json")),
            StringMode::AggressiveDisplayFields
        ));
        assert!(matches!(
            string_mode_for_path(Path::new("data/example/recipes/iron.json")),
            StringMode::DisplayFieldsOnly
        ));
    }
}
