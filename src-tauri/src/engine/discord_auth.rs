//! Discord 桌面登入與官方伺服器會員驗證。
//!
//! 登入流程沿用 ZeitFrei 工具箱：瀏覽器在 cloud.zeitfrei.uk 完成 Discord OAuth，
//! 再把短期桌面 token POST 回本機 loopback。真正的 session 只由 Rust 保存，不進前端。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const WEBSITE_URL: &str = "https://cloud.zeitfrei.uk";
pub const DISCORD_INVITE_URL: &str = "https://discord.gg/zeitfrei";
static LOGIN_CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscordAuthStatus {
    pub logged_in: bool,
    pub in_guild: bool,
    pub service_available: bool,
    pub user_id: String,
    pub username: String,
    pub nickname: String,
    pub avatar: String,
    pub message: String,
}

impl DiscordAuthStatus {
    fn logged_out(message: &str) -> Self {
        Self {
            logged_in: false,
            in_guild: false,
            service_available: true,
            user_id: String::new(),
            username: String::new(),
            nickname: String::new(),
            avatar: String::new(),
            message: message.into(),
        }
    }

    fn unavailable(message: &str, logged_in: bool) -> Self {
        Self {
            logged_in,
            in_guild: false,
            service_available: false,
            user_id: String::new(),
            username: String::new(),
            nickname: String::new(),
            avatar: String::new(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DiscordSessionFile {
    #[serde(default)]
    cookie: String,
    #[serde(default)]
    user: String,
}

fn session_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("modpack-i18n-tool")
        .join("discord-session.json")
}

fn read_session() -> DiscordSessionFile {
    fs::read_to_string(session_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_session(session: &DiscordSessionFile) -> Result<(), String> {
    let path = session_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("無法建立登入資料夾：{e}"))?;
    }
    let text = serde_json::to_string_pretty(session).map_err(|e| e.to_string())? + "\n";
    fs::write(path, text).map_err(|e| format!("無法儲存登入狀態：{e}"))
}

pub fn logout_discord() -> Result<(), String> {
    write_session(&DiscordSessionFile::default())
}

pub fn cancel_discord_login() {
    LOGIN_CANCEL.store(true, Ordering::SeqCst);
}

/// 只回傳網站 session cookie；Worker 仍會再次驗簽並查官方伺服器會員狀態。
pub fn managed_ai_session_cookie() -> Result<String, String> {
    let saved = read_session().cookie;
    if saved.trim().is_empty() {
        return Err("使用開發者提供的 AI 前，請先登入 Discord。".into());
    }
    decode_inner(&saved)
        .filter(|cookie| !cookie.trim().is_empty())
        .ok_or_else(|| "Discord 登入資訊已損壞，請重新登入。".into())
}

/// 驗證登入 session，再用既有會員端點確認人仍在官方伺服器；任何查詢錯誤都不放行。
pub fn check_discord_auth_status() -> DiscordAuthStatus {
    let cookie = match managed_ai_session_cookie() {
        Ok(cookie) => cookie,
        Err(_) => return DiscordAuthStatus::logged_out("尚未登入 Discord。"),
    };
    let client = match reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(client) => client,
        Err(_) => return DiscordAuthStatus::unavailable("目前無法建立登入驗證連線。", true),
    };

    let account = match client
        .get(format!("{WEBSITE_URL}/api/check-upload"))
        .header("Cookie", format!("cf_storage_v3_session={cookie}"))
        .send()
    {
        Ok(response) if response.status().as_u16() == 401 => {
            return DiscordAuthStatus::logged_out("Discord 登入已過期，請重新登入。")
        }
        Ok(response) if response.status().is_success() => match response.json::<Value>() {
            Ok(value) => value,
            Err(_) => {
                return DiscordAuthStatus::unavailable("登入服務回應格式異常，請稍後再試。", true)
            }
        },
        Ok(response) => {
            return DiscordAuthStatus::unavailable(
                &format!(
                    "登入服務回應錯誤（HTTP {}）。請檢查網路或稍後再試。",
                    response.status().as_u16()
                ),
                true,
            )
        }
        Err(error) => {
            return DiscordAuthStatus::unavailable(
                &format!("目前連不上登入服務：{error}"),
                true,
            )
        }
    };

    let user_id = account
        .get("user_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if user_id.is_empty() || !user_id.chars().all(|c| c.is_ascii_digit()) {
        return DiscordAuthStatus::logged_out("Discord 登入資訊無效，請重新登入。");
    }

    let member = match client
        .get(format!("{WEBSITE_URL}/api/member-tier/{user_id}"))
        .send()
    {
        Ok(response) if response.status().is_success() => match response.json::<Value>() {
            Ok(value) => value,
            Err(_) => return DiscordAuthStatus::unavailable("伺服器會員驗證回應異常。", true),
        },
        Ok(response) => {
            return DiscordAuthStatus::unavailable(
                &format!(
                    "伺服器會員驗證回應錯誤（HTTP {}）。請稍後再試。",
                    response.status().as_u16()
                ),
                true,
            )
        }
        Err(error) => {
            return DiscordAuthStatus::unavailable(
                &format!("目前無法確認 Discord 伺服器會員狀態：{error}"),
                true,
            )
        }
    };

    let in_guild = member.get("inGuild").and_then(Value::as_bool) == Some(true);
    let username = account
        .get("username")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let nickname = account
        .get("nickname")
        .and_then(Value::as_str)
        .unwrap_or(&username)
        .to_string();
    let avatar = account
        .get("avatar")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    DiscordAuthStatus {
        logged_in: true,
        in_guild,
        service_available: true,
        user_id,
        username,
        nickname,
        avatar,
        message: if in_guild {
            "Discord 登入與官方伺服器會員驗證完成。".into()
        } else {
            "請先加入 ZeitFrei 官方 Discord 伺服器，再按重新檢查。".into()
        },
    }
}

pub fn login_discord_blocking(app: AppHandle) -> Value {
    LOGIN_CANCEL.store(false, Ordering::SeqCst);
    let listener = match bind_login_listener() {
        Ok(listener) => listener,
        Err(error) => return json!({ "ok": false, "error": error }),
    };
    let port = listener
        .local_addr()
        .map(|addr| addr.port())
        .unwrap_or(19420);
    let url = format!("{WEBSITE_URL}/api/desktop-auth?redirect=http://127.0.0.1:{port}/callback");
    let _ = app.emit("discord-login-url", json!({ "url": url }));
    let _ = open::that(&url);
    let deadline = Instant::now() + Duration::from_secs(300);
    let _ = listener.set_nonblocking(true);

    while Instant::now() < deadline {
        if LOGIN_CANCEL.load(Ordering::SeqCst) {
            return json!({ "ok": false, "error": "cancelled" });
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let request = read_http_request(&mut stream);
                write_callback_response(&mut stream);
                if request.starts_with("OPTIONS ") {
                    continue;
                }
                let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
                if let Ok(data) = serde_json::from_str::<Value>(body) {
                    if let Some(token) = data.get("token").and_then(Value::as_str) {
                        if decode_inner(token).is_none() {
                            return json!({ "ok": false, "error": "invalid_token" });
                        }
                        let user = data
                            .get("user")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        if let Err(error) = write_session(&DiscordSessionFile {
                            cookie: token.to_string(),
                            user: user.clone(),
                        }) {
                            return json!({ "ok": false, "error": error });
                        }
                        return json!({ "ok": true, "user": user });
                    }
                    if let Some(error) = data.get("error").and_then(Value::as_str) {
                        return json!({ "ok": false, "error": error });
                    }
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

fn bind_login_listener() -> Result<TcpListener, String> {
    for port in 19420u16..=19430u16 {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return Ok(listener);
        }
    }
    Err("無法啟動本機登入服務，請檢查防火牆或稍後再試。".into())
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut bytes = Vec::new();
    let mut buf = [0u8; 4096];
    let mut expected_len = None;
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(count) => bytes.extend_from_slice(&buf[..count]),
        }
        if bytes.len() > 65_536 {
            break;
        }
        if let Some(header_end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            if expected_len.is_none() {
                let header = String::from_utf8_lossy(&bytes[..header_end]);
                expected_len = header.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                });
            }
            let body_len = bytes.len().saturating_sub(header_end + 4);
            if body_len >= expected_len.unwrap_or(0) {
                break;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn write_callback_response(stream: &mut TcpStream) {
    let page = "<!doctype html><meta charset=utf-8><title>登入完成</title><body style=\"font-family:sans-serif;background:#171a1d;color:#f3f0e8;text-align:center;padding-top:18vh\"><h2>登入完成</h2><p>可以關閉這個分頁，回到模組包翻譯工具。</p><script>setTimeout(function(){window.close()},1000)</script></body>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        page.len(),
        page
    );
    let _ = stream.write_all(response.as_bytes());
}

fn decode_inner(cookie: &str) -> Option<String> {
    let bytes = b64decode(cookie)?;
    let outer: Value = serde_json::from_slice(&bytes).ok()?;
    let payload = outer.get("p")?.as_str()?;
    let inner: Value = serde_json::from_str(payload).ok()?;
    inner.get("s").and_then(Value::as_str).map(str::to_string)
}

fn b64decode(value: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut reverse = [255u8; 256];
    for (index, byte) in TABLE.iter().enumerate() {
        reverse[*byte as usize] = index as u8;
    }
    let clean: Vec<u8> = value
        .bytes()
        .filter(|byte| *byte != b'=' && !byte.is_ascii_whitespace())
        .collect();
    let mut output = Vec::new();
    for chunk in clean.chunks(4) {
        let mut decoded = [0u8; 4];
        for (index, byte) in chunk.iter().enumerate() {
            let value = reverse[*byte as usize];
            if value == 255 {
                return None;
            }
            decoded[index] = value;
        }
        if chunk.len() >= 2 {
            output.push((decoded[0] << 2) | (decoded[1] >> 4));
        }
        if chunk.len() >= 3 {
            output.push((decoded[1] << 4) | (decoded[2] >> 2));
        }
        if chunk.len() >= 4 {
            output.push((decoded[2] << 6) | decoded[3]);
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::b64decode;

    #[test]
    fn base64_decoder_accepts_padded_input() {
        assert_eq!(b64decode("SGVsbG8=").unwrap(), b"Hello");
    }

    #[test]
    fn base64_decoder_rejects_invalid_input() {
        assert!(b64decode("not@base64").is_none());
    }
}
