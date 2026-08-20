//! 檢查更新（通知＋驗證＋安裝）。
//!
//! 本專案只發佈免安裝 EXE。Windows 更新流程對齊 ZeitFrei-Tool `do_update`：
//!   1. 打 Worker `/api/desktop/latest` 拿最新版本
//!   2. 比版本，較新才提示
//!   3. 下載到 `%TEMP%\MCPL_Update.exe`，強制驗 SHA-256、檔案大小與 PE
//!   4. 優先：執行中 exe 改名 `.bak` → copy 新檔到原路徑 → 隱藏延遲 bat 等 PID 死再 start
//!   5. rename 失敗則 fallback：寫 `.new` + 延遲 bat move+start
//!   6. `app.exit(0)`；新版啟動時清 `.bak`／`.new`，並排 post-update 保底重開
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

    // 固定暫存檔名（與 ZeitFrei_Update.exe 同策略），避免中文檔名／括號干擾。
    let dest = update_temp_exe_path();
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

    let (launched, automatic, should_exit) = apply_portable_update(&dest)?;
    Ok(DownloadResult {
        path: dest.display().to_string(),
        launched,
        automatic,
        should_exit,
        message: if automatic {
            "免安裝更新檔已驗證，工具將關閉、替換並由新版重新開啟。".into()
        } else {
            format!(
                "已驗證並開啟免安裝更新檔：{}\n若工具沒有自動替換，請關閉目前工具後手動開啟新版。",
                dest.display()
            )
        },
    })
}

fn update_temp_exe_path() -> PathBuf {
    std::env::temp_dir().join("MCPL_Update.exe")
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
fn write_bat_file(path: &std::path::Path, content: &str) -> Result<(), String> {
    std::fs::write(path, content.as_bytes()).map_err(|e| format!("建立更新排程失敗：{e}"))
}

/// ZeitFrei-Tool `spawn_hidden_bat`：優先隱藏且能重開，失敗再放寬。
#[cfg(windows)]
fn spawn_hidden_bat(bat: &std::path::Path, breakaway: bool) -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let bat_s = bat.to_string_lossy().replace('"', "");
    if bat_s.is_empty() || !bat.exists() {
        return false;
    }
    let mut hide_flags: u32 = 0x0800_0000 | 0x0000_0200; // NO_WINDOW | NEW_PROCESS_GROUP
    if breakaway {
        hide_flags |= 0x0100_0000; // BREAKAWAY_FROM_JOB
    }
    let mut det_flags: u32 = 0x0000_0008 | 0x0000_0200; // DETACHED | NEW_PROCESS_GROUP
    if breakaway {
        det_flags |= 0x0100_0000;
    }

    // 1) cmd /c bat + CREATE_NO_WINDOW（不要用 DETACHED，也不要先假成功的 VBS）
    if Command::new("cmd")
        .args(["/c", &bat_s])
        .creation_flags(hide_flags)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
    {
        return true;
    }

    // 2) PowerShell 隱藏啟動 cmd /c bat
    let ps = format!(
        "Start-Process -FilePath 'cmd.exe' -ArgumentList @('/c','{}') -WindowStyle Hidden",
        bat_s.replace('\'', "''")
    );
    if Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps])
        .creation_flags(hide_flags)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
    {
        return true;
    }

    // 3) VBS Run 0
    let vbs_path = bat.with_extension("vbs");
    let vbs = format!(
        "CreateObject(\"WScript.Shell\").Run \"cmd /c \"\"{}\"\"\", 0, False\r\n",
        bat_s
    );
    if std::fs::write(&vbs_path, vbs).is_ok() {
        let vbs_s = vbs_path.to_string_lossy().replace('"', "");
        if Command::new("wscript.exe")
            .args(["//B", "//Nologo", &vbs_s])
            .creation_flags(hide_flags)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            return true;
        }
    }

    // 4) DETACHED fallback（可能閃黑框，總比沒重開好）
    Command::new("cmd")
        .args(["/c", &bat_s])
        .creation_flags(det_flags)
        .spawn()
        .is_ok()
}

/// 等本 PID 結束後再 `start` 新 exe（主路徑：檔已替換完成）。
#[cfg(windows)]
fn build_delayed_start_bat(pid: u32, exe: &str) -> String {
    format!(
        "@echo off\r\n:wait\r\ntasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul\r\nif not errorlevel 1 (\r\n  ping -n 2 127.0.0.1 >nul\r\n  goto wait\r\n)\r\nif exist \"{exe}\" start \"\" \"{exe}\"\r\ndel \"%~f0\"\r\n",
        pid = pid,
        exe = exe
    )
}

/// rename 失敗時：等 PID 結束後把 `.new` move 回原檔名再 start。
#[cfg(windows)]
fn build_fallback_replace_bat(pid: u32, new_path: &str, cur_path: &str) -> String {
    format!(
        "@echo off\r\n:wait\r\ntasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul\r\nif not errorlevel 1 (\r\n  ping -n 2 127.0.0.1 >nul\r\n  goto wait\r\n)\r\nmove /Y \"{new}\" \"{cur}\" >nul\r\nif exist \"{cur}\" start \"\" \"{cur}\"\r\ndel \"%~f0\"\r\n",
        pid = pid,
        new = new_path,
        cur = cur_path
    )
}

/// 剛被更新拉起時（旁有 .bak）：約 3 秒後若本程式已死再拉一次（ZeitFrei post-update）。
#[cfg(windows)]
fn schedule_post_update_relaunch(exe: &std::path::Path) {
    let path_s = exe.to_string_lossy().replace('"', "");
    let exe_name = exe
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "MCPL.exe".into());
    let script = format!(
        "@echo off\r\nping -n 4 127.0.0.1 >nul\r\ntasklist /FI \"IMAGENAME eq {name}\" 2>nul | find /I \"{name}\" >nul\r\nif not errorlevel 1 exit /b 0\r\nif exist \"{exe}\" start \"\" \"{exe}\"\r\ndel \"%~f0\"\r\n",
        name = exe_name.replace('"', ""),
        exe = path_s
    );
    let pid = std::process::id();
    let bat = std::env::temp_dir().join(format!("mcpl_post_update_{pid}.bat"));
    let bat_w = std::env::temp_dir().join(format!("mcpl_post_update_{pid}_w.bat"));
    if write_bat_file(&bat, &script).is_err() {
        return;
    }
    let _ = write_bat_file(&bat_w, &script);
    let _ = spawn_hidden_bat(&bat, true)
        || spawn_hidden_bat(&bat, false)
        || spawn_hidden_bat(&bat_w, true)
        || spawn_hidden_bat(&bat_w, false);
}

/// 啟動時清更新殘留；若有 `.bak` 表示剛完成自動更新，排保底重開。
pub fn cleanup_update_residuals() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let bak = exe.with_extension("bak");
    #[cfg(windows)]
    if bak.exists() {
        schedule_post_update_relaunch(&exe);
    }
    let _ = std::fs::remove_file(&bak);
    let _ = std::fs::remove_file(exe.with_extension("new"));
    let _ = std::fs::remove_file(exe.with_extension("update.bat"));
    let _ = std::fs::remove_file(update_temp_exe_path());
}

/// ZeitFrei-Tool `do_update` Windows 主流程：
/// 1) 執行中舊 exe 改名 `.bak`（Windows 允許改名鎖定檔）
/// 2) 新檔 copy 到原路徑
/// 3) 延遲 bat 等本 PID 死再 start（BREAKAWAY 脫 Job）
/// 4) 回傳 should_exit，由 command 端 `app.exit(0)`
/// rename 失敗 → fallback：寫 `.new` + 等 PID 後 move+start
#[cfg(windows)]
fn apply_portable_update(tmp: &std::path::Path) -> Result<(bool, bool, bool), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    let current = std::env::current_exe().map_err(|e| format!("找不到目前工具位置：{e}"))?;
    let bak = current.with_extension("bak");
    let _ = std::fs::remove_file(&bak);

    let renamed = std::fs::rename(&current, &bak);
    if renamed.is_ok() {
        if let Err(e) = std::fs::copy(tmp, &current) {
            let _ = std::fs::rename(&bak, &current);
            return Err(format!(
                "無法寫入新版（多半是防毒攔截）：{e}。請改用更新視窗的「手動下載」。"
            ));
        }
        let _ = std::fs::remove_file(tmp);

        let path_s = current.to_string_lossy().replace('"', "");
        let pid = std::process::id();
        let script = build_delayed_start_bat(pid, &path_s);
        let script_path = current.with_extension("update.bat");
        write_bat_file(&script_path, &script)?;

        let mut launched = spawn_hidden_bat(&script_path, true) || spawn_hidden_bat(&script_path, false);
        if !launched {
            // bat 失敗才 breakaway 直開（備援；父仍活著，仍有 Job 風險）
            const DIRECT_FLAGS: u32 = 0x0800_0000 | 0x0000_0200 | 0x0100_0000;
            let mut cmd = Command::new(&current);
            if let Some(dir) = current.parent() {
                cmd.current_dir(dir);
            }
            cmd.creation_flags(DIRECT_FLAGS);
            launched = cmd.spawn().is_ok();
        }
        if !launched {
            return Err("已更新完成，但自動重啟失敗，請手動開啟程式。".into());
        }
        // 給子行程一點時間；真正 start 仍等 PID 結束（與 ZeitFrei 一致）
        std::thread::sleep(std::time::Duration::from_millis(600));
        return Ok((true, true, true));
    }

    // fallback：複製到 .new，延遲 bat 等 PID 後 move+start
    let next_to = current.with_extension("new");
    let _ = std::fs::remove_file(&next_to);
    std::fs::copy(tmp, &next_to).map_err(|e| format!("無法寫入更新檔：{e}"))?;
    let _ = std::fs::remove_file(tmp);

    let cur_str = current.to_string_lossy().replace('"', "");
    let new_str = next_to.to_string_lossy().replace('"', "");
    let pid = std::process::id();
    let script = build_fallback_replace_bat(pid, &new_str, &cur_str);
    let script_path = current.with_extension("update.bat");
    write_bat_file(&script_path, &script)?;
    if !spawn_hidden_bat(&script_path, true) && !spawn_hidden_bat(&script_path, false) {
        return Err("已下載更新檔，但無法啟動重啟腳本，請關閉程式後手動覆蓋 exe。".into());
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    Ok((true, true, true))
}

#[cfg(not(windows))]
fn apply_portable_update(path: &std::path::Path) -> Result<(bool, bool, bool), String> {
    open::that(path)
        .map(|_| (true, false, false))
        .map_err(|e| format!("更新檔已下載，但無法啟動：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_version_is_detected_numerically() {
        assert!(is_newer("0.5.0", "0.4.0"));
        assert!(is_newer("0.4.1", "0.4.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
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
    fn update_temp_uses_ascii_name() {
        let p = update_temp_exe_path();
        assert!(p.to_string_lossy().ends_with("MCPL_Update.exe"));
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
    fn delayed_start_bat_matches_zeitfrei_shape() {
        let script = build_delayed_start_bat(12345, "D:\\Down\\MCPL-1.0.3 (1).exe");
        assert!(script.contains("PID eq 12345"));
        assert!(script.contains("start \"\" \"D:\\Down\\MCPL-1.0.3 (1).exe\""));
        // if exist 單行（無區塊括號包住路徑），括號檔名安全
        assert!(script.contains("if exist \"D:\\Down\\MCPL-1.0.3 (1).exe\" start"));
    }

    #[cfg(windows)]
    #[test]
    fn fallback_replace_bat_matches_zeitfrei_shape() {
        let script = build_fallback_replace_bat(
            99,
            "D:\\Down\\MCPL-1.0.3 (1).new",
            "D:\\Down\\MCPL-1.0.3 (1).exe",
        );
        assert!(script.contains("move /Y \"D:\\Down\\MCPL-1.0.3 (1).new\" \"D:\\Down\\MCPL-1.0.3 (1).exe\""));
        assert!(script.contains("start \"\" \"D:\\Down\\MCPL-1.0.3 (1).exe\""));
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
