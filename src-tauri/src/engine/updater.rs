//! 檢查更新（通知＋驗證＋安裝）。
//!
//! 本專案只發佈免安裝 EXE，不直接改寫正在執行的 exe。流程採用 ZeitFrei-Tool 的可靠性原則：
//!   1. 打 Worker `/api/desktop/latest` 拿最新版本
//!   2. 比版本，較新才提示
//!   3. 防連點，下載官方免安裝 EXE 到暫存，強制驗 SHA-256、檔案大小與 PE 標頭
//!   4. Windows 由脫離父行程的背景工作等待舊程式結束，再替換同一路徑並重新開啟
//!
//! 更新端點與下載連結都非機密，比對邏輯全在本地。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::hashutil::sha256_hex;
use super::secrets::MANAGED_BASE_URL;

/// 目前版本（編譯時由 Cargo 帶入）。
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const MIN_UPDATE_BYTES: usize = 100_000;
const MAX_UPDATE_BYTES: usize = 256 * 1024 * 1024;
// 若更新流程掛住，避免鎖永遠不回收；前端也有 download_invoke timeout。
// stale 門檻需要明顯大於前端 timeout，降低「正常慢」誤觸的機率。
const UPDATE_STALE_MS: u64 = 20 * 60 * 1000; // 20 分鐘
static UPDATE_LOCK_TOKEN: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct UpdateGuard {
    token: u64,
}

impl UpdateGuard {
    fn acquire() -> Result<Self, String> {
        let now = now_ms();
        let cur = UPDATE_LOCK_TOKEN.load(Ordering::SeqCst);

        // 1) 空閒：嘗試從 0 取得鎖
        if cur == 0 {
            if UPDATE_LOCK_TOKEN
                .compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Ok(Self { token: now });
            }
        }

        // 2) 非空：若鎖超過 stale 門檻，允許接手（CAS token，避免舊 guard 釋放新鎖）
        if cur != 0 && now.saturating_sub(cur) > UPDATE_STALE_MS {
            if UPDATE_LOCK_TOKEN
                .compare_exchange(cur, now, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Ok(Self { token: now });
            }
        }

        Err("更新已在進行中，請稍候，不要重複點擊。".to_string())
    }
}

impl Drop for UpdateGuard {
    fn drop(&mut self) {
        // 只有 token 沒變時才回收鎖；避免舊 guard 釋放掉新 guard 的鎖。
        let _ = UPDATE_LOCK_TOKEN.compare_exchange(
            self.token,
            0,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub current: String,
    pub latest: String,
    /// latest 是否比 current 新
    pub update_available: bool,
    /// 官方免安裝 EXE 下載連結
    pub url: String,
    /// 更新說明（可空）
    pub notes: String,
    /// 檢查本身是否成功（false＝連不上，UI 顯示「暫時無法檢查」）
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct LatestResponse {
    #[serde(default)]
    version: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    sha256: Option<String>,
}

/// 把 `1.2.3` 這種版本轉成可比較的數字序列；非數字尾綴（`-beta`）安全忽略。
fn version_tuple(v: &str) -> Vec<u32> {
    v.trim()
        .trim_start_matches(['v', 'V'])
        .split('.')
        .map(|chunk| {
            chunk
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}

/// latest 是否嚴格新於 current。
pub fn is_newer(latest: &str, current: &str) -> bool {
    let a = version_tuple(latest);
    let b = version_tuple(current);
    let n = a.len().max(b.len());
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

fn endpoint() -> String {
    format!("{}/api/desktop/latest", MANAGED_BASE_URL.trim_end_matches('/'))
}

fn fetch_latest() -> Result<LatestResponse, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|e| format!("無法建立更新連線：{e}"))?;
    let response = client
        .get(endpoint())
        .send()
        .map_err(|_| "暫時無法檢查更新（可能沒有網路）。".to_string())?;
    if !response.status().is_success() {
        return Err(format!("檢查更新失敗（{}）。", response.status().as_u16()));
    }
    let latest: LatestResponse = response
        .json()
        .map_err(|_| "檢查更新回應無法解析。".to_string())?;
    if latest.version.trim().is_empty() {
        return Err("伺服器未提供版本資訊。".into());
    }
    Ok(latest)
}

/// 檢查更新。連不上時回 `ok=false`，不當成錯誤（純資訊查詢）。
pub fn check_update() -> UpdateCheck {
    let current = CURRENT_VERSION.to_string();
    let fail = |msg: &str| UpdateCheck {
        current: current.clone(),
        latest: current.clone(),
        update_available: false,
        url: String::new(),
        notes: String::new(),
        ok: false,
        message: msg.to_string(),
    };

    let latest = match fetch_latest() {
        Ok(value) => value,
        Err(error) => return fail(&error),
    };

    let available = is_newer(&latest.version, &current);
    UpdateCheck {
        current: current.clone(),
        latest: latest.version.clone(),
        update_available: available,
        url: latest.url,
        notes: latest.notes,
        ok: true,
        message: if available {
            format!("有新版本 {}（目前 {}）", latest.version, current)
        } else {
            format!("已是最新版（{current}）")
        },
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadResult {
    pub path: String,
    pub launched: bool,
    /// true＝已排程替換免安裝 EXE 並要求完成後重開。
    pub automatic: bool,
    /// true＝背景更新工作已脫離目前行程，前端收到回應後可關閉舊程式。
    pub should_exit: bool,
    pub message: String,
}

/// 下載免安裝 EXE 到暫存，強制驗證後排程替換更新。
pub fn download_and_launch() -> Result<DownloadResult, String> {
    let _guard = UpdateGuard::acquire()?;
    let latest = fetch_latest()?;
    let current = CURRENT_VERSION.to_string();
    if !is_newer(&latest.version, &current) {
        return Ok(DownloadResult {
            path: String::new(),
            launched: false,
            automatic: false,
            should_exit: false,
            message: format!("已是最新版（{current}），不需要更新。"),
        });
    }
    let url = validate_download_url(&latest.url)?;
    let expected_sha = validate_sha256(latest.sha256.as_deref())?;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("下載失敗：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下載失敗（{}）。", resp.status().as_u16()));
    }
    if resp
        .content_length()
        .is_some_and(|size| size < MIN_UPDATE_BYTES as u64 || size > MAX_UPDATE_BYTES as u64)
    {
        return Err("伺服器提供的更新 EXE 大小不合理，已停止更新。".into());
    }
    let bytes = resp.bytes().map_err(|e| format!("下載中斷：{e}"))?;
    validate_update_bytes(&bytes)?;
    let got = sha256_hex(&bytes);
    if !got.eq_ignore_ascii_case(&expected_sha) {
        return Err("更新 EXE 完整性驗證失敗，已停止更新。請改用官方下載連結。".into());
    }

    let dest = download_target(&url, &latest.version);
    let partial = dest.with_extension("download");
    let _ = std::fs::remove_file(&partial);
    std::fs::write(&partial, &bytes).map_err(|e| format!("寫入更新暫存檔失敗：{e}"))?;
    // 雙雜湊：落盤後再讀一次比對 SHA-256（防寫入中途損壞）
    let on_disk = std::fs::read(&partial).map_err(|e| format!("讀取更新暫存檔失敗：{e}"))?;
    if on_disk.len() != bytes.len() || !sha256_hex(&on_disk).eq_ignore_ascii_case(&expected_sha) {
        let _ = std::fs::remove_file(&partial);
        return Err("更新 EXE 落盤後完整性複驗失敗，已停止更新。".into());
    }
    let _ = std::fs::remove_file(&dest);
    std::fs::rename(&partial, &dest).map_err(|e| format!("準備更新 EXE 失敗：{e}"))?;

    let (launched, automatic, should_exit) = launch_portable_update(&dest)?;
    let log_hint = std::env::temp_dir().join(format!(
        "modpack_i18n_update_{}.log",
        std::process::id()
    ));
    Ok(DownloadResult {
        path: dest.display().to_string(),
        launched,
        automatic,
        should_exit,
        message: if automatic {
            format!(
                "免安裝更新檔已驗證，工具將關閉、替換並由新版重新開啟。\n若未自動重開，請查看日誌：{}",
                log_hint.display()
            )
        } else {
            format!(
                "已驗證並開啟免安裝更新檔：{}\n若工具沒有自動替換，請關閉目前工具後手動開啟新版。\n更新日誌：{}",
                dest.display(),
                log_hint.display()
            )
        },
    })
}

fn validate_download_url(url: &str) -> Result<String, String> {
    let value = url.trim();
    let prefix = format!("{}/download/", MANAGED_BASE_URL.trim_end_matches('/'));
    let lower = value.to_ascii_lowercase();
    let name = value.get(prefix.len()..).unwrap_or("");
    if !value.starts_with(&prefix)
        || !is_mcpl_update_filename(name)
        || lower.contains("portable")
        || lower.contains("..")
        || lower.contains("%2e")
        || value.contains('\\')
        || value.contains('?')
        || value.contains('#')
        || name.contains('/')
    {
        return Err("更新下載連結不是官方 MCPL 免安裝 EXE，已停止更新。".into());
    }
    Ok(value.to_string())
}

fn is_mcpl_update_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if !lower.starts_with("mcpl-") || !lower.ends_with(".exe") || lower.contains("portable") {
        return false;
    }
    let Some(ver) = lower
        .strip_prefix("mcpl-")
        .and_then(|s| s.strip_suffix(".exe"))
    else {
        return false;
    };
    let mut parts = ver.split('.');
    let Some(major) = parts.next() else {
        return false;
    };
    let Some(minor) = parts.next() else {
        return false;
    };
    let Some(patch) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && !major.is_empty()
        && major.bytes().all(|b| b.is_ascii_digit())
        && !minor.is_empty()
        && minor.bytes().all(|b| b.is_ascii_digit())
        && patch.chars().next().is_some_and(|c| c.is_ascii_digit())
}

fn validate_sha256(value: Option<&str>) -> Result<String, String> {
    let checksum = value.unwrap_or("").trim();
    if checksum.len() != 64 || !checksum.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("伺服器未提供有效的更新 EXE SHA-256，已停止更新。".into());
    }
    Ok(checksum.to_ascii_lowercase())
}

fn validate_update_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < MIN_UPDATE_BYTES || bytes.len() > MAX_UPDATE_BYTES {
        return Err("下載的更新 EXE 大小不合理，已停止更新。".into());
    }
    if !bytes.starts_with(b"MZ") {
        return Err("下載內容不是有效的 Windows EXE，已停止更新。".into());
    }
    Ok(())
}

#[cfg(windows)]
fn write_utf16_le_bat(path: &std::path::Path, content: &str) -> Result<(), String> {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in content.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(path, bytes).map_err(|e| format!("建立更新排程失敗：{e}"))
}

#[cfg(windows)]
fn build_portable_update_bat_script(
    pid: u32,
    current: &str,
    current_dir: &str,
    replacement: &str,
    backup: &str,
    log: &str,
) -> String {
    format!(
        "@echo off\r\nset \"LOG={log}\"\r\necho [%date% %time%] update worker started pid={pid}>>\"%LOG%\"\r\n:wait\r\ntasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul\r\nif not errorlevel 1 (\r\n  ping -n 2 127.0.0.1 >nul\r\n  goto wait\r\n)\r\necho [%date% %time%] parent exited>>\"%LOG%\"\r\nif exist \"{backup}\" del /f /q \"{backup}\" >>\"%LOG%\" 2>&1\r\nset /a MOVE_TRIES=0\r\n:move_old\r\nset /a MOVE_TRIES+=1\r\nmove /Y \"{current}\" \"{backup}\" >>\"%LOG%\" 2>&1\r\nif errorlevel 1 (\r\n  if %MOVE_TRIES% lss 30 (\r\n    ping -n 2 127.0.0.1 >nul\r\n    goto move_old\r\n  )\r\n  echo [%date% %time%] move old failed>>\"%LOG%\"\r\n  goto cleanup\r\n)\r\nset /a MOVE_TRIES=0\r\n:move_new\r\nset /a MOVE_TRIES+=1\r\nmove /Y \"{replacement}\" \"{current}\" >>\"%LOG%\" 2>&1\r\nif errorlevel 1 (\r\n  if %MOVE_TRIES% lss 30 (\r\n    ping -n 2 127.0.0.1 >nul\r\n    goto move_new\r\n  )\r\n  echo [%date% %time%] move new failed, restoring backup>>\"%LOG%\"\r\n  move /Y \"{backup}\" \"{current}\" >>\"%LOG%\" 2>&1\r\n  goto cleanup\r\n)\r\nif not exist \"{current}\" (\r\n  echo [%date% %time%] target missing, restoring backup>>\"%LOG%\"\r\n  move /Y \"{backup}\" \"{current}\" >>\"%LOG%\" 2>&1\r\n)\r\nif exist \"{current}\" (\r\n  echo [%date% %time%] launching new exe>>\"%LOG%\"\r\n  cd /d \"{current_dir}\"\r\n  start \"\" \"{current}\"\r\n  ping -n 2 127.0.0.1 >nul\r\n  del /f /q \"{backup}\" >>\"%LOG%\" 2>&1\r\n)\r\n:cleanup\r\ndel \"%~f0\"\r\n",
        pid = pid,
        current = current,
        current_dir = current_dir,
        replacement = replacement,
        backup = backup,
        log = log,
    )
}

#[cfg(windows)]
fn launch_portable_update(path: &std::path::Path) -> Result<(bool, bool, bool), String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let current = std::env::current_exe().map_err(|e| format!("找不到目前工具位置：{e}"))?;
    let replacement = current.with_extension("new");
    let backup = current.with_extension("bak");
    std::fs::copy(path, &replacement).map_err(|e| format!("準備免安裝更新檔失敗：{e}"))?;

    // 參考 ZeitFrei 工具箱：由脫離 Tauri Job 的隱藏 bat 等待目前 PID 結束，
    // 再以同一路徑替換 exe 並重開。UTF-16 bat + move 重試 + 日誌，避免鎖檔靜默失敗。
    let pid = std::process::id();
    let current_s = current.to_string_lossy().replace('"', "");
    let current_dir_s = current
        .parent()
        .map(|p| p.to_string_lossy().replace('"', ""))
        .unwrap_or_default();
    let replacement_s = replacement.to_string_lossy().replace('"', "");
    let backup_s = backup.to_string_lossy().replace('"', "");
    let log_path = std::env::temp_dir().join(format!("modpack_i18n_update_{pid}.log"));
    let log_s = log_path.to_string_lossy().replace('"', "");
    let script = build_portable_update_bat_script(
        pid,
        &current_s,
        &current_dir_s,
        &replacement_s,
        &backup_s,
        &log_s,
    );
    let script_path = std::env::temp_dir().join(format!("modpack_i18n_update_{pid}.bat"));
    write_utf16_le_bat(&script_path, &script)?;
    const FLAGS: u32 = 0x0800_0000 | 0x0000_0008 | 0x0000_0200 | 0x0100_0000;
    let launched = Command::new("cmd.exe")
        .arg("/c")
        .arg(&script_path)
        .creation_flags(FLAGS)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok();
    if launched {
        return Ok((true, true, true));
    }

    let _ = std::fs::remove_file(&replacement);
    open::that(path)
        .map(|_| (true, false, false))
        .map_err(|e| format!("更新檔已下載，但無法啟動：{e}"))
}

#[cfg(not(windows))]
fn launch_portable_update(path: &std::path::Path) -> Result<(bool, bool, bool), String> {
    open::that(path)
        .map(|_| (true, false, false))
        .map_err(|e| format!("更新檔已下載，但無法啟動：{e}"))
}

/// 暫存檔名：保留原副檔名（通常 .exe），避免被當成未知格式。
fn download_target(url: &str, version: &str) -> PathBuf {
    let ext = url
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.').map(|(_, e)| e))
        .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("exe");
    std::env::temp_dir().join(format!("模組包翻譯工具_更新_{version}.{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_version_is_detected_numerically() {
        assert!(is_newer("0.5.0", "0.4.0"));
        assert!(is_newer("0.4.1", "0.4.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        // 字串比較會誤判 0.10 < 0.9；數字比較不會
        assert!(is_newer("0.10.0", "0.9.0"));
    }

    #[test]
    fn same_or_older_is_not_an_update() {
        assert!(!is_newer("0.4.0", "0.4.0"));
        assert!(!is_newer("0.3.9", "0.4.0"));
        assert!(!is_newer("0.4.0", "0.4.1"));
    }

    #[test]
    fn tolerates_v_prefix_and_suffix() {
        assert!(is_newer("v0.5.0", "0.4.0"));
        assert!(is_newer("0.5.0-beta", "0.4.0"));
        assert!(!is_newer("v0.4.0", "0.4.0"));
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn download_target_keeps_exe_extension() {
        let p = download_target("https://x/modpack.exe", "0.5.0");
        assert!(p.to_string_lossy().ends_with(".exe"));
        // 沒有副檔名時退回 exe
        let p2 = download_target("https://x/download", "0.5.0");
        assert!(p2.to_string_lossy().ends_with(".exe"));
    }

    #[test]
    fn updater_accepts_only_official_exe_downloads() {
        assert!(validate_download_url(
            "https://modpack-i18n.jolin34563.workers.dev/download/MCPL-1.0.0.exe"
        )
        .is_ok());
        assert!(validate_download_url(
            "https://modpack-i18n.jolin34563.workers.dev/download/minecraftpacklocal-1.0.1-portable.exe"
        )
        .is_err());
        assert!(validate_download_url("https://example.com/fake.exe").is_err());
        assert!(validate_download_url(
            "https://modpack-i18n.jolin34563.workers.dev/download/minecraftpacklocal-1.0.1-setup.exe"
        )
        .is_err());
        assert!(validate_download_url(
            "https://modpack-i18n.jolin34563.workers.dev/download/../fake.exe"
        )
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn portable_update_bat_includes_cd_start_and_log() {
        let script = build_portable_update_bat_script(
            12345,
            "C:\\Tools\\MCPL-1.0.3.exe",
            "C:\\Tools",
            "C:\\Tools\\MCPL-1.0.3.new",
            "C:\\Tools\\MCPL-1.0.3.bak",
            "C:\\Temp\\modpack_i18n_update_12345.log",
        );
        assert!(script.contains("cd /d \"C:\\Tools\""));
        assert!(script.contains("start \"\" \"C:\\Tools\\MCPL-1.0.3.exe\""));
        assert!(script.contains("modpack_i18n_update_12345.log"));
    }

    #[test]
    fn updater_requires_a_valid_checksum_and_pe_file() {
        assert!(validate_sha256(Some(&"a".repeat(64))).is_ok());
        assert!(validate_sha256(None).is_err());
        assert!(validate_sha256(Some("xyz")).is_err());

        let mut update = vec![0u8; MIN_UPDATE_BYTES];
        update[0..2].copy_from_slice(b"MZ");
        assert!(validate_update_bytes(&update).is_ok());
        assert!(validate_update_bytes(b"not an executable").is_err());
    }
}
