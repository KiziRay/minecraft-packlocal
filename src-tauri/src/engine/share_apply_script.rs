//! S1：分享包收件端的套用／還原腳本與附帶清單。
//!
//! 原則：必須先確認，才能動收件人的檔。
//! - 套用前確認：是 Minecraft 遊戲資料夾、遊戲沒開著（判斷不了就停）、分享包完整（雜湊相符）、
//!   模組和分享者相同（不同就列出差異，輸入 y 才繼續，預設取消）。
//! - 覆蓋前把每個會被覆蓋的檔複製到 `<遊戲資料夾>/.mcpl-share-backup/<時間戳>/files/`（保留相對路徑）
//!   並寫清單；備份失敗就中止，一個檔都不覆蓋。
//! - 附還原腳本：只動「內容仍是分享包寫入的版本」的檔，被改過的不動並列出。
//!
//! 腳本一律 UTF-8 BOM＋CRLF：Windows PowerShell 5.1 讀無 BOM 的檔會把中文當成系統字碼頁而亂碼。
//! 確認輸入只收 ASCII 的 y：主控台輸入中文在部分 Windows 版本會讀成空字串。

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use walkdir::WalkDir;

use super::share_pack::CLOUD_URL_SHORTCUT_NAME;
use super::share_apply_script_text::{APPLY_BODY, APPLY_PARAMS, COMMON, README, RESTORE_BODY, RESTORE_PARAMS};

pub const APPLY_SCRIPT_NAME: &str = "套用翻譯.ps1";
/// 放進備份資料夾的還原腳本（ASCII 檔名，讓 .cmd 啟動器內容維持純 ASCII）。
pub const RESTORE_SCRIPT_NAME: &str = "restore.ps1";
pub const FILES_LIST_NAME: &str = "mcpl-share-files.tsv";
pub const MODS_LIST_NAME: &str = "mcpl-share-mods.tsv";
pub const README_NAME: &str = "說明-先看我.txt";
pub const BACKUP_DIR_NAME: &str = ".mcpl-share-backup";
pub const RESTORE_LAUNCHER_NAME: &str = "還原這次套用.cmd";

/// 分享包頂層的支援檔（不放進遊戲資料夾）。
pub const SUPPORT_FILES: &[&str] = &[APPLY_SCRIPT_NAME, RESTORE_SCRIPT_NAME, FILES_LIST_NAME, MODS_LIST_NAME, README_NAME];

/// 分享內容允許的頂層資料夾（與 share_pack::is_shareable_path 共用同一份；不含 mods）。
/// 收件端腳本只接受這些頂層資料夾，以及頂層的雲端捷徑檔。
pub const SHARE_TOP_LEVEL_DIRS: &[&str] = &[
    "resourcepacks", "patchouli_books", "kubejs", "minemenu", "datapacks",
    "defaultconfigs", "global_packs", "paxi", "data", "config",
];

pub fn is_support_file(top_level_name: &str) -> bool {
    SUPPORT_FILES.iter().any(|n| *n == top_level_name)
}

/// 檔案 SHA-256（小寫十六進位），串流讀取不整檔進記憶體。
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("讀取失敗 {}：{e}", path.display()))?;
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("讀取失敗 {}：{e}", path.display()))?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    Ok(ctx.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect())
}

/// 分享者的模組清單：`mods/` 頂層 *.jar 的「大小<TAB>檔名」，依檔名排序。沒有 mods 資料夾回 None。
pub fn mods_list_text(mods_dir: &Path) -> Option<String> {
    let entries = fs::read_dir(mods_dir).ok()?;
    let mut rows: Vec<(String, u64)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.to_ascii_lowercase().ends_with(".jar") {
                return None;
            }
            Some((name, e.metadata().ok()?.len()))
        })
        .collect();
    rows.sort_by_key(|(n, _)| n.to_lowercase());
    let mut out = String::from("# mcpl-share-mods v1\n");
    for (name, size) in rows {
        out.push_str(&format!("{size}\t{name}\n"));
    }
    Some(out)
}

/// 分享包裡要放進遊戲資料夾的檔（相對路徑，反斜線）；排除頂層支援檔、SFX 設定與 .7z。
fn payload_files(stage: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for entry in WalkDir::new(stage).min_depth(1) {
        let entry = entry.map_err(|e| format!("讀取暫存區失敗：{e}"))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(stage).map_err(|e| e.to_string())?;
        if rel.components().count() == 1 {
            let name = rel.to_string_lossy();
            if is_support_file(&name) || name == "sfx_config.txt" || name.to_ascii_lowercase().ends_with(".7z") {
                continue;
            }
        }
        out.push(rel.to_string_lossy().replace('/', "\\"));
    }
    out.sort();
    Ok(out)
}

/// 寫齊收件端需要的支援檔：檔案清單（含雜湊）、模組清單（有分享者遊戲資料夾時）、套用／還原腳本、說明。
pub fn write_support_files(stage: &Path, pack_zip_name: &str, sender_game: Option<&Path>) -> Result<(), String> {
    let files = payload_files(stage)?;
    if files.is_empty() {
        return Err("暫存區沒有可分享的安裝檔。".into());
    }
    let mut list = String::from("# mcpl-share-files v1\n");
    for rel in &files {
        list.push_str(&format!("{}\t{rel}\n", sha256_file(&stage.join(rel))?));
    }
    fs::write(stage.join(FILES_LIST_NAME), list).map_err(|e| format!("寫入檔案清單失敗：{e}"))?;
    let mods_path = stage.join(MODS_LIST_NAME);
    let _ = fs::remove_file(&mods_path);
    if let Some(mods) = sender_game.and_then(|g| mods_list_text(&g.join("mods"))) {
        fs::write(&mods_path, mods).map_err(|e| format!("寫入模組清單失敗：{e}"))?;
    }
    write_bom_text(&stage.join(APPLY_SCRIPT_NAME), &apply_script_text(pack_zip_name))?;
    write_bom_text(&stage.join(RESTORE_SCRIPT_NAME), &restore_script_text())?;
    write_bom_text(&stage.join(README_NAME), &readme_text(pack_zip_name))?;
    Ok(())
}

fn write_bom_text(path: &Path, text: &str) -> Result<(), String> {
    let body = if text.starts_with('\u{FEFF}') { text.to_string() } else { format!("\u{FEFF}{text}") };
    fs::write(path, body).map_err(|e| format!("寫入 {} 失敗：{e}", path.display()))
}

fn crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

pub fn apply_script_text(pack_zip_name: &str) -> String {
    let body = APPLY_BODY.replace("__PACK_ZIP__", &pack_zip_name.replace('\'', "''"));
    crlf(&fill_names(&format!("\u{FEFF}{APPLY_PARAMS}{COMMON}{body}")))
}

pub fn restore_script_text() -> String {
    crlf(&fill_names(&format!("\u{FEFF}{RESTORE_PARAMS}{COMMON}{RESTORE_BODY}")))
}

pub fn readme_text(pack_zip_name: &str) -> String {
    crlf(&fill_names(&README.replace("__PACK_ZIP__", pack_zip_name)))
}

/// 腳本與說明裡的檔名、資料夾名一律取自本檔常數（單一真相源）。
fn fill_names(text: &str) -> String {
    text.replace("__BACKUP_DIR__", BACKUP_DIR_NAME)
        .replace("__RESTORE_CMD__", RESTORE_LAUNCHER_NAME)
        .replace("__FILES_LIST__", FILES_LIST_NAME)
        .replace("__MODS_LIST__", MODS_LIST_NAME)
        .replace("__RESTORE_PS1__", RESTORE_SCRIPT_NAME)
        .replace("__SHORTCUT__", &CLOUD_URL_SHORTCUT_NAME.replace('\'', "''"))
        .replace("__TOP_DIRS__", &SHARE_TOP_LEVEL_DIRS.iter().map(|d| format!("'{d}'")).collect::<Vec<_>>().join(", "))
}
