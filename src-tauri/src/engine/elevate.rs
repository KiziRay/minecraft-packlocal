//! 需要系統管理員權限時，提供一鍵解法——但**絕不預設要求**。
//!
//! # 設計原則
//!
//! 1. **不在啟動時要求管理員**。多數整合包放在使用者自己的資料夾底下，
//!    根本不需要提權；一開工具就跳 UAC 只會讓人覺得這程式有問題。
//! 2. **提早發現，不要等三小時後才失敗**。使用者選完資料夾當下就試寫一次，
//!    寫不進去馬上講。等到翻完才在套用階段失敗，那三小時就白費了。
//! 3. **使用者按取消不是錯誤**。他可能只是想改選別的資料夾，
//!    那條路要留著，不能把他逼進死巷。

use std::path::Path;

/// 這個資料夾寫得進去嗎？寫不進去時附上「為什麼」與「可以怎麼辦」。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteAccessReport {
    pub writable: bool,
    /// 需要管理員權限才寫得進去（跟「磁碟滿了」是兩回事，處理方式也不同）
    pub needs_admin: bool,
    pub path: String,
    pub message: String,
}

/// 常見的「這個位置注定要管理員權限」路徑。
///
/// 判斷用路徑而不是只看錯誤碼：Windows 對 `Program Files` 有虛擬化行為，
/// 有時寫入「看起來成功」卻被導到別的地方，等玩家進遊戲才發現沒生效。
fn looks_like_protected_location(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_ascii_lowercase().replace('/', "\\");
    ["\\program files", "\\program files (x86)", "\\windows\\", "\\programdata\\"]
        .iter()
        .any(|needle| lower.contains(needle))
}

/// 在使用者選完資料夾的當下就檢查，而不是等翻完才失敗。
pub fn check_write_access(path: &Path) -> WriteAccessReport {
    let protected = looks_like_protected_location(path);
    match super::disk::probe_apply_targets(path) {
        Ok(()) => WriteAccessReport {
            writable: true,
            needs_admin: false,
            path: path.display().to_string(),
            message: String::new(),
        },
        Err(detail) => {
            let needs_admin = protected || mentions_permission(&detail);
            let message = if needs_admin {
                format!(
                    "這個遊戲資料夾需要系統管理員權限才寫得進去{}。\
你可以用下面的按鈕以管理員身分重新開啟工具，或改選一個放在你自己資料夾底下的整合包。",
                    if protected { "（它在系統保護的位置底下）" } else { "" }
                )
            } else {
                format!("這個資料夾目前寫不進去：{detail}")
            };
            WriteAccessReport {
                writable: false,
                needs_admin,
                path: path.display().to_string(),
                message,
            }
        }
    }
}

fn mentions_permission(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.contains("permission")
        || lower.contains("denied")
        || lower.contains("os error 5")
        || detail.contains("拒絕")
        || detail.contains("權限")
}

/// 以系統管理員身分重新啟動工具，並把目前的資料夾帶回去。
///
/// 使用者在 UAC 按取消時回 `Ok(false)`——**那不是錯誤**，
/// 只是他決定不要提權，畫面應該回到原狀讓他改選別的資料夾。
#[cfg(windows)]
pub fn relaunch_as_admin(carry_instance: &str) -> Result<bool, String> {
    use std::os::windows::ffi::OsStrExt;

    let exe = std::env::current_exe().map_err(|e| format!("找不到工具位置：{e}"))?;
    let mut params = String::new();
    let instance = carry_instance.trim();
    if !instance.is_empty() {
        // 提權後把原本選的資料夾帶回去，不要讓使用者再選一次
        params = format!("--instance \"{}\"", instance.replace('"', ""));
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }
    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(exe.as_os_str());
    let args = wide(std::ffi::OsStr::new(&params));

    // ShellExecuteW 回傳值 > 32 代表成功；ERROR_CANCELLED(1223) 代表使用者按了取消
    const SE_ERR_MIN_SUCCESS: isize = 32;
    const ERROR_CANCELLED: isize = 1223;
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: isize,
            lpOperation: *const u16,
            lpFile: *const u16,
            lpParameters: *const u16,
            lpDirectory: *const u16,
            nShowCmd: i32,
        ) -> isize;
    }
    let result = unsafe {
        ShellExecuteW(
            0,
            verb.as_ptr(),
            file.as_ptr(),
            if params.is_empty() { std::ptr::null() } else { args.as_ptr() },
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    };
    if result > SE_ERR_MIN_SUCCESS {
        return Ok(true);
    }
    if result == ERROR_CANCELLED {
        // 使用者自己選擇不提權：這是正常結果，不是失敗
        return Ok(false);
    }
    Err(format!("無法以管理員身分重新開啟（代碼 {result}）。請自己以系統管理員身分執行工具。"))
}

#[cfg(not(windows))]
pub fn relaunch_as_admin(_carry_instance: &str) -> Result<bool, String> {
    Err("這個功能只在 Windows 上提供。".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_windows_protected_locations() {
        // 這些位置一定要管理員權限，而且 Windows 還會做檔案虛擬化——
        // 「看起來寫成功了但遊戲讀不到」比直接失敗更難查。
        for p in [
            r"C:\Program Files\Minecraft\instances\pack",
            r"C:\Program Files (x86)\launcher\pack",
            r"C:\Windows\Temp\pack",
            r"C:\ProgramData\launcher\pack",
        ] {
            assert!(looks_like_protected_location(Path::new(p)), "{p}");
        }
        // 正斜線寫法也要認得
        assert!(looks_like_protected_location(Path::new("C:/Program Files/x/y")));
    }

    #[test]
    fn ordinary_user_folders_are_not_flagged() {
        for p in [
            r"C:\Users\jolin\AppData\Roaming\PrismLauncher\instances\pack",
            r"D:\Games\CurseForge\Instances\pack",
            r"C:\Users\jolin\Downloads\pack",
        ] {
            assert!(!looks_like_protected_location(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn permission_errors_are_told_apart_from_other_failures() {
        // 這兩種的處理方式完全不同：權限問題可以提權解決，
        // 磁碟滿了提權也沒用——訊息不該把使用者導向錯的方向。
        assert!(mentions_permission("Access is denied. (os error 5)"));
        assert!(mentions_permission("權限不足"));
        assert!(mentions_permission("Permission denied"));
        assert!(!mentions_permission("磁碟空間不足"));
        assert!(!mentions_permission("找不到資料夾"));
    }
}
