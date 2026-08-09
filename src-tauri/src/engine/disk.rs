//! 磁碟可用空間檢查。
//!
//! 空間快滿時硬跑翻譯，會在寫 zip／覆寫檔的半途失敗，留下半成品又難排查。
//! 所以在開工前先擋：可用空間低於門檻就明確中止，叫使用者清一清再來。
//!
//! Windows 用 `GetDiskFreeSpaceExW`（kernel32，Rust 預設就連結，不必額外相依）。
//! 查不到空間時**不阻擋**——寧可放行也不要因為偵測失敗擋住正常使用者。

use std::path::Path;

/// 開工前要求的最低可用空間（結果多為數 MB，但書本／datapack 可能較大，留足餘裕）。
pub const MIN_FREE_BYTES: u64 = 200 * 1024 * 1024; // 200 MB

/// 回傳該路徑所在磁碟的可用位元組；查不到回 `None`。
pub fn free_space(path: &Path) -> Option<u64> {
    free_space_impl(path)
}

/// 開工前檢查。空間夠或查不到 → `Ok(())`；明確不足 → `Err(可讀訊息)`。
pub fn ensure_space(path: &Path, need: u64) -> Result<(), String> {
    match free_space(path) {
        Some(free) if free < need => Err(format!(
            "磁碟空間不足，無法開始翻譯。\n\
目標磁碟可用空間約 {}，建議至少留 {} 再試。\n\
請清出一些空間，或把「翻譯結果放哪」改到空間較多的磁碟。",
            human(free),
            human(need)
        )),
        _ => Ok(()),
    }
}

fn human(bytes: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else {
        format!("{} MB", bytes / MB)
    }
}

#[cfg(windows)]
fn free_space_impl(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;

    // GetDiskFreeSpaceExW 接受檔案或目錄路徑；取一個一定存在的祖先來問。
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        match probe.parent() {
            Some(p) if p != probe => probe = p.to_path_buf(),
            _ => break,
        }
    }
    let wide: Vec<u16> = probe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // kernel32：Rust 於 Windows 預設連結，直接宣告即可。
    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }

    let mut avail: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut avail,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok != 0 {
        Some(avail)
    } else {
        None
    }
}

#[cfg(not(windows))]
fn free_space_impl(_path: &Path) -> Option<u64> {
    // 非 Windows 目前不做偵測（本工具目標平台為 Windows）；回 None＝不阻擋。
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_readable_units() {
        assert_eq!(human(200 * 1024 * 1024), "200 MB");
        assert!(human(3 * 1024 * 1024 * 1024).ends_with("GB"));
    }

    #[test]
    fn ensure_space_passes_when_detection_unavailable() {
        // 不存在的磁碟機代號在 Windows 會查不到 → None → 放行（不因偵測失敗擋人）
        assert!(ensure_space(Path::new("Q:/definitely/not/here"), MIN_FREE_BYTES).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn current_dir_reports_some_space() {
        // 目前工作目錄一定存在，應查得到一個 > 0 的可用空間
        let cwd = std::env::current_dir().unwrap();
        let free = free_space(&cwd);
        assert!(free.is_some());
        assert!(free.unwrap() > 0);
    }

    #[cfg(windows)]
    #[test]
    fn huge_requirement_is_rejected() {
        let cwd = std::env::current_dir().unwrap();
        // 要求 1 PB，一定不足 → 必須擋下
        let petabyte = 1024u64 * 1024 * 1024 * 1024 * 1024;
        assert!(ensure_space(&cwd, petabyte).is_err());
    }
}
