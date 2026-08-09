//! 檢查更新（通知＋下載安裝檔）。
//!
//! 刻意**不做自我替換 exe**：靜默改寫執行檔正是防毒判「木馬 dropper」的頭號特徵，
//! 與「降低誤刪」的目標直接衝突。流程比照 ZeitFrei-Tool：
//!   1. 打 Worker `/api/desktop/latest` 拿最新版本
//!   2. 比版本，較新才提示
//!   3. 使用者要更新 → 下載官方 NSIS 安裝檔到暫存、（有提供時）驗 sha256 → 啟動安裝檔
//!   4. 使用者在安裝精靈點一次「下一步」完成，工具本身不碰系統
//!
//! 更新端點與下載連結都非機密，比對邏輯全在本地。

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::secrets::MANAGED_BASE_URL;

/// 目前版本（編譯時由 Cargo 帶入）。
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub current: String,
    pub latest: String,
    /// latest 是否比 current 新
    pub update_available: bool,
    /// 下載頁或安裝檔連結
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

    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
    {
        Ok(c) => c,
        Err(e) => return fail(&format!("無法建立連線：{e}")),
    };

    let resp = match client.get(endpoint()).send() {
        Ok(r) => r,
        Err(_) => return fail("暫時無法檢查更新（可能沒有網路）。"),
    };
    if !resp.status().is_success() {
        return fail(&format!("檢查更新失敗（{}）。", resp.status().as_u16()));
    }
    let latest: LatestResponse = match resp.json() {
        Ok(v) => v,
        Err(_) => return fail("檢查更新回應無法解析。"),
    };
    if latest.version.trim().is_empty() {
        return fail("伺服器未提供版本資訊。");
    }

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
    pub message: String,
}

/// 下載安裝檔到暫存並啟動它（使用者手動點一次完成安裝）。
///
/// 若 `/api/desktop/latest` 給了 sha256 就驗證，不符不啟動——防止半途損毀或被掉包。
pub fn download_and_launch() -> Result<DownloadResult, String> {
    let info = check_update();
    if !info.ok {
        return Err(info.message);
    }
    if !info.update_available {
        return Ok(DownloadResult {
            path: String::new(),
            launched: false,
            message: format!("已是最新版（{}），不需要更新。", info.current),
        });
    }
    if info.url.trim().is_empty() {
        return Err("伺服器沒有提供下載連結。".into());
    }

    // 再抓一次拿 sha256（check_update 的結構沒帶 sha256 出來，這裡直接讀原始回應）。
    let expected_sha = fetch_expected_sha();

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(&info.url)
        .send()
        .map_err(|e| format!("下載失敗：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下載失敗（{}）。", resp.status().as_u16()));
    }
    let bytes = resp.bytes().map_err(|e| format!("下載中斷：{e}"))?;

    if let Some(expected) = expected_sha.as_deref() {
        let got = sha256_hex(&bytes);
        if !got.eq_ignore_ascii_case(expected) {
            return Err(format!(
                "安裝檔校驗不符，已中止（預期 {expected}，實得 {got}）。請改用官方下載頁。"
            ));
        }
    }

    let dest = download_target(&info.url, &info.latest);
    std::fs::write(&dest, &bytes).map_err(|e| format!("寫入暫存失敗：{e}"))?;

    // 啟動安裝檔（ShellExecute）；不隱藏視窗、不自動關閉本程式，交給使用者。
    let launched = open::that(&dest).is_ok();
    Ok(DownloadResult {
        path: dest.display().to_string(),
        launched,
        message: if launched {
            "已下載安裝檔並開啟，請依畫面完成安裝。".into()
        } else {
            format!("已下載到：{}\n請手動開啟完成安裝。", dest.display())
        },
    })
}

fn fetch_expected_sha() -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .ok()?;
    let resp = client.get(endpoint()).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let latest: LatestResponse = resp.json().ok()?;
    latest.sha256.filter(|s| !s.trim().is_empty())
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

/// 純 Rust SHA-256（避免多拉一個 crate）。
fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let bitlen = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());

    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, wi) in w.iter_mut().take(16).enumerate() {
            *wi = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ ((!v[4]) & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v[7] = v[6];
            v[6] = v[5];
            v[5] = v[4];
            v[4] = v[3].wrapping_add(t1);
            v[3] = v[2];
            v[2] = v[1];
            v[1] = v[0];
            v[0] = t1.wrapping_add(t2);
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
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
}
