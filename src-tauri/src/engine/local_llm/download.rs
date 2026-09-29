use super::select::FileSpec;
use crate::engine::hashutil::Sha256Hasher;
use crate::engine::secrets::managed_base_url;
use crate::engine::turnstile::MANAGED_AI_PROTOCOL;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;

const CHUNK: usize = 1024 * 1024;
/// 每個 Range 請求抓多大。
///
/// 曾經是 8 MiB：7.38 GB 的模型要切成 **880 個連續 HTTP 請求**，每個都付一次往返延遲。
/// 對同一個 Worker 端點實測（單連線吞吐約 21 MB/s）：
///   單一 24 MiB 請求 1.11 s ／ 3 × 8 MiB 連續 2.51 s ／ 4 條並行各 6 MiB 2.08 s
/// 也就是切太碎大約付了 2.3 倍的代價，而**並行沒有用**（單連線已把線路吃滿）。
///
/// 選 32 MiB 而不是更大：reqwest 的 blocking client 只有「整段請求」逾時、沒有閒置逾時，
/// 所以 chunk 越大，`CHUNK_TIMEOUT_SECS` 就得跟著放大，連帶讓「連線卡死」要更久才被發現。
/// 32 MiB＝231 個請求（原本 880），已經吃掉約四分之三的往返開銷；再往上加只再省十幾秒，
/// 卻要把停滯偵測時間拉長一倍，不划算。
pub(crate) const RANGE_CHUNK: u64 = 32 * 1024 * 1024;
/// 單一 Range 請求的整段逾時。
///
/// 必須大於「chunk 在可接受的最慢速率下」所需時間，否則正常下載會被誤砍成無限重試：
/// 32 MiB ÷ 180 s ≈ 182 KB/s。比這更慢的線路要下載幾 GB 本來就不可行。
const CHUNK_TIMEOUT_SECS: u64 = 180;

pub(crate) fn format_file_send_error() -> String {
    "無法連上模型下載服務。請檢查網路後再試；若仍失敗，請用頁尾「回報」告訴我們。".into()
}

pub(crate) fn format_file_http_error(code: u16) -> String {
    match code {
        401 => "翻譯前請先登入 Discord 並加入官方伺服器。".into(),
        // 403 是「登入了但這個檔案不在允許清單」，跟沒登入是兩回事。
        // 舊版兩者共用同一句，玩家會一直重登入卻永遠好不了。
        403 => "已登入，但這個檔案目前不開放下載。請用頁尾「回報」告訴我們，或先改用自訂 API／GPT。".into(),
        400 | 404 => "雲端還沒提供這個模型檔。請稍後再試；若持續發生，請用頁尾「回報」告訴我們。".into(),
        // 503＝雲端的本地模型儲存區還沒開通，重試一百次也不會好，要給替代路徑。
        503 => "本地模型服務暫時未開放。請稍後再試，或先改用自訂 API／GPT 翻譯。".into(),
        _ => format!(
            "下載失敗（HTTP {code}）。請稍後再試；若仍失敗，請用頁尾「回報」告訴我們。"
        ),
    }
}

pub(crate) fn format_checksum_error() -> String {
    "下載的檔案驗證失敗，已刪除不完整檔。請重新下載。".into()
}

/// Inclusive byte spans covering `total` with fixed-size Range windows.
pub(crate) fn range_spans(total: u64, chunk: u64) -> Vec<(u64, u64)> {
    if total == 0 || chunk == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut off = 0u64;
    while off < total {
        let end = off.saturating_add(chunk).saturating_sub(1).min(total.saturating_sub(1));
        out.push((off, end));
        off = end.saturating_add(1);
    }
    out
}

pub(crate) fn range_header(start: u64, end: u64) -> String {
    format!("bytes={start}-{end}")
}

fn remaining_spans(offset: u64, total: u64, chunk: u64) -> Vec<(u64, u64)> {
    range_spans(total, chunk)
        .into_iter()
        .filter(|(_, end)| *end >= offset)
        .map(|(start, end)| (start.max(offset), end))
        .collect()
}

fn encode_object_name(name: &str) -> String {
    name.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

pub fn object_url(name: &str) -> String {
    format!(
        "{}/api/local-llm/file/{}",
        managed_base_url().trim_end_matches('/'),
        encode_object_name(name)
    )
}

pub fn manifest_url() -> String {
    format!(
        "{}/api/local-llm/manifest",
        managed_base_url().trim_end_matches('/')
    )
}

fn discord_headers() -> Result<Vec<(String, String)>, String> {
    let session = crate::engine::discord_auth::managed_ai_session_cookie()?;
    Ok(vec![
        (
            "X-Zeitfrei-AI-Protocol".into(),
            MANAGED_AI_PROTOCOL.to_string(),
        ),
        ("X-Zeitfrei-Session".into(), session),
    ])
}

pub fn fetch_manifest_text() -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.get(manifest_url());
    for (k, v) in discord_headers()? {
        req = req.header(k, v);
    }
    let resp = req
        .send()
        .map_err(|_| "無法連上模型下載服務。請檢查網路後再試；若仍失敗，請用頁尾「回報」告訴我們。".to_string())?;
    let code = resp.status().as_u16();
    if code == 401 || code == 403 {
        return Err("翻譯前請先登入 Discord 並加入官方伺服器。".into());
    }
    if code == 404 {
        return Err(format_file_http_error(404));
    }
    if !resp.status().is_success() {
        return Err(format!("本地模型清單下載失敗（HTTP {code}）。請稍後再試。"));
    }
    resp.text()
        .map_err(|_| "無法讀取本地模型清單。請稍後再試。".to_string())
}

pub fn file_sha256_hex(path: &Path) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256Hasher::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize_hex())
}

pub fn skip_if_sha_matches(dest: &Path, spec: &FileSpec) -> Result<bool, String> {
    if !dest.is_file() {
        return Ok(false);
    }
    let meta = fs::metadata(dest).map_err(|e| e.to_string())?;
    if spec.bytes > 0 && meta.len() != spec.bytes {
        return Ok(false);
    }
    if spec.sha256.len() != 64 {
        return Ok(false);
    }
    Ok(file_sha256_hex(dest)?.eq_ignore_ascii_case(&spec.sha256))
}

fn open_dest_at(dest: &Path, offset: u64) -> Result<File, String> {
    if offset == 0 {
        return File::create(dest).map_err(|e| e.to_string());
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(dest)
        .map_err(|e| e.to_string())?;
    file.set_len(offset).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    Ok(file)
}

pub fn download_file_resumable(
    spec: &FileSpec,
    dest: &Path,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    if skip_if_sha_matches(dest, spec)? {
        on_progress(spec.bytes, spec.bytes.max(1));
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(CHUNK_TIMEOUT_SECS))
        .build()
        .map_err(|_| format_file_send_error())?;
    let mut offset = 0u64;
    if dest.is_file() {
        offset = fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
        if spec.bytes > 0 && offset >= spec.bytes {
            offset = 0;
            let _ = fs::remove_file(dest);
        }
    }
    let headers = discord_headers()?;
    let total_hint = spec.bytes;
    let planned: Vec<(u64, u64)> = if total_hint > 0 {
        remaining_spans(offset, total_hint, RANGE_CHUNK)
    } else {
        Vec::new()
    };
    let mut plan_i = 0usize;
    let mut file = open_dest_at(dest, offset)?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        crate::engine::cancel::check()?;
        if total_hint > 0 && offset >= total_hint {
            break;
        }
        let end = if total_hint > 0 {
            let Some((_, e)) = planned.get(plan_i).copied() else {
                break;
            };
            e
        } else {
            offset.saturating_add(RANGE_CHUNK).saturating_sub(1)
        };
        if end < offset {
            break;
        }
        let mut req = client.get(object_url(&spec.name));
        for (k, v) in &headers {
            req = req.header(k, v);
        }
        req = req.header("Range", range_header(offset, end));
        let mut resp = req.send().map_err(|_| format_file_send_error())?;
        let code = resp.status().as_u16();
        if code != 200 && code != 206 {
            return Err(format_file_http_error(code));
        }
        if code == 200 {
            if offset != 0 {
                drop(file);
                offset = 0;
                file = open_dest_at(dest, 0)?;
            }
        }
        let expect = end.saturating_sub(offset).saturating_add(1);
        let mut got = 0u64;
        loop {
            crate::engine::cancel::check()?;
            let n = resp.read(&mut buf).map_err(|_| format_file_send_error())?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            got += n as u64;
            offset += n as u64;
            on_progress(offset, total_hint.max(offset).max(1));
        }
        file.flush().map_err(|e| e.to_string())?;
        if code == 200 {
            break;
        }
        if got == 0 {
            break;
        }
        if total_hint == 0 && got < expect {
            break;
        }
        if total_hint > 0 && offset > end {
            plan_i += 1;
        }
    }
    drop(file);
    if spec.sha256.len() == 64 {
        let got = file_sha256_hex(dest)?;
        if !got.eq_ignore_ascii_case(&spec.sha256) {
            let _ = fs::remove_file(dest);
            return Err(format_checksum_error());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn skip_when_sha_and_size_match() {
        let dir = std::env::temp_dir().join(format!("mcpl-llm-skip-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("a.bin");
        let data = b"hello-local-llm";
        File::create(&path).unwrap().write_all(data).unwrap();
        let sha = file_sha256_hex(&path).unwrap();
        let spec = FileSpec {
            name: "a.bin".into(),
            sha256: sha,
            bytes: data.len() as u64,
        };
        assert!(skip_if_sha_matches(&path, &spec).unwrap());
        let _ = fs::remove_dir_all(&dir);
    }

    fn assert_player_safe(msg: &str) {
        let lower = msg.to_ascii_lowercase();
        assert!(!lower.contains(".gguf"), "{msg}");
        assert!(!lower.contains(".zip"), "{msg}");
        assert!(!lower.contains("http://") && !lower.contains("https://"), "{msg}");
        assert!(!lower.contains("reqwest"), "{msg}");
        assert!(!lower.contains("runtime zip"), "{msg}");
        assert!(!msg.contains("error sending request"), "{msg}");
    }

    #[test]
    fn send_error_is_human_and_hides_url() {
        let msg = format_file_send_error();
        assert!(msg.contains("無法連上模型下載服務"));
        assert!(msg.contains("回報"));
        assert_player_safe(&msg);
    }

    #[test]
    fn http_401_asks_discord() {
        let msg = format_file_http_error(401);
        assert!(msg.contains("Discord"));
        assert_player_safe(&msg);
    }

    #[test]
    fn http_403_is_not_confused_with_login() {
        let msg = format_file_http_error(403);
        assert_ne!(msg, format_file_http_error(401));
        assert!(!msg.contains("請先登入"));
        assert!(msg.contains("回報"));
        assert_player_safe(&msg);
    }

    #[test]
    fn http_503_offers_an_alternative() {
        let msg = format_file_http_error(503);
        assert!(!msg.contains("503"));
        assert!(msg.contains("自訂 API") || msg.contains("GPT"));
        assert_player_safe(&msg);
    }

    #[test]
    fn http_404_does_not_name_object() {
        let msg = format_file_http_error(404);
        assert!(msg.contains("雲端還沒提供"));
        assert!(!msg.contains("model-q4"));
        assert_player_safe(&msg);
    }

    #[test]
    fn http_400_matches_missing_file_copy() {
        let msg = format_file_http_error(400);
        assert_eq!(msg, format_file_http_error(404));
        assert_player_safe(&msg);
    }

    #[test]
    fn object_url_encodes_unsafe_bytes_only() {
        let url = object_url("model-q4.gguf");
        assert!(url.ends_with("/api/local-llm/file/model-q4.gguf"));
        let spaced = object_url("model q4.gguf");
        assert!(spaced.contains("model%20q4.gguf"));
        assert!(!spaced.contains("model q4"));
    }

    #[test]
    fn checksum_error_hides_filename() {
        let msg = format_checksum_error();
        assert!(msg.contains("驗證失敗"));
        assert!(!msg.contains("model-q4"));
        assert_player_safe(&msg);
    }

    #[test]
    fn range_spans_cover_without_gap_or_overlap() {
        let total = RANGE_CHUNK * 2 + 5 * 1024 * 1024;
        let spans = range_spans(total, RANGE_CHUNK);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0], (0, RANGE_CHUNK - 1));
        assert_eq!(spans[1], (RANGE_CHUNK, RANGE_CHUNK * 2 - 1));
        assert_eq!(spans[2], (RANGE_CHUNK * 2, total - 1));
        let covered: u64 = spans.iter().map(|(s, e)| e - s + 1).sum();
        assert_eq!(covered, total);
        for w in spans.windows(2) {
            assert_eq!(w[0].1 + 1, w[1].0);
        }
        assert_eq!(range_header(0, RANGE_CHUNK - 1), "bytes=0-33554431");
        assert!(range_spans(0, RANGE_CHUNK).is_empty());
        let mid = remaining_spans(RANGE_CHUNK, total, RANGE_CHUNK);
        assert_eq!(mid[0].0, RANGE_CHUNK);
        assert_eq!(mid.last().map(|s| s.1), Some(total - 1));
    }

    #[test]
    fn chunk_size_keeps_request_count_sane_for_a_large_model() {
        // 7.38 GB 的模型：8 MiB 要 880 個請求，32 MiB 只要 231 個。
        let model_bytes: u64 = 7_381_381_760;
        let requests = range_spans(model_bytes, RANGE_CHUNK).len();
        assert!(requests <= 256, "請求數 {requests} 太多，切太碎會被往返延遲吃掉");
        assert!(RANGE_CHUNK >= 32 * 1024 * 1024);
        // chunk 與逾時必須綁在一起看：逾時要能撐住 chunk 在很慢的線路上跑完，
        // 否則正常下載會被誤判成失敗並無限重試。
        let floor_bytes_per_sec = RANGE_CHUNK / CHUNK_TIMEOUT_SECS;
        assert!(
            floor_bytes_per_sec <= 200 * 1024,
            "逾時撐不住這個 chunk：需要 {floor_bytes_per_sec} B/s 才不會被砍"
        );
    }

    #[test]
    fn range_chunks_assemble_to_original() {
        let src: Vec<u8> = (0u8..=200).collect();
        let chunk = 64usize;
        let mut out = Vec::new();
        let mut parts = 0usize;
        let mut off = 0usize;
        while off < src.len() {
            let end = (off + chunk).min(src.len());
            out.extend_from_slice(&src[off..end]);
            parts += 1;
            off = end;
        }
        assert!(parts >= 3);
        assert_eq!(out, src);
    }
}
