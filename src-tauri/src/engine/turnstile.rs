//! Cloudflare Turnstile 桌面驗證橋接。
//!
//! Widget 與 Siteverify 都在官方 Worker 網域執行。桌面端只負責：
//! 1. 用既有 Discord session 申請一次性挑戰網址。
//! 2. 在 127.0.0.1 等待 Worker 回傳已簽章的短效通行憑證。
//! 3. 僅於目前程式生命週期的記憶體保存憑證，不寫入磁碟。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

use super::discord_auth::managed_ai_session_cookie;
use super::secrets::MANAGED_BASE_URL;

pub const MANAGED_AI_PROTOCOL: &str = "3";
const CALLBACK_ORIGIN: &str = "https://modpack-i18n.jolin34563.workers.dev";
const PROOF_MAX_LEN: usize = 4096;
const VERIFY_TIMEOUT_SECONDS: u64 = 300;
static VERIFY_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static VERIFY_CANCEL: AtomicBool = AtomicBool::new(false);
static PROOF: OnceLock<Mutex<Option<CachedProof>>> = OnceLock::new();

#[derive(Debug, Clone)]
struct CachedProof {
    token: String,
    expires_at: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnstileStatus {
    pub verified: bool,
    pub expires_at: u64,
    pub message: String,
}

/// 讀取 Worker 的公開健康狀態，確認目前線上版本是否真的強制 Turnstile。
///
/// 不能只看本機是否有憑證：Worker 可以在部署後切換強制模式，
/// 而且舊版桌面程式曾把「未設定金鑰」的相容行為誤套到已啟用的服務上。
pub fn managed_turnstile_required() -> Result<bool, String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|error| format!("無法建立安全驗證狀態連線：{error}"))?;
    let response = client
        .get(format!("{}/health", MANAGED_BASE_URL.trim_end_matches('/')))
        .send()
        .map_err(|error| format!("無法確認安全驗證服務狀態：{error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "安全驗證服務狀態回應錯誤：HTTP {}",
            response.status().as_u16()
        ));
    }
    let value = response
        .json::<Value>()
        .map_err(|error| format!("安全驗證服務回應格式錯誤：{error}"))?;
    turnstile_required_from_health(&value)
}

/// 從 `/health` 判定代管 AI 是否強制 Turnstile。
/// - `Ok(true)`：已就緒且 enforced
/// - `Ok(false)`：服務端明確不強制（enforced=false）
/// - `Err`：無法讀狀態，或 enforced 但尚未就緒（缺 Site Key／Secret／Proof）
fn turnstile_required_from_health(value: &Value) -> Result<bool, String> {
    let ready = value
        .get("turnstileReady")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| "安全驗證服務缺少必要的狀態欄位。".to_string())?;
    let enforced = value
        .pointer("/turnstile/enforced")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| "安全驗證服務缺少必要的狀態欄位。".to_string())?;

    if enforced && !ready {
        let mut missing = Vec::new();
        let flag = |key: &str| {
            value
                .pointer(&format!("/turnstile/{key}"))
                .and_then(|v| v.as_bool())
                == Some(true)
        };
        if !flag("siteKey") {
            missing.push("TURNSTILE_SITE_KEY（vars）");
        }
        if !flag("siteSecret") {
            missing.push("TURNSTILE_SECRET_KEY（secret）");
        }
        if !flag("proofSecret") {
            missing.push("TURNSTILE_PROOF_SECRET（secret，≥32 字元）");
        }
        let detail = if missing.is_empty() {
            "強制驗證已開但尚未就緒".to_string()
        } else {
            format!("缺少 {}", missing.join("／"))
        };
        return Err(format!(
            "安全驗證尚未完成服務端設定 — {detail}。請管理員用 wrangler secret put 補齊後確認 /health turnstileReady=true，或改用自訂 API。"
        ));
    }

    Ok(ready && enforced)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartResponse {
    ok: bool,
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CallbackBody {
    proof: String,
    expires_at: u64,
}

struct VerifyGuard;

impl VerifyGuard {
    fn acquire() -> Result<Self, String> {
        VERIFY_IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| Self)
            .map_err(|_| "安全驗證已在進行中，請回到瀏覽器完成。".to_string())
    }
}

impl Drop for VerifyGuard {
    fn drop(&mut self) {
        VERIFY_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

fn proof_cache() -> &'static Mutex<Option<CachedProof>> {
    PROOF.get_or_init(|| Mutex::new(None))
}

pub fn clear_turnstile_proof() {
    if let Ok(mut cached) = proof_cache().lock() {
        *cached = None;
    }
}

pub fn cancel_turnstile_verification() {
    VERIFY_CANCEL.store(true, Ordering::SeqCst);
}

pub fn turnstile_status() -> TurnstileStatus {
    let now = epoch_seconds();
    let cached = proof_cache()
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
        .filter(|proof| proof.expires_at > now + 30 && valid_proof_shape(&proof.token));
    match cached {
        Some(proof) => TurnstileStatus {
            verified: true,
            expires_at: proof.expires_at,
            message: "Cloudflare 安全驗證已完成。".into(),
        },
        None => {
            clear_turnstile_proof();
            TurnstileStatus {
                verified: false,
                expires_at: 0,
                message: "使用開發者提供的 AI 前，請完成 Cloudflare 安全驗證。".into(),
            }
        }
    }
}

pub fn managed_ai_turnstile_proof() -> Result<String, String> {
    let status = turnstile_status();
    if !status.verified {
        return Err(status.message);
    }
    proof_cache()
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|proof| proof.token.clone()))
        .ok_or_else(|| "Cloudflare 安全驗證已失效，請重新驗證。".into())
}

pub fn verify_turnstile_blocking(app: AppHandle) -> Value {
    let _guard = match VerifyGuard::acquire() {
        Ok(guard) => guard,
        Err(error) => return json!({ "ok": false, "error": error }),
    };
    VERIFY_CANCEL.store(false, Ordering::SeqCst);

    let listener = match bind_callback_listener() {
        Ok(listener) => listener,
        Err(error) => return json!({ "ok": false, "error": error }),
    };
    let port = listener
        .local_addr()
        .map(|address| address.port())
        .unwrap_or(19431);
    let callback = format!("http://127.0.0.1:{port}/turnstile-callback");
    let session = match managed_ai_session_cookie() {
        Ok(session) => session,
        Err(error) => return json!({ "ok": false, "error": error }),
    };

    let client = match reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .build()
    {
        Ok(client) => client,
        Err(error) => return json!({ "ok": false, "error": format!("無法建立安全驗證連線：{error}") }),
    };
    let response = match client
        .post(format!(
            "{}/api/turnstile/start",
            MANAGED_BASE_URL.trim_end_matches('/')
        ))
        .header("X-Zeitfrei-AI-Protocol", MANAGED_AI_PROTOCOL)
        .header("X-Zeitfrei-Client-Version", env!("CARGO_PKG_VERSION"))
        .header("X-Zeitfrei-Session", session)
        .json(&json!({ "callback": callback }))
        .send()
    {
        Ok(response) => response,
        Err(_) => return json!({ "ok": false, "error": "目前連不上安全驗證服務。" }),
    };
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().unwrap_or_default();
        let mut detail = String::new();
        if let Ok(value) = serde_json::from_str::<Value>(&body) {
            detail = value
                .get("error")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
        }
        if detail.is_empty() {
            detail = body.chars().take(160).collect();
        }
        return json!({
            "ok": false,
            "error": if status == 426 {
                "安全驗證協定已更新，請先更新工具。".to_string()
            } else if status == 401 {
                "Discord 登入已過期，請重新登入。".to_string()
            } else if status == 403 {
                "請先加入 ZeitFrei 官方 Discord 伺服器。".to_string()
            } else if !detail.is_empty() {
                format!("安全驗證失敗（HTTP {status}）：{detail}")
            } else {
                format!("安全驗證服務回應錯誤（HTTP {status}）。")
            }
        });
    }
    let started = match response.json::<StartResponse>() {
        Ok(value) if value.ok && valid_challenge_url(&value.url) => value,
        _ => return json!({ "ok": false, "error": "安全驗證服務回應格式異常。" }),
    };
    let _ = app.emit("turnstile-url", json!({ "url": started.url }));
    if open::that(&started.url).is_err() {
        return json!({ "ok": false, "error": "browser_open_failed", "url": started.url });
    }

    let deadline = Instant::now() + Duration::from_secs(VERIFY_TIMEOUT_SECONDS);
    let _ = listener.set_nonblocking(true);
    while Instant::now() < deadline {
        if VERIFY_CANCEL.load(Ordering::SeqCst) {
            return json!({ "ok": false, "error": "cancelled" });
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let request = read_http_request(&mut stream);
                if request.starts_with("OPTIONS ") {
                    write_callback_response(&mut stream, true);
                    continue;
                }
                let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
                let accepted = request.starts_with("POST /turnstile-callback ")
                    && serde_json::from_str::<CallbackBody>(body)
                        .ok()
                        .and_then(store_callback_proof)
                        .is_some();
                write_callback_response(&mut stream, accepted);
                if accepted {
                    return json!({ "ok": true, "expiresAt": turnstile_status().expires_at });
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(_) => return json!({ "ok": false, "error": "loopback_error" }),
        }
    }
    json!({ "ok": false, "error": "timeout" })
}

fn store_callback_proof(body: CallbackBody) -> Option<()> {
    let now = epoch_seconds();
    if !valid_proof_shape(&body.proof)
        || body.expires_at <= now + 30
        || body.expires_at > now + 2 * 60 * 60 + 60
    {
        return None;
    }
    let mut cached = proof_cache().lock().ok()?;
    *cached = Some(CachedProof {
        token: body.proof,
        expires_at: body.expires_at,
    });
    Some(())
}

fn valid_challenge_url(value: &str) -> bool {
    let prefix = format!("{}/turnstile?state=", MANAGED_BASE_URL.trim_end_matches('/'));
    value.starts_with(&prefix) && value.len() <= 8192 && !value.contains(['\r', '\n'])
}

fn valid_proof_shape(value: &str) -> bool {
    value.len() >= 20
        && value.len() <= PROOF_MAX_LEN
        && value.split_once('.').is_some_and(|(body, signature)| {
            !body.is_empty()
                && !signature.is_empty()
                && body
                    .chars()
                    .chain(signature.chars())
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

fn bind_callback_listener() -> Result<TcpListener, String> {
    for port in 19431u16..=19440u16 {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return Ok(listener);
        }
    }
    Err("無法啟動本機安全驗證服務，請檢查防火牆或稍後再試。".into())
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    let mut expected_len = None;
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => bytes.extend_from_slice(&buffer[..count]),
        }
        if bytes.len() > 16_384 {
            break;
        }
        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            if expected_len.is_none() {
                let header = String::from_utf8_lossy(&bytes[..header_end]);
                expected_len = header.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                });
            }
            if bytes.len().saturating_sub(header_end + 4) >= expected_len.unwrap_or(0) {
                break;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn write_callback_response(stream: &mut TcpStream, ok: bool) {
    let status = if ok { "200 OK" } else { "400 Bad Request" };
    let body = if ok { "ok" } else { "invalid" };
    let response = format!(
        "HTTP/1.1 {status}\r\nAccess-Control-Allow-Origin: {CALLBACK_ORIGIN}\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{turnstile_required_from_health, valid_challenge_url, valid_proof_shape};
    use serde_json::json;

    #[test]
    fn accepts_only_managed_worker_challenge_url() {
        assert!(valid_challenge_url(
            "https://modpack-i18n.jolin34563.workers.dev/turnstile?state=abc"
        ));
        assert!(!valid_challenge_url("https://example.com/turnstile?state=abc"));
        assert!(!valid_challenge_url(
            "https://modpack-i18n.jolin34563.workers.dev/turnstile?state=abc\r\nX-Test: 1"
        ));
    }

    #[test]
    fn proof_shape_rejects_whitespace_and_invalid_characters() {
        assert!(valid_proof_shape("abcdefghijklmnop.qrstuvwxyz012345"));
        assert!(!valid_proof_shape("abc.def"));
        assert!(!valid_proof_shape("abcdefghijklmnop.qrstuvwxy z012345"));
        assert!(!valid_proof_shape("abcdefghijklmnop.qrstuvwxyz01234+"));
    }

    #[test]
    fn health_status_distinguishes_enforced_turnstile() {
        assert_eq!(
            turnstile_required_from_health(&json!({
                "turnstileReady": true,
                "turnstile": { "enforced": true }
            })),
            Ok(true)
        );
        assert_eq!(
            turnstile_required_from_health(&json!({
                "turnstileReady": true,
                "turnstile": { "enforced": false }
            })),
            Ok(false)
        );
        assert_eq!(
            turnstile_required_from_health(&json!({
                "turnstileReady": false,
                "turnstile": { "enforced": false }
            })),
            Ok(false)
        );
        let misconfigured = turnstile_required_from_health(&json!({
            "turnstileReady": false,
            "turnstile": {
                "siteKey": true,
                "siteSecret": false,
                "proofSecret": true,
                "enforced": true
            }
        }));
        assert!(misconfigured.is_err());
        let err = misconfigured.unwrap_err();
        assert!(err.contains("TURNSTILE_SECRET_KEY"));
        assert!(!err.contains("目前不需要"));
        assert!(turnstile_required_from_health(&json!({})).is_err());
    }
}
