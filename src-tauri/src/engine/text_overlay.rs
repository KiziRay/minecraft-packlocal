//! 遊戲內可讀文字覆寫（非 jar lang）：patchouli / openloader / kubejs / datapacks / fancymenu。
//!
//! **搜尋原則（對齊手翻、泛化到任意模組包）**：
//! - 多根目錄：不同模組常把文字塞在不同資料夾（openloader／global_packs／datapacks…）。
//! - 路徑提示 + 內容嗅探：不假設單一固定樹；路徑像顯示內容、或 JSON 含 `loc_name` 等鍵才收。
//! - 資料包 gameplay JSON 只翻「顯示欄位白名單」（`loc_*`／`flavor_text`／`effect_tip`…），
//!   不掃全部字串，避免把 particle type、guid、id 翻壞。
//! - **進度（advancement／advancements）同樣只翻顯示欄位**，絕不改 requirements／frame／translate。
//!
//! 只輸出到 work 目錄，不直接改實例。

use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::convert::convert_s2tw_batch;
use super::deepseek::translate_plain_strings_mapped;
use super::mech_tokens::{
    is_ascii_enum_token, is_bracket_meta_token, is_fancymenu_display_key,
    is_fancymenu_translatable_source, is_mechanism_path_segment, is_origins_powers_path,
    is_poisoned_mech_translation, is_resource_path_token, is_sentence_only_aggressive_key,
    looks_like_display_sentence,
};
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
    "landing_text",
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
    "armorsetname",
    "customtooltips",
    "choosekittext",
    "firstjoinmessage",
    "backupremindermessage",
];

/// 只在已判定為玩家顯示內容的資料夾啟用，避免把一般 JSON 的 name/id 當成文字。
const AGGRESSIVE_DISPLAY_FIELD_KEYS: &[&str] = &[
    "name",
    "display_name",
    "displayname",
    "display",
    "body",
    "content",
    "page",
    "pages",
    "category",
    "help",
    "armorsetname",
    "customtooltips",
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
    let mut ns_by_src: HashMap<String, String> = HashMap::new();
    let mut parse_failures: Vec<String> = Vec::new();

    for path in &files {
        match load_payload(path) {
            Ok(payload) => {
                for s in payload.collect_strings() {
                    if should_translate_overlay_string(&s) && seen.insert(s.clone(), ()).is_none() {
                        super::shared_identity::remember_ns(
                            &mut ns_by_src,
                            &s,
                            &payload.path,
                            scope,
                        );
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
                let translated = translate_plain_strings_mapped(batch, scope, &ns_by_src, |pct, msg| {
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

    let _ = super::shared_tm::contribute_plain_pairs(&map, &ns_by_src, "overlay", scope);

    // 3) 寫出（Patchouli：en_us 路徑同步寫 zh_tw，避免只改英檔卻算「完成」）
    on_progress(88, "覆寫文字：寫出檔案…");
    let mut written = 0usize;
    let mut patchouli_zh_tw = 0usize;
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
            fs::write(&out_path, &new_bytes)
                .map_err(|e| format!("{}: {e}", out_path.display()))?;
            written += 1;
            if let Some(zh_rel) = book_en_to_zh_tw_rel(rel) {
                let zh_out = output_dir.join(&zh_rel);
                if let Some(parent) = zh_out.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                fs::write(&zh_out, &new_bytes)
                    .map_err(|e| format!("{}: {e}", zh_out.display()))?;
                patchouli_zh_tw += 1;
                written += 1;
            }
        }
    }

    let coverage_hint = if unique.is_empty() {
        String::new()
    } else {
        let translated = map.len();
        let pct = (translated as f64 * 100.0) / unique.len() as f64;
        format!(
            "；覆寫字串覆蓋約 {:.0}%（{translated}/{} 唯一可譯）",
            pct,
            unique.len()
        )
    };
    let note = format!(
        "覆寫文字：掃描 {} 檔、唯一字串 {}、翻譯表 {} 條、寫出 {} 檔{}{}（patchouli／openloader／kubejs／datapacks／fancymenu／顯示型 config）。流程完成時會直接套用到遊戲。",
        file_payloads.len(),
        unique.len(),
        map.len(),
        written,
        if patchouli_zh_tw > 0 {
            format!("、另寫 Patchouli zh_tw {patchouli_zh_tw} 檔")
        } else {
            String::new()
        },
        coverage_hint,
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

    // patchouli_books/**/*.json／書頁 txt
    let pb = mc.join("patchouli_books");
    if pb.is_dir() {
        for p in walk_files(&pb, &["json"]) {
            push(p);
        }
        for p in walk_files(&pb, &["txt"]) {
            if is_book_locale_txt(&p) {
                push(p);
            }
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
        for p in walk_files(root, &["txt"]) {
            if is_book_locale_txt(&p) {
                push(p);
            }
        }
    }

    // 部分整合包把玩家看得到的文字放在 config，而不是 datapack。
    // 只依顯示路徑／欄位白名單收取，避免把整個 config 當成語言檔。
    let config_root = mc.join("config");
    if config_root.is_dir() {
        for p in walk_files(&config_root, &["json", "json5", "txt", "properties", "local"]) {
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
            for p in walk_files(&fm, &["txt", "json", "properties", "local"]) {
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
fn is_advancement_path(path: &Path) -> bool {
    // 1.20 複數 advancements；1.21+ 單數 advancement
    path_has_segment(path, "advancements") || path_has_segment(path, "advancement")
}

fn path_lower_slash(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn is_pack_text_json(path: &Path) -> bool {
    let lower = path_lower_slash(path);
    // Origins／powers 交給 origins.rs，避免 Aggressive 雙寫 condition 樹
    if is_origins_powers_path(&lower) {
        return false;
    }
    // 1) 標準文字路徑
    if path_has_segment(path, "lang")
        || is_advancement_path(path)
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
    is_mechanism_path_segment(&path_lower_slash(path))
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
        // /powers/ /origins/ 不在此列：交給 origins.rs
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
        "firstjoinmessage",
        "deathbackup",
    ]
    .iter()
    .any(|segment| path_has_segment(path, segment))
        || path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| {
                let l = s.to_ascii_lowercase();
                l.contains("firstjoin")
                    || l.contains("deathbackup")
                    || l == "starterkit"
                    || l.ends_with("message")
            })
            .unwrap_or(false)
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
        || s.contains("\"armorSetName\"")
        || s.contains("\"customTooltips\"")
        || s.contains("\"chooseKitText\"")
        || s.contains("\"firstJoinMessage\"")
        || s.contains("\"backupReminderMessage\"")
        || s.contains("description =")
        || s.contains("description=")
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
    /// Citadel／書頁內文：整檔當一段（無引號散文）
    WholeText,
    Properties,
}

/// 字串擷取模式：lang/書本可掃全部；gameplay 資料包只碰顯示欄位。
#[derive(Clone, Copy, PartialEq, Eq)]
enum StringMode {
    All,
    DisplayFieldsOnly,
    AggressiveDisplayFields,
    /// FancyMenu layout：只翻 label／description 等顯示鍵。
    FancyMenuDisplayKeys,
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
                let skip_book = is_citadel_style_book_json(&self.path);
                match self.mode {
                    StringMode::All => collect_json_strings(&v, &mut out),
                    StringMode::DisplayFieldsOnly => {
                        collect_display_field_strings(&v, &mut out, false, skip_book)
                    }
                    StringMode::AggressiveDisplayFields => {
                        collect_display_field_strings(&v, &mut out, true, skip_book)
                    }
                    StringMode::FancyMenuDisplayKeys => {
                        collect_display_field_strings(&v, &mut out, false, skip_book)
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
            FileKind::WholeText => {
                let t = self.raw.trim();
                if t.is_empty() {
                    Vec::new()
                } else {
                    vec![t.to_string()]
                }
            }
            FileKind::Properties => {
                let fancy = self.mode == StringMode::FancyMenuDisplayKeys;
                self.raw
                    .lines()
                    .filter_map(|line| property_kv(line))
                    .filter(|(k, v)| {
                        if !fancy {
                            return true;
                        }
                        if !is_fancymenu_display_key(k) {
                            return false;
                        }
                        // source：只收可讀句，擋圖檔／[source:…] 路徑
                        if k.eq_ignore_ascii_case("source") {
                            return is_fancymenu_translatable_source(v);
                        }
                        true
                    })
                    .map(|(_, v)| v.to_string())
                    .collect()
            }
        }
    }

    fn apply(&self, map: &HashMap<String, String>) -> Result<Option<Vec<u8>>, String> {
        match self.kind {
            FileKind::Json => {
                let mut v: Value = super::lenient_json::parse(&self.raw)
                    .map_err(|e| format!("{}: {e}", self.path.display()))?;
                let skip_book = is_citadel_style_book_json(&self.path);
                let changed = match self.mode {
                    StringMode::All => apply_json_strings(&mut v, map),
                    StringMode::DisplayFieldsOnly | StringMode::FancyMenuDisplayKeys => {
                        apply_display_field_strings(&mut v, map, false, skip_book)
                    }
                    StringMode::AggressiveDisplayFields => {
                        apply_display_field_strings(&mut v, map, true, skip_book)
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
            FileKind::WholeText => {
                let trimmed = self.raw.trim();
                let Some(replacement) = map.get(trimmed).or_else(|| map.get(&self.raw)) else {
                    return Ok(None);
                };
                if replacement == trimmed || replacement == &self.raw {
                    return Ok(None);
                }
                let mut output = replacement.clone();
                if self.raw.ends_with('\n') && !output.ends_with('\n') {
                    output.push('\n');
                }
                Ok(Some(output.into_bytes()))
            }
            FileKind::Properties => {
                let fancy = self.mode == StringMode::FancyMenuDisplayKeys;
                let mut changed = false;
                let mut output = String::with_capacity(self.raw.len());
                for line in self.raw.split_inclusive('\n') {
                    let has_newline = line.ends_with('\n');
                    let content = line.strip_suffix('\n').unwrap_or(line);
                    let replacement = property_replaced_line(content, map, fancy);
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
    // 進度 JSON：只翻 display 顯示欄位。全字串會改壞 requirements／frame／translate。
    if is_advancement_path(path) {
        return StringMode::DisplayFieldsOnly;
    }
    // lang：玩家文案多，沿用全字串
    if path_has_segment(path, "lang") {
        return StringMode::All;
    }
    // Patchouli：停用 All，避免 type／recipe／anchor／flag 被翻壞
    if path_has_segment(path, "patchouli_books") || path_has_segment(path, "patchouli") {
        return StringMode::AggressiveDisplayFields;
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

/// Citadel 系書頁 JSON（`assets/*/book/**/*.json`）：`text`／`parent`／`linked_page` 是檔名，不是內文。
fn is_citadel_style_book_json(path: &Path) -> bool {
    let lower = path_lower_slash(path);
    if lower.contains("patchouli") {
        return false;
    }
    let is_json = lower.ends_with(".json") || lower.ends_with(".json5");
    is_json && lower.contains("/book/")
}

fn is_book_resource_id_key(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "parent" | "text" | "linked_page" | "linkedpage"
    )
}

fn is_book_content_path(path: &Path) -> bool {
    let lower = path_lower_slash(path);
    lower.contains("/book/")
        || lower.contains("patchouli_books")
        || lower.contains("/patchouli/")
}

/// Citadel／Patchouli 的書頁內文（en_us 待譯；zh_cn 轉台灣繁後寫 zh_tw）。
fn is_book_locale_txt(path: &Path) -> bool {
    let lower = path_lower_slash(path);
    if !lower.ends_with(".txt") || !is_book_content_path(path) {
        return false;
    }
    lower.contains("/en_us/")
        || lower.ends_with("/en_us.txt")
        || lower.contains("/zh_cn/")
        || lower.ends_with("/zh_cn.txt")
}

fn is_book_prose_txt(path: &Path) -> bool {
    let lower = path_lower_slash(path);
    lower.ends_with(".txt") && is_book_content_path(path)
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

    if ext == "properties" || ext == "local" {
        return Ok(FilePayload {
            path: path.to_path_buf(),
            kind: FileKind::Properties,
            mode: StringMode::All,
            raw,
        });
    }

    // FancyMenu layout：只翻顯示鍵，避免 anchor_point／meta 被共享庫或 AI 翻壞
    if path_has_segment(path, "fancymenu")
        && (ext == "txt" || ext.is_empty())
        && raw.lines().any(|l| {
            let t = l.trim();
            !t.is_empty()
                && !t.starts_with('#')
                && t.contains('=')
                && !t.trim_start().starts_with('{')
        })
    {
        return Ok(FilePayload {
            path: path.to_path_buf(),
            kind: FileKind::Properties,
            mode: StringMode::FancyMenuDisplayKeys,
            raw,
        });
    }

    // starterkit descriptions：純文字整檔當可譯
    if path_has_segment(path, "starterkit") && (ext == "txt" || ext.is_empty()) {
        return Ok(FilePayload {
            path: path.to_path_buf(),
            kind: FileKind::Markdown,
            mode: StringMode::All,
            raw,
        });
    }

    // Citadel／Patchouli 書頁內文：無引號散文，整檔當一段
    if is_book_prose_txt(path) {
        return Ok(FilePayload {
            path: path.to_path_buf(),
            kind: FileKind::WholeText,
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
                if t != s && !is_poisoned_mech_translation(s, t) {
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
fn collect_display_field_strings(
    v: &Value,
    out: &mut Vec<String>,
    aggressive: bool,
    skip_book_path_ids: bool,
) {
    match v {
        Value::Object(m) => {
            for (k, child) in m {
                if skip_book_path_ids && is_book_resource_id_key(k) {
                    collect_display_field_strings(child, out, aggressive, skip_book_path_ids);
                } else if is_display_field_key_for_mode(k, aggressive) {
                    if aggressive && is_sentence_only_aggressive_key(k) {
                        push_sentence_only_display_value(child, out, aggressive);
                    } else {
                        push_display_value(child, out, aggressive);
                    }
                } else {
                    collect_display_field_strings(child, out, aggressive, skip_book_path_ids);
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                collect_display_field_strings(x, out, aggressive, skip_book_path_ids);
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
                    if aggressive && is_sentence_only_aggressive_key(k) {
                        push_sentence_only_display_value(child, out, aggressive);
                    } else {
                        push_display_value(child, out, aggressive);
                    }
                }
            }
        }
        _ => {}
    }
}

/// `category` 等：僅句子才進候選（純 id 如 Combat／mining 跳過）。
fn push_sentence_only_display_value(v: &Value, out: &mut Vec<String>, aggressive: bool) {
    match v {
        Value::String(s) => {
            if looks_like_display_sentence(s) {
                out.push(s.clone());
            }
        }
        Value::Array(a) => {
            for x in a {
                push_sentence_only_display_value(x, out, aggressive);
            }
        }
        Value::Object(m) => {
            for (k, child) in m {
                let kl = k.to_ascii_lowercase();
                if kl == "text" || kl == "extra" || is_display_field_key_for_mode(k, aggressive) {
                    push_sentence_only_display_value(child, out, aggressive);
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
    skip_book_path_ids: bool,
) -> bool {
    match v {
        Value::Object(m) => {
            let mut any = false;
            // 先收集鍵，避免邊改邊借
            let keys: Vec<String> = m.keys().cloned().collect();
            for k in keys {
                let skip_id = skip_book_path_ids && is_book_resource_id_key(&k);
                let is_disp = !skip_id && is_display_field_key_for_mode(&k, aggressive);
                if let Some(child) = m.get_mut(&k) {
                    if is_disp {
                        let sentence_only = aggressive && is_sentence_only_aggressive_key(&k);
                        if apply_display_value(child, map, aggressive, sentence_only) {
                            any = true;
                        }
                    } else if apply_display_field_strings(
                        child,
                        map,
                        aggressive,
                        skip_book_path_ids,
                    ) {
                        any = true;
                    }
                }
            }
            any
        }
        Value::Array(a) => {
            let mut any = false;
            for x in a {
                if apply_display_field_strings(x, map, aggressive, skip_book_path_ids) {
                    any = true;
                }
            }
            any
        }
        _ => false,
    }
}

fn apply_display_value(
    v: &mut Value,
    map: &HashMap<String, String>,
    aggressive: bool,
    sentence_only: bool,
) -> bool {
    match v {
        Value::String(s) => {
            if sentence_only && !looks_like_display_sentence(s) {
                return false;
            }
            if let Some(t) = map.get(s) {
                if t != s && !is_poisoned_mech_translation(s, t) {
                    *s = t.clone();
                    return true;
                }
            }
            false
        }
        Value::Array(a) => {
            let mut any = false;
            for x in a {
                if apply_display_value(x, map, aggressive, sentence_only) {
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
                    let child_sentence =
                        sentence_only || (aggressive && is_sentence_only_aggressive_key(&k));
                    if let Some(child) = m.get_mut(&k) {
                        if apply_display_value(child, map, aggressive, child_sentence) {
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
    // ASCII enum／snake／kebab id、資源檔名／相對路徑不當顯示文
    if is_ascii_enum_token(t) || is_bracket_meta_token(t) || is_resource_path_token(t) {
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

fn property_kv(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
        return None;
    }
    let separator = trimmed.find('=').or_else(|| trimmed.find(':'))?;
    let key = trimmed.get(..separator)?.trim();
    let value = trimmed.get(separator + 1..)?.trim();
    if key.is_empty() || value.is_empty() {
        return None;
    }
    Some((key, strip_wrapping_quotes(value)))
}

fn strip_wrapping_quotes(s: &str) -> &str {
    let t = s.trim();
    if t.len() >= 2 {
        let bytes = t.as_bytes();
        if (bytes[0] == b'"' && bytes[t.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[t.len() - 1] == b'\'')
        {
            return &t[1..t.len() - 1];
        }
    }
    t
}

/// Patchouli／Citadel 書 `…/en_us/` 或 `…/zh_cn/` → 同相對路徑的 `zh_tw`（供遊戲選繁中）。
fn book_en_to_zh_tw_rel(rel: &Path) -> Option<PathBuf> {
    let s = rel.to_string_lossy().replace('\\', "/");
    let lower = s.to_ascii_lowercase();
    if !is_book_content_path(rel) {
        return None;
    }
    let from = if lower.contains("/en_us/") || lower.ends_with("/en_us") {
        ("/en_us/", "/en_us", "\\en_us\\")
    } else if lower.contains("/zh_cn/") || lower.ends_with("/zh_cn") {
        ("/zh_cn/", "/zh_cn", "\\zh_cn\\")
    } else if lower.contains("/zh_hk/") || lower.ends_with("/zh_hk") {
        ("/zh_hk/", "/zh_hk", "\\zh_hk\\")
    } else {
        return None;
    };
    let replaced = s
        .replace(from.0, "/zh_tw/")
        .replace(from.1, "/zh_tw")
        .replace(from.2, "\\zh_tw\\");
    if replaced == s {
        return None;
    }
    Some(PathBuf::from(replaced))
}

fn property_replaced_line(line: &str, map: &HashMap<String, String>, fancy_only: bool) -> String {
    let Some(separator) = line.find('=').or_else(|| line.find(':')) else {
        return line.to_string();
    };
    let key = line[..separator].trim();
    if fancy_only && !is_fancymenu_display_key(key) {
        return line.to_string();
    }
    let after = &line[separator + 1..];
    let value = after.trim();
    let lookup = strip_wrapping_quotes(value);
    if fancy_only
        && key.eq_ignore_ascii_case("source")
        && !is_fancymenu_translatable_source(lookup)
    {
        return line.to_string();
    }
    let Some(replacement) = map.get(lookup).or_else(|| map.get(value)) else {
        return line.to_string();
    };
    if is_poisoned_mech_translation(lookup, replacement)
        || is_poisoned_mech_translation(value, replacement)
    {
        return line.to_string();
    }
    let leading_len = after.len() - after.trim_start().len();
    let trailing_len = after.len() - after.trim_end().len();
    let leading = &after[..leading_len];
    let trailing = if trailing_len == 0 {
        ""
    } else {
        &after[after.len() - trailing_len..]
    };
    let new_value = if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        let q = &value[..1];
        format!("{q}{replacement}{q}")
    } else {
        replacement.clone()
    };
    format!("{}{leading}{new_value}{trailing}", &line[..=separator])
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
        assert!(is_config_text_path(Path::new(
            "minecraft/config/firstjoinmessage/config.json5"
        )));
        assert!(is_config_text_path(Path::new(
            "minecraft/config/deathbackup/config.json"
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
            property_replaced_line("welcome =  Welcome to the pack  ", &map, false),
            "welcome =  歡迎來到整合包  "
        );
        assert_eq!(property_kv("# welcome = Hello"), None);
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

    #[test]
    fn advancement_paths_use_display_fields_only() {
        assert!(matches!(
            string_mode_for_path(Path::new(
                "data/farmersdelight/advancements/main/get_rich_soil.json"
            )),
            StringMode::DisplayFieldsOnly
        ));
        assert!(matches!(
            string_mode_for_path(Path::new(
                "data/minecraft/advancement/story/mine_stone.json"
            )),
            StringMode::DisplayFieldsOnly
        ));
        assert!(is_pack_text_json(Path::new(
            "kubejs/data/pack/advancement/root.json"
        )));
    }

    #[test]
    fn patchouli_uses_aggressive_not_all() {
        assert!(matches!(
            string_mode_for_path(Path::new(
                "patchouli_books/guide/en_us/entries/intro.json"
            )),
            StringMode::AggressiveDisplayFields
        ));
        assert!(matches!(
            string_mode_for_path(Path::new("assets/mod/patchouli/book.json")),
            StringMode::AggressiveDisplayFields
        ));
        assert!(matches!(
            string_mode_for_path(Path::new("assets/mod/lang/en_us.json")),
            StringMode::All
        ));
    }

    #[test]
    fn patchouli_apply_keeps_type_recipe_anchor() {
        let raw = r#"{
  "name": "Intro",
  "category": "Combat",
  "pages": [
    {
      "type": "crafting",
      "recipe": "minecraft:stick",
      "anchor": "stick_page",
      "title": "Make a Stick",
      "text": "Craft sticks from planks."
    }
  ]
}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut collected = Vec::new();
        collect_display_field_strings(&value, &mut collected, true, false);
        assert!(
            !collected
                .iter()
                .any(|s| s == "crafting" || s == "minecraft:stick" || s == "stick_page"),
            "mechanism fields must not be collected: {collected:?}"
        );
        assert!(
            !collected.iter().any(|s| s == "Combat"),
            "category id must not be collected: {collected:?}"
        );
        assert!(collected.iter().any(|s| s == "Make a Stick"), "{collected:?}");
        assert!(
            collected
                .iter()
                .any(|s| s == "Craft sticks from planks."),
            "{collected:?}"
        );
        assert!(collected.iter().any(|s| s == "Intro"), "{collected:?}");

        let mut map = HashMap::new();
        map.insert("Make a Stick".into(), "製作木棒".into());
        map.insert("Craft sticks from planks.".into(), "用木板合成木棒。".into());
        map.insert("crafting".into(), "合成".into());
        map.insert("Combat".into(), "戰鬥".into());
        let mut applied = value.clone();
        assert!(apply_display_field_strings(&mut applied, &map, true, false));
        assert_eq!(
            applied.pointer("/pages/0/type").and_then(|v| v.as_str()),
            Some("crafting")
        );
        assert_eq!(
            applied.pointer("/pages/0/recipe").and_then(|v| v.as_str()),
            Some("minecraft:stick")
        );
        assert_eq!(
            applied.pointer("/pages/0/anchor").and_then(|v| v.as_str()),
            Some("stick_page")
        );
        assert_eq!(
            applied.get("category").and_then(|v| v.as_str()),
            Some("Combat")
        );
        assert_eq!(
            applied.pointer("/pages/0/title").and_then(|v| v.as_str()),
            Some("製作木棒")
        );
    }

    #[test]
    fn overlay_filter_rejects_enum_tokens() {
        assert!(!should_translate_overlay_string("goal"));
        assert!(!should_translate_overlay_string("strawberry_crate"));
        assert!(!should_translate_overlay_string("root.txt"));
        assert!(!should_translate_overlay_string("alligator.json"));
        assert!(!should_translate_overlay_string("book/animal_dictionary/root"));
        assert!(should_translate_overlay_string("Get rich soil"));
        assert!(should_translate_overlay_string("Welcome to the pack"));
        assert!(!is_resource_path_token("hello"));
    }

    #[test]
    fn citadel_book_json_keeps_path_ids() {
        let raw = r#"{
  "parent": "root.json",
  "text": "root.txt",
  "title": "Alligator",
  "linked_page": "alligator.json"
}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut collected = Vec::new();
        collect_display_field_strings(&value, &mut collected, true, true);
        assert!(
            !collected.iter().any(|s| {
                s == "root.txt" || s == "root.json" || s == "alligator.json"
            }),
            "book path ids must not be collected: {collected:?}"
        );
        assert!(collected.iter().any(|s| s == "Alligator"), "{collected:?}");

        let mut map = HashMap::new();
        map.insert("root.txt".into(), "根.txt".into());
        map.insert("root.json".into(), "根.json".into());
        map.insert("alligator.json".into(), "短吻鱷.json".into());
        map.insert("Alligator".into(), "短吻鱷".into());
        let mut applied = value.clone();
        assert!(apply_display_field_strings(&mut applied, &map, true, true));
        assert_eq!(applied.get("text").and_then(|v| v.as_str()), Some("root.txt"));
        assert_eq!(
            applied.get("parent").and_then(|v| v.as_str()),
            Some("root.json")
        );
        assert_eq!(
            applied.get("linked_page").and_then(|v| v.as_str()),
            Some("alligator.json")
        );
        assert_eq!(applied.get("title").and_then(|v| v.as_str()), Some("短吻鱷"));
        assert!(is_citadel_style_book_json(Path::new(
            "assets/alexsmobs/book/animal_dictionary/root.json"
        )));
        assert!(!is_citadel_style_book_json(Path::new(
            "patchouli_books/guide/en_us/entries/intro.json"
        )));
    }

    #[test]
    fn landing_text_is_a_display_field() {
        let raw = r#"{
  "name": "Iron's Guidebook",
  "landing_text": "Iron's Spells 'n Spellbooks is an RPG-inspired spellcasting mod.",
  "book_texture": "patchouli:textures/gui/book_brown.png"
}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut collected = Vec::new();
        collect_display_field_strings(&value, &mut collected, true, false);
        assert!(
            collected.iter().any(|s| s.contains("RPG-inspired")),
            "{collected:?}"
        );
        let mut map = HashMap::new();
        map.insert(
            "Iron's Spells 'n Spellbooks is an RPG-inspired spellcasting mod.".into(),
            "鐵之魔法與法術書是一款 RPG 風格的施法模組。".into(),
        );
        map.insert("Iron's Guidebook".into(), "鐵之指南書".into());
        let mut applied = value.clone();
        assert!(apply_display_field_strings(&mut applied, &map, true, false));
        assert_eq!(
            applied.get("name").and_then(|v| v.as_str()),
            Some("鐵之指南書")
        );
        assert!(applied
            .get("landing_text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("施法模組"));
    }

    #[test]
    fn book_en_us_txt_maps_to_zh_tw_and_is_whole_file() {
        assert!(is_book_locale_txt(Path::new(
            "assets/alexsmobs/book/animal_dictionary/en_us/root.txt"
        )));
        assert!(!is_book_locale_txt(Path::new(
            "assets/alexsmobs/book/animal_dictionary/root.json"
        )));
        assert_eq!(
            book_en_to_zh_tw_rel(Path::new(
                "assets/alexsmobs/book/animal_dictionary/en_us/root.txt"
            )),
            Some(PathBuf::from(
                "assets/alexsmobs/book/animal_dictionary/zh_tw/root.txt"
            ))
        );
        assert_eq!(
            book_en_to_zh_tw_rel(Path::new(
                "assets/alexsmobs/book/animal_dictionary/zh_cn/root.txt"
            )),
            Some(PathBuf::from(
                "assets/alexsmobs/book/animal_dictionary/zh_tw/root.txt"
            ))
        );
        assert!(is_book_locale_txt(Path::new(
            "assets/alexsmobs/book/animal_dictionary/zh_cn/root.txt"
        )));
        let payload = FilePayload {
            path: PathBuf::from("assets/alexsmobs/book/animal_dictionary/en_us/root.txt"),
            kind: FileKind::WholeText,
            mode: StringMode::All,
            raw: "Numerous strange creatures inhabit the Overworld.\n".into(),
        };
        let collected = payload.collect_strings();
        assert_eq!(
            collected,
            vec!["Numerous strange creatures inhabit the Overworld."]
        );
        let mut map = HashMap::new();
        map.insert(
            "Numerous strange creatures inhabit the Overworld.".into(),
            "主世界棲息著許多奇特生物。".into(),
        );
        let out = payload.apply(&map).unwrap().unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("奇特生物"));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn powers_and_origins_paths_skipped_by_overlay() {
        assert!(!is_pack_text_json(Path::new(
            "data/origins/powers/climbing.json"
        )));
        assert!(!is_pack_text_json(Path::new(
            "kubejs/data/mod/origins/human.json"
        )));
        assert!(!is_display_content_path(Path::new(
            "data/mod/powers/foo.json"
        )));
    }

    #[test]
    fn advancement_apply_keeps_mechanism_ids_and_translates_literal_title() {
        let raw = r#"{
  "criteria": { "get_rich_soil": { "trigger": "minecraft:inventory_changed" } },
  "display": {
    "frame": "goal",
    "title": { "translate": "farmersdelight.advancement.get_rich_soil" },
    "description": { "text": "Get rich soil" }
  },
  "requirements": [["get_rich_soil"]]
}"#;
        let mut value: Value = serde_json::from_str(raw).unwrap();
        let mut collected = Vec::new();
        collect_display_field_strings(&value, &mut collected, false, false);
        assert!(
            !collected.iter().any(|s| s == "get_rich_soil" || s == "goal"),
            "mechanism strings must not be collected: {collected:?}"
        );
        assert!(
            !collected
                .iter()
                .any(|s| s == "farmersdelight.advancement.get_rich_soil"),
            "translate keys must not be collected: {collected:?}"
        );
        assert!(collected.iter().any(|s| s == "Get rich soil"), "{collected:?}");

        let mut map = HashMap::new();
        map.insert("Get rich soil".into(), "取得肥沃土壤".into());
        map.insert("get_rich_soil".into(), "獲得肥沃土壤".into());
        map.insert("goal".into(), "目標".into());
        map.insert(
            "farmersdelight.advancement.get_rich_soil".into(),
            "肥沃土壤".into(),
        );
        assert!(apply_display_field_strings(&mut value, &map, false, false));

        let display = value.get("display").unwrap();
        assert_eq!(display.get("frame").and_then(|v| v.as_str()), Some("goal"));
        assert_eq!(
            display
                .pointer("/title/translate")
                .and_then(|v| v.as_str()),
            Some("farmersdelight.advancement.get_rich_soil")
        );
        assert_eq!(
            display
                .pointer("/description/text")
                .and_then(|v| v.as_str()),
            Some("取得肥沃土壤")
        );
        assert_eq!(
            value
                .pointer("/requirements/0/0")
                .and_then(|v| v.as_str()),
            Some("get_rich_soil")
        );
        assert!(value.pointer("/criteria/get_rich_soil").is_some());
    }

    #[test]
    fn fancymenu_keeps_anchor_and_rejects_poisoned_hits() {
        let raw = "\
type = customization
anchor_point = mid-left
label = Multiplayer
custom_element_layer_name = Side Bar Left
[groups:][instances:]
";
        let payload = FilePayload {
            path: PathBuf::from("config/fancymenu/customization/Prominence.txt"),
            kind: FileKind::Properties,
            mode: StringMode::FancyMenuDisplayKeys,
            raw: raw.to_string(),
        };
        let collected = payload.collect_strings();
        assert!(collected.iter().any(|s| s == "Multiplayer"), "{collected:?}");
        assert!(
            !collected.iter().any(|s| s == "mid-left" || s.contains("Side Bar")),
            "structure/editor fields must not be collected: {collected:?}"
        );

        let mut map = HashMap::new();
        map.insert("Multiplayer".into(), "多人遊戲".into());
        map.insert("mid-left".into(), "中左".into());
        map.insert("Side Bar Left".into(), "左側欄".into());
        let out = payload.apply(&map).unwrap().expect("label should change");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("anchor_point = mid-left"), "{text}");
        assert!(text.contains("label = 多人遊戲"), "{text}");
        assert!(text.contains("custom_element_layer_name = Side Bar Left"), "{text}");
        assert!(!text.contains("中左"), "{text}");
    }

    #[test]
    fn fancymenu_text_v2_source_translates_lore_skips_assets() {
        let raw = "\
element_type = text_v2
source = You are a Flameborn.%n%%n%Stop the Void.
element_type = text_v2
source = The Talent Tree
source = [source:local]/config/fancymenu/assets/welcome_screen/who_you_are.png
label = Start Journey
";
        let payload = FilePayload {
            path: PathBuf::from("config/fancymenu/customization/welcomescreen_welcome_layout.txt"),
            kind: FileKind::Properties,
            mode: StringMode::FancyMenuDisplayKeys,
            raw: raw.to_string(),
        };
        let collected = payload.collect_strings();
        assert!(
            collected
                .iter()
                .any(|s| s.contains("Flameborn") && s.contains("%n%")),
            "{collected:?}"
        );
        assert!(collected.iter().any(|s| s == "The Talent Tree"), "{collected:?}");
        assert!(collected.iter().any(|s| s == "Start Journey"), "{collected:?}");
        assert!(
            !collected.iter().any(|s| s.contains("[source:local]") || s.ends_with(".png")),
            "{collected:?}"
        );

        let mut map = HashMap::new();
        map.insert(
            "You are a Flameborn.%n%%n%Stop the Void.".into(),
            "你是焰裔。%n%%n%阻止虛空。".into(),
        );
        map.insert("The Talent Tree".into(), "天賦樹".into());
        map.insert(
            "[source:local]/config/fancymenu/assets/welcome_screen/who_you_are.png".into(),
            "不該寫入.png".into(),
        );
        map.insert("Start Journey".into(), "開始旅程".into());
        let out = payload.apply(&map).unwrap().expect("display source/label change");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("你是焰裔。%n%%n%阻止虛空。"), "{text}");
        assert!(text.contains("source = 天賦樹"), "{text}");
        assert!(text.contains("label = 開始旅程"), "{text}");
        assert!(
            text.contains("[source:local]/config/fancymenu/assets/welcome_screen/who_you_are.png"),
            "{text}"
        );
        assert!(!text.contains("不該寫入"), "{text}");
    }
}
