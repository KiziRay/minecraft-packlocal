//! 翻譯後模組檔的來源指紋。
//!
//! 翻譯時：每做出一個翻譯後的模組檔，就記下它是從哪個模組檔（檔名＋指紋）做出來的，
//! 存在翻譯結果資料夾的 `.mcpl-jar-sources.json`。
//! 套用前：遊戲裡的模組檔必須還是同一個（指紋相同；或遊戲裡已是工具放的翻譯版、
//! 且當初被覆蓋的原檔指紋相同）。模組被更新、改名或刪除 → 不放進遊戲，列為「模組已更新，需重新翻譯」。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::apply_plan::{ApplyPlan, Group, ItemSource};
use super::apply_record::{self, ApplyRecord};
use super::mcpl_marker as mk;
use super::paths::long_path;

pub const SOURCES_FILE: &str = ".mcpl-jar-sources.json";

/// 翻譯時多個背景工作會同時記錄，讀改寫要排隊。
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JarSource {
    /// 翻譯時遊戲裡的模組檔名
    pub name: String,
    /// 翻譯時遊戲裡模組檔的指紋
    pub sha256: String,
}

fn sources_path(work_root: &Path) -> PathBuf {
    work_root.join(SOURCES_FILE)
}

fn key_of(relative: &Path) -> String {
    relative.to_string_lossy().replace('\\', "/")
}

fn read(work_root: &Path) -> BTreeMap<String, JarSource> {
    fs::read_to_string(long_path(&sources_path(work_root)))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// 重新產生全部翻譯後模組檔之前清掉舊紀錄。
pub fn reset(work_root: &Path) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = fs::remove_file(long_path(&sources_path(work_root)));
}

/// 記下 `jar-translated/<relative>` 是從哪個模組檔做出來的（同一個位置以最新一次為準）。
pub fn record_source(work_root: &Path, relative: &Path, source_jar: &Path) -> Result<(), String> {
    let sha256 = apply_record::file_sha256(source_jar)
        .ok_or_else(|| format!("讀不到模組檔，無法記下它的指紋：{}", source_jar.display()))?;
    let name = source_jar.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut map = read(work_root);
    map.insert(key_of(relative), JarSource { name, sha256 });
    let text = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
    apply_record::write_atomic(&sources_path(work_root), text.as_bytes())
        .map_err(|e| format!("無法記下模組檔的指紋：{e}"))
}

/// 從套用清單拿掉「遊戲裡的模組已經不是翻譯時那一個」的翻譯後模組檔，回傳被拿掉的遊戲相對路徑。
pub fn drop_outdated(work_root: &Path, mc: &Path, plan: &mut ApplyPlan, record: &ApplyRecord) -> Vec<String> {
    let sources = read(work_root);
    let translated_root = work_root.join("jar-translated");
    let mut outdated = Vec::new();
    plan.items.retain(|item| {
        let ItemSource::File(file) = &item.source else {
            return true;
        };
        if !matches!(item.group, Group::Mods) {
            return true;
        }
        let key = file.strip_prefix(&translated_root).map(key_of).unwrap_or_default();
        let rel = apply_record::rel_key(mc, &item.dest);
        // 沒有來源紀錄（舊版工具做的結果）也無法確認是同一個模組：不放
        let same = sources.get(&key).is_some_and(|source| game_matches(mc, &rel, &item.dest, source, record));
        if !same {
            crate::dev_log!("apply", "{rel}：翻譯後模組已更新、改名或刪除，不放進遊戲");
            outdated.push(rel);
        }
        same
    });
    outdated
}

fn game_matches(mc: &Path, rel: &str, dest: &Path, source: &JarSource, record: &ApplyRecord) -> bool {
    let Some(current) = apply_record::file_sha256(dest) else {
        // 刪除或改名了
        return false;
    };
    if current == source.sha256 {
        return true;
    }
    if !record.is_tool_version(rel, Some(&current)) {
        return false;
    }
    // 遊戲裡是工具放的翻譯版：比對當初被覆蓋的原檔指紋。標記不見時交給套用時的標記檢查
    // （補不回來會列為無法確認、不覆蓋）。
    match mk::read_file_marker(mc, rel) {
        Some(marker) => marker.original_sha256 == source.sha256,
        None => true,
    }
}
