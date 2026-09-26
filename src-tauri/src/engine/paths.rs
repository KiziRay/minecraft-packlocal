//! 統一「工具資料根目錄」解析：可攜式優先（執行檔旁），既有安裝仍讀舊位置。
//!
//! 背景：改版前全部 11 個子系統（金鑰、Discord／GPT 憑證、術語表、翻譯記憶、
//! 掃描快取、工作資料夾、本地模型……）固定存在 `%APPDATA%\modpack-i18n-tool\`
//! （Windows 上等於 C 槽）。使用者要求改成不擅自佔用系統槽——新安裝一律改存
//! 執行檔所在資料夾底下的 `modpack-i18n-data\`；但既有安裝的資料不強制搬遷，
//! 沿用第九輪 `local_llm::server::resolve_state_path()` 已經驗證過的模式：
//! 「新根已有東西就用新根，否則舊根有就沿用舊根（讀寫都留在舊根），兩邊都沒有
//! 才落在新根」。
//!
//! 不變式：全 repo 除了本檔案，其餘檔案不得再直接呼叫 `dirs::data_dir()` /
//! `dirs::data_local_dir()`——一律經由這裡的 `resolve_file`（或 `legacy_local_root()`
//! 搭配 `local_llm::server::resolve_state_path()` 那種專屬三代疊代解析）。

use std::path::{Path, PathBuf};

const APP_FOLDER: &str = "modpack-i18n-tool";
const PORTABLE_FOLDER: &str = "modpack-i18n-data";

/// 新預設根：執行檔所在資料夾底下的 `modpack-i18n-data`。
///
/// 刻意用 `current_exe()` 的父目錄，不用 `current_dir()`——後者受「從哪裡啟動」
/// 影響（捷徑的「起始位置」、命令列所在目錄都可能不同），對可攜式工具來說不穩定；
/// 執行檔自己的路徑才是使用者實際「這個工具在哪」的認知。取不到執行檔路徑
/// （理論上不會發生，保險起見）才退回目前工作目錄。
pub fn portable_root() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(PORTABLE_FOLDER)
}

/// 舊版根（`%APPDATA%`，Roaming）——第九輪以前，除本地模型／CFPA 快取外的
/// 全部子系統都用這個。
pub fn legacy_roaming_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_FOLDER)
}

/// 舊版根（`%LOCALAPPDATA%`，Local）——本地模型／CFPA 快取在第九輪以前用這個。
pub fn legacy_local_root() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_FOLDER)
}

/// 純函式核心：給定新／舊兩個根目錄，回傳實際要用的路徑。抽出來是因為
/// `portable_root()`／`legacy_*_root()` 讀真實環境（執行檔路徑、環境變數），
/// 測試裡沒辦法安全覆寫；純函式版本可以直接餵假路徑驗證分支邏輯。
fn resolve_with_roots(rel: &Path, new_root: &Path, legacy_root: &Path) -> PathBuf {
    let new_path = new_root.join(rel);
    if new_path.exists() {
        return new_path;
    }
    let legacy_path = legacy_root.join(rel);
    if legacy_path.exists() {
        return legacy_path;
    }
    new_path
}

/// 解析「資料根目錄下的某個相對路徑」（可以是檔案，也可以是資料夾）。
///
/// 新根（可攜式）已存在就用新根；否則舊根（Roaming）存在就沿用舊根；
/// 都不存在則落在新根（全新安裝從這裡開始）。
///
/// 本地模型另有專屬的三代（可攜式／Roaming／Local）疊代解析，見
/// `local_llm::server::resolve_state_path`，不走這個通用版本。
pub fn resolve_file(rel: &Path) -> PathBuf {
    resolve_with_roots(rel, &portable_root(), &legacy_roaming_root())
}

/// 目前實際在用的資料根：可攜式根若已存在就是它，否則舊根若存在就是舊根。
///
/// 給設定頁顯示「你的資料現在放哪」用。使用者反映「不希望工具在系統碟建檔」，
/// 但既有安裝的資料留在 `%APPDATA%` 是刻意的（不強制搬遷），所以必須讓使用者
/// 看得到現況、也能自己決定要不要搬。
pub fn active_root() -> PathBuf {
    let portable = portable_root();
    if portable.exists() {
        return portable;
    }
    let legacy = legacy_roaming_root();
    if legacy.exists() {
        return legacy;
    }
    portable
}

/// 現在是不是已經用可攜式（跟著工具走）的資料夾。
pub fn is_using_portable_root() -> bool {
    active_root() == portable_root()
}

/// 遞迴計算資料夾大小，供「要搬多少東西」的提示。讀不到的項目直接略過。
pub fn dir_size_bytes(root: &Path) -> u64 {
    if !root.is_dir() {
        return 0;
    }
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// 把舊根的資料**複製**到可攜式根。
///
/// 刻意用複製而不是移動：中途失敗（磁碟滿、檔案被佔用）時，舊資料仍然完整，
/// 使用者不會兩邊都壞掉。複製完成後舊資料夾原地保留當備份，由使用者自行刪除。
pub fn migrate_legacy_to_portable() -> Result<(usize, u64), String> {
    let legacy = legacy_roaming_root();
    let portable = portable_root();
    if !legacy.is_dir() {
        return Err("找不到舊的資料資料夾，不需要搬移。".into());
    }
    if legacy == portable {
        return Err("新舊位置相同，不需要搬移。".into());
    }
    std::fs::create_dir_all(&portable).map_err(|e| format!("無法建立新資料夾：{e}"))?;
    let mut files = 0usize;
    let mut bytes = 0u64;
    for entry in walkdir::WalkDir::new(&legacy).into_iter().filter_map(Result::ok) {
        let src = entry.path();
        if !src.is_file() {
            continue;
        }
        let rel = match src.strip_prefix(&legacy) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let dst = portable.join(rel);
        // 已經存在的不覆蓋：新根的內容比較新，搬移不該把它蓋掉
        if dst.exists() {
            continue;
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("無法建立 {}：{e}", parent.display()))?;
        }
        let copied = std::fs::copy(src, &dst)
            .map_err(|e| format!("複製 {} 失敗：{e}", src.display()))?;
        files += 1;
        bytes += copied;
    }
    Ok((files, bytes))
}

/// 超過這個長度才轉成 Windows 長路徑形式。留一段餘裕：資料夾本身不能超過 248 字元。
const LONG_PATH_THRESHOLD: usize = 240;

/// 備份與套用用的路徑：太長（接近 Windows 260 字元上限）時轉成 `\\?\` 長路徑形式。
///
/// 只給「實際讀寫檔案」的那一步用；顯示給玩家、寫進套用紀錄的相對路徑一律用原本的形式，
/// 不然紀錄裡會混進 `\\?\` 前綴，之後比對不到同一個檔案。
pub fn long_path(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if !cfg!(windows) || raw.len() < LONG_PATH_THRESHOLD {
        return path.to_path_buf();
    }
    PathBuf::from(verbatim_form(&raw))
}

/// 純字串轉換（抽出來讓非 Windows 也能測）：
/// `C:\a\b` → `\\?\C:\a\b`；`\\server\share\x` → `\\?\UNC\server\share\x`；
/// 已經是長路徑形式或相對路徑則不動。長路徑形式不會幫你處理 `.`／`..`，所以這裡先正規化。
fn verbatim_form(raw: &str) -> String {
    let unified = raw.replace('/', "\\");
    if unified.starts_with("\\\\?\\") {
        return unified;
    }
    let (prefix, rest) = if let Some(rest) = unified.strip_prefix("\\\\") {
        ("\\\\?\\UNC\\".to_string(), rest.to_string())
    } else {
        let bytes = unified.as_bytes();
        let is_drive = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\';
        if !is_drive {
            return raw.to_string();
        }
        (format!("\\\\?\\{}\\", &unified[..2]), unified[3..].to_string())
    };
    let mut parts: Vec<&str> = Vec::new();
    for segment in rest.split('\\') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("{prefix}{}", parts.join("\\"))
}

#[cfg(test)]
mod long_path_tests {
    use super::*;
    use std::fs;

    #[test]
    fn verbatim_form_handles_drive_unc_and_dots() {
        assert_eq!(verbatim_form("C:\\Games\\pack"), "\\\\?\\C:\\Games\\pack");
        assert_eq!(verbatim_form("C:/Games/./a/../pack"), "\\\\?\\C:\\Games\\pack");
        assert_eq!(verbatim_form("\\\\nas\\share\\mc"), "\\\\?\\UNC\\nas\\share\\mc");
        assert_eq!(verbatim_form("\\\\?\\C:\\x"), "\\\\?\\C:\\x");
        assert_eq!(verbatim_form("relative\\x"), "relative\\x");
    }

    #[test]
    fn short_paths_are_left_alone() {
        let p = Path::new("C:/short/path");
        assert_eq!(long_path(p), p.to_path_buf());
    }

    #[test]
    fn long_paths_can_be_created_written_and_copied() {
        let mut dir = std::env::temp_dir().join(format!("mcpl-long-{}", std::process::id()));
        let root = dir.clone();
        let _ = fs::remove_dir_all(long_path(&root));
        while dir.to_string_lossy().len() < 300 {
            dir = dir.join("very_long_folder_name_for_mcpl_test");
        }
        let file = dir.join("config.json");
        assert!(file.to_string_lossy().len() > 260);
        fs::create_dir_all(long_path(&dir)).unwrap();
        fs::write(long_path(&file), b"hello").unwrap();
        let copy = dir.join("copy.json");
        fs::copy(long_path(&file), long_path(&copy)).unwrap();
        assert_eq!(fs::read(long_path(&copy)).unwrap(), b"hello");
        let _ = fs::remove_dir_all(long_path(&root));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mcpl-paths-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn prefers_new_root_when_it_already_has_the_file() {
        let root = scratch("new-has-it");
        let new_root = root.join("new");
        let legacy_root = root.join("legacy");
        let new_file = new_root.join("secrets.json");
        let legacy_file = legacy_root.join("secrets.json");
        fs::create_dir_all(&new_root).unwrap();
        fs::write(&new_file, "{}").unwrap();
        fs::create_dir_all(&legacy_root).unwrap();
        fs::write(&legacy_file, "{}").unwrap();
        // 兩邊都有時，新根優先——這是「全新安裝已經開始寫新根」的情境。
        assert_eq!(
            resolve_with_roots(Path::new("secrets.json"), &new_root, &legacy_root),
            new_file
        );
    }

    #[test]
    fn falls_back_to_legacy_when_only_legacy_has_the_file() {
        // 既有安裝（第九輪以前）：資料還留在 %APPDATA%，新根什麼都沒有——
        // 必須沿用舊根，不然使用者升級後金鑰／術語表／翻譯記憶會像消失一樣。
        let root = scratch("legacy-only");
        let new_root = root.join("new");
        let legacy_root = root.join("legacy");
        let legacy_file = legacy_root.join("tm.json");
        fs::create_dir_all(&legacy_root).unwrap();
        fs::write(&legacy_file, "{}").unwrap();
        assert_eq!(
            resolve_with_roots(Path::new("tm.json"), &new_root, &legacy_root),
            legacy_file
        );
    }

    #[test]
    fn defaults_to_new_root_when_neither_exists() {
        // 全新安裝：兩邊都沒有，落在新根（可攜式資料夾），交給呼叫端自己建立。
        let root = scratch("neither");
        let new_root = root.join("new");
        let legacy_root = root.join("legacy");
        let expected = new_root.join("glossary.json");
        assert_eq!(
            resolve_with_roots(Path::new("glossary.json"), &new_root, &legacy_root),
            expected
        );
    }

    #[test]
    fn resolve_with_roots_also_works_for_directories() {
        // work／local-llm 這類子系統解析的是資料夾而不是單一檔案；同一套邏輯要適用。
        let root = scratch("dir-case");
        let new_root = root.join("new");
        let legacy_root = root.join("legacy");
        let legacy_dir = legacy_root.join("work");
        fs::create_dir_all(&legacy_dir).unwrap();
        assert_eq!(
            resolve_with_roots(Path::new("work"), &new_root, &legacy_root),
            legacy_dir
        );
    }

    #[test]
    fn portable_root_is_under_the_executable_directory() {
        let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
        assert_eq!(portable_root(), exe_dir.join(PORTABLE_FOLDER));
    }
}
