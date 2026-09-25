#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEVICE_USER_CODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const DEVICE_VERIFY_URL: &str = "https://auth.openai.com/codex/device";
const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const POLL_INTERVAL_SECS: u64 = 5;
const LOGIN_TIMEOUT_SECS: u64 = 15 * 60;
const EXPIRY_SKEW_SECS: u64 = 60;

static LOGIN_CANCEL: AtomicBool = AtomicBool::new(false);
static REFRESH_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

// OAuth token 有效只代表可向 Codex 端點送出請求，不能代表端點當下仍有翻譯用量。
// 真正的翻譯驗證由 deepseek::verify_ai_assistance 在寫入前執行。
const GPT_LOGGED_IN_PENDING_TRANSLATION_PROBE: &str =
    "ChatGPT 已登入。開始翻譯前會先試翻一句，確認現在能用；翻譯會消耗你的 ChatGPT 帳號額度。";
const GPT_TOKEN_REFRESHED_PENDING_TRANSLATION_PROBE: &str =
    "ChatGPT 已登入（登入狀態已自動更新）。開始翻譯前會先試翻一句，確認現在能用；翻譯會消耗你的 ChatGPT 帳號額度。";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GptAuthStatus {
    pub logged_in: bool,
    pub usable: bool,
    pub state: String,
    pub email: String,
    pub account_id: String,
    pub expired: bool,
    pub expires_at: u64,
    pub checked_at: u64,
    pub message: String,
}

impl GptAuthStatus {
    fn logged_out(message: &str) -> Self {
        Self {
            logged_in: false,
            usable: false,
            state: "logged_out".into(),
            email: String::new(),
            account_id: String::new(),
            expired: true,
            expires_at: 0,
            checked_at: now_unix_secs(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GptAccessContext {
    pub access_token: String,
    pub account_id: String,
    pub email: String,
    pub expires_at: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct CodexAuthFile {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    account_id: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    expired: u64,
}

#[derive(Debug, Deserialize)]
struct DeviceUserCodeResponse {
    #[serde(default)]
    device_auth_id: String,
    #[serde(default)]
    user_code: String,
    #[serde(default)]
    usercode: String,
}

#[derive(Debug, Deserialize)]
struct DeviceTokenResponse {
    #[serde(default)]
    authorization_code: String,
    #[serde(default)]
    code_verifier: String,
    #[serde(default)]
    code_challenge: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    id_token: String,
    #[serde(default)]
    expires_in: u64,
}

#[derive(Debug, Default, Deserialize)]
struct JwtClaims {
    #[serde(default)]
    email: String,
    #[serde(rename = "https://api.openai.com/auth", default)]
    auth: JwtAuthInfo,
}

#[derive(Debug, Default, Deserialize)]
struct JwtAuthInfo {
    #[serde(default)]
    chatgpt_account_id: String,
}

pub fn gpt_login_blocking(app: AppHandle) -> Value {
    LOGIN_CANCEL.store(false, Ordering::SeqCst);
    let client = match build_http_client() {
        Ok(client) => client,
        Err(error) => return json!({ "ok": false, "error": error }),
    };
    let auth_seed = match request_device_user_code(&client) {
        Ok(seed) => seed,
        Err(error) => return json!({ "ok": false, "error": error }),
    };
    let user_code = preferred_user_code(&auth_seed);
    if user_code.is_empty() || auth_seed.device_auth_id.trim().is_empty() {
        return json!({ "ok": false, "error": "GPT 登入服務沒有回傳完整裝置驗證資訊。" });
    }
    let verify_url = format!("{DEVICE_VERIFY_URL}?user_code={user_code}");
    let _ = app.emit(
        "gpt-login-code",
        json!({
            "userCode": user_code,
            "url": verify_url,
        }),
    );
    if let Err(error) = open::that(&verify_url) {
        return json!({
            "ok": false,
            "error": format!("無法開啟 GPT 登入頁：{error}"),
            "url": verify_url,
            "userCode": user_code,
        });
    }
    let device_token = match poll_device_token(&client, &auth_seed.device_auth_id, &user_code) {
        Ok(token) => token,
        Err(error) => return json!({ "ok": false, "error": error }),
    };
    let auth_file = match exchange_authorization_code(&client, &device_token, None) {
        Ok(file) => file,
        Err(error) => return json!({ "ok": false, "error": error }),
    };
    if let Err(error) = write_auth_file(&auth_file) {
        return json!({ "ok": false, "error": error });
    }
    json!({
        "ok": true,
        "email": auth_file.email,
        "accountId": auth_file.account_id,
        "expiresAt": auth_file.expired,
    })
}

pub fn gpt_logout() -> Result<(), String> {
    write_auth_file(&CodexAuthFile::default())
}

pub fn gpt_auth_status() -> GptAuthStatus {
    let auth = read_auth_file();
    if auth.refresh_token.trim().is_empty() || auth.account_id.trim().is_empty() {
        return GptAuthStatus::logged_out("尚未登入 GPT。");
    }

    if !is_token_stale(&auth) {
        return GptAuthStatus {
            logged_in: true,
            usable: true,
            state: "ready".into(),
            email: auth.email,
            account_id: auth.account_id,
            expired: false,
            expires_at: auth.expired,
            checked_at: now_unix_secs(),
            message: GPT_LOGGED_IN_PENDING_TRANSLATION_PROBE.into(),
        };
    }

    match refresh_access_token_internal(false) {
        Ok(fresh) => GptAuthStatus {
            logged_in: true,
            usable: true,
            state: "ready".into(),
            email: fresh.email,
            account_id: fresh.account_id,
            expired: false,
            expires_at: fresh.expires_at,
            checked_at: now_unix_secs(),
            message: GPT_TOKEN_REFRESHED_PENDING_TRANSLATION_PROBE.into(),
        },
        Err(error) if is_refresh_reauthorization_error(&error) => GptAuthStatus {
            logged_in: true,
            usable: false,
            state: "reauth_required".into(),
            email: auth.email,
            account_id: auth.account_id,
            expired: true,
            expires_at: auth.expired,
            checked_at: now_unix_secs(),
            message: "GPT 登入已失效，請重新登入。".into(),
        },
        Err(error) => GptAuthStatus {
            logged_in: true,
            usable: false,
            state: "refresh_failed".into(),
            email: auth.email,
            account_id: auth.account_id,
            expired: true,
            expires_at: auth.expired,
            checked_at: now_unix_secs(),
            message: format!("GPT 登入已保存，但目前無法更新存取權杖：{error}"),
        },
    }
}

pub fn cancel_gpt_login() {
    LOGIN_CANCEL.store(true, Ordering::SeqCst);
}

pub fn ensure_fresh_access_token() -> Result<GptAccessContext, String> {
    refresh_access_token_internal(false)
}

pub(crate) fn refresh_access_token_force() -> Result<GptAccessContext, String> {
    refresh_access_token_internal(true)
}

pub(crate) fn has_saved_gpt_login() -> bool {
    let auth = read_auth_file();
    !auth.refresh_token.trim().is_empty() && !auth.account_id.trim().is_empty()
}

fn refresh_access_token_internal(force: bool) -> Result<GptAccessContext, String> {
    let _guard = REFRESH_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "GPT 登入狀態鎖定失敗，請重試。".to_string())?;
    let auth = read_auth_file();
    if auth.refresh_token.trim().is_empty() || auth.account_id.trim().is_empty() {
        return Err("已選 GPT，請先登入 GPT 帳號。".into());
    }
    if !force && !is_token_stale(&auth) {
        return Ok(to_access_context(&auth));
    }
    let client = build_http_client()?;
    let refreshed = refresh_access_token_with_client(&client, &auth.refresh_token, Some(&auth))?;
    write_auth_file(&refreshed)?;
    Ok(to_access_context(&refreshed))
}

fn build_http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|e| format!("無法建立 GPT 登入連線：{e}"))
}

fn request_device_user_code(client: &reqwest::blocking::Client) -> Result<DeviceUserCodeResponse, String> {
    let response = client
        .post(DEVICE_USER_CODE_URL)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&json!({ "client_id": CLIENT_ID }))
        .send()
        .map_err(|e| format!("無法向 GPT 登入服務取得裝置碼：{e}"))?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "GPT 登入服務回應錯誤（HTTP {}）：{}",
            status.as_u16(),
            sanitize_error_excerpt(&body)
        ));
    }
    serde_json::from_str(&body)
        .map_err(|e| format!("GPT 登入服務回應格式異常：{e}"))
}

fn preferred_user_code(response: &DeviceUserCodeResponse) -> String {
    let primary = response.user_code.trim();
    if !primary.is_empty() {
        return primary.to_string();
    }
    response.usercode.trim().to_string()
}

fn poll_device_token(
    client: &reqwest::blocking::Client,
    device_auth_id: &str,
    user_code: &str,
) -> Result<DeviceTokenResponse, String> {
    let deadline = Instant::now() + Duration::from_secs(LOGIN_TIMEOUT_SECS);
    while Instant::now() < deadline {
        if LOGIN_CANCEL.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let response = client
            .post(DEVICE_TOKEN_URL)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&json!({
                "device_auth_id": device_auth_id,
                "user_code": user_code,
            }))
            .send()
            .map_err(|e| format!("無法輪詢 GPT 裝置登入狀態：{e}"))?;
        let status = response.status();
        let body = response.text().unwrap_or_default();
        if status.is_success() {
            let parsed: DeviceTokenResponse = serde_json::from_str(&body)
                .map_err(|e| format!("GPT 裝置登入回應格式異常：{e}"))?;
            if parsed.authorization_code.trim().is_empty()
                || parsed.code_verifier.trim().is_empty()
                || parsed.code_challenge.trim().is_empty()
            {
                return Err("GPT 裝置登入回應缺少必要欄位。".into());
            }
            return Ok(parsed);
        }
        if is_pending_device_poll(status.as_u16(), &body) {
            std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
            continue;
        }
        return Err(format!(
            "GPT 裝置登入失敗（HTTP {}）：{}",
            status.as_u16(),
            sanitize_error_excerpt(&body)
        ));
    }
    Err("GPT 登入逾時，請重新嘗試。".into())
}

fn is_pending_device_poll(status: u16, body: &str) -> bool {
    if matches!(status, 403 | 404) {
        return true;
    }
    if status != 400 {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("authorization_pending")
        || lower.contains("device authorization")
        || lower.contains("device auth")
}

fn exchange_authorization_code(
    client: &reqwest::blocking::Client,
    device_token: &DeviceTokenResponse,
    existing: Option<&CodexAuthFile>,
) -> Result<CodexAuthFile, String> {
    let response = client
        .post(OAUTH_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", device_token.authorization_code.trim()),
            ("redirect_uri", DEVICE_REDIRECT_URI),
            ("code_verifier", device_token.code_verifier.trim()),
        ])
        .send()
        .map_err(|e| format!("無法完成 GPT 權杖交換：{e}"))?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "GPT 權杖交換失敗（HTTP {}）：{}",
            status.as_u16(),
            sanitize_error_excerpt(&body)
        ));
    }
    let token: TokenResponse = serde_json::from_str(&body)
        .map_err(|e| format!("GPT 權杖交換回應格式異常：{e}"))?;
    build_auth_file(token, existing)
}

fn refresh_access_token_with_client(
    client: &reqwest::blocking::Client,
    refresh_token: &str,
    existing: Option<&CodexAuthFile>,
) -> Result<CodexAuthFile, String> {
    let response = client
        .post(OAUTH_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.trim()),
            ("scope", "openid profile email"),
        ])
        .send()
        .map_err(|e| format!("無法更新 GPT 存取權杖：{e}"))?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "GPT 存取權杖更新失敗（HTTP {}）：{}",
            status.as_u16(),
            sanitize_error_excerpt(&body)
        ));
    }
    let token: TokenResponse = serde_json::from_str(&body)
        .map_err(|e| format!("GPT 存取權杖更新回應格式異常：{e}"))?;
    build_auth_file(token, existing)
}

fn build_auth_file(token: TokenResponse, existing: Option<&CodexAuthFile>) -> Result<CodexAuthFile, String> {
    if token.access_token.trim().is_empty() {
        return Err("GPT OAuth 沒有回傳 access token。".into());
    }
    let claims = decode_jwt_claims(&token.id_token);
    let mut account_id = claims
        .as_ref()
        .map(|value| value.auth.chatgpt_account_id.trim().to_string())
        .unwrap_or_default();
    let mut email = claims
        .as_ref()
        .map(|value| value.email.trim().to_string())
        .unwrap_or_default();
    if account_id.is_empty() {
        account_id = existing
            .map(|value| value.account_id.trim().to_string())
            .unwrap_or_default();
    }
    if email.is_empty() {
        email = existing
            .map(|value| value.email.trim().to_string())
            .unwrap_or_default();
    }
    if account_id.is_empty() || email.is_empty() {
        return Err("GPT OAuth 沒有回傳完整帳號資訊，請重新登入。".into());
    }
    let refresh_token = if token.refresh_token.trim().is_empty() {
        existing
            .map(|value| value.refresh_token.trim().to_string())
            .unwrap_or_default()
    } else {
        token.refresh_token.trim().to_string()
    };
    if refresh_token.is_empty() {
        return Err("GPT OAuth 沒有回傳 refresh token，請重新登入。".into());
    }
    Ok(CodexAuthFile {
        access_token: token.access_token.trim().to_string(),
        refresh_token,
        account_id,
        email,
        expired: now_unix_secs().saturating_add(token.expires_in.max(60)),
    })
}

fn auth_path() -> PathBuf {
    super::paths::resolve_file(Path::new("codex_auth.json"))
}

fn read_auth_file() -> CodexAuthFile {
    fs::read_to_string(auth_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_auth_file(auth: &CodexAuthFile) -> Result<(), String> {
    let path = auth_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("無法建立 GPT 登入資料夾：{e}"))?;
    }
    let text = serde_json::to_string_pretty(auth).map_err(|e| e.to_string())? + "\n";
    fs::write(&path, text).map_err(|e| format!("無法儲存 GPT 登入狀態：{e}"))?;
    harden_auth_file_acl(&path);
    Ok(())
}

fn harden_auth_file_acl(path: &Path) {
    #[cfg(windows)]
    {
        let Some(path_str) = path.to_str() else {
            return;
        };
        let user = std::env::var("USERNAME").unwrap_or_default();
        if user.is_empty() {
            return;
        }
        let grant = format!("{user}:(R,W)");
        let _ = crate::engine::win_process::hidden_command("icacls")
            .args([path_str, "/inheritance:r", "/grant:r", &grant])
            .output();
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}

fn sanitize_error_excerpt(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = compact.trim();
    if trimmed.is_empty() {
        return "空回應".into();
    }
    trimmed.chars().take(220).collect()
}

fn is_refresh_reauthorization_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("invalid_grant")
        || lower.contains("invalid refresh")
        || lower.contains("refresh token") && lower.contains("invalid")
        || lower.contains("http 401")
}

fn is_token_stale(auth: &CodexAuthFile) -> bool {
    auth.access_token.trim().is_empty()
        || auth.expired == 0
        || auth.expired <= now_unix_secs().saturating_add(EXPIRY_SKEW_SECS)
}

fn to_access_context(auth: &CodexAuthFile) -> GptAccessContext {
    GptAccessContext {
        access_token: auth.access_token.trim().to_string(),
        account_id: auth.account_id.trim().to_string(),
        email: auth.email.trim().to_string(),
        expires_at: auth.expired,
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn decode_jwt_claims(id_token: &str) -> Option<JwtClaims> {
    let mut parts = id_token.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let _sig = parts.next()?;
    let bytes = decode_base64_url(payload)?;
    serde_json::from_slice(&bytes).ok()
}

fn decode_base64_url(input: &str) -> Option<Vec<u8>> {
    let mut normalized = input.replace('-', "+").replace('_', "/");
    while normalized.len() % 4 != 0 {
        normalized.push('=');
    }
    decode_base64_standard(&normalized)
}

fn decode_base64_standard(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut reverse = [255u8; 256];
    for (index, byte) in TABLE.iter().enumerate() {
        reverse[*byte as usize] = index as u8;
    }
    let mut output = Vec::new();
    let clean: Vec<u8> = input.bytes().filter(|byte| !byte.is_ascii_whitespace()).collect();
    for chunk in clean.chunks(4) {
        if chunk.len() < 4 {
            return None;
        }
        let mut decoded = [0u8; 4];
        let mut padding = 0usize;
        for (index, byte) in chunk.iter().enumerate() {
            if *byte == b'=' {
                decoded[index] = 0;
                padding += 1;
                continue;
            }
            let value = reverse[*byte as usize];
            if value == 255 {
                return None;
            }
            decoded[index] = value;
        }
        output.push((decoded[0] << 2) | (decoded[1] >> 4));
        if padding < 2 {
            output.push((decoded[1] << 4) | (decoded[2] >> 2));
        }
        if padding < 1 {
            output.push((decoded[2] << 6) | decoded[3]);
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::{
        auth_path, build_auth_file, decode_jwt_claims, has_saved_gpt_login, is_pending_device_poll,
        preferred_user_code, read_auth_file, CodexAuthFile, DeviceUserCodeResponse,
        GPT_LOGGED_IN_PENDING_TRANSLATION_PROBE, GPT_TOKEN_REFRESHED_PENDING_TRANSLATION_PROBE,
        TokenResponse,
    };
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn temp_data_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!("mcpl-codex-auth-test-{unique}"));
        fs::create_dir_all(dir.join("modpack-i18n-tool")).unwrap();
        dir
    }

    fn with_fake_appdata<T>(test: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let dir = temp_data_dir();
        let old = env::var_os("APPDATA");
        env::set_var("APPDATA", &dir);
        let result = test();
        if let Some(value) = old {
            env::set_var("APPDATA", value);
        } else {
            env::remove_var("APPDATA");
        }
        let _ = fs::remove_dir_all(&dir);
        result
    }

    fn encode_base64_url(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        let mut index = 0usize;
        while index < bytes.len() {
            let a = bytes[index];
            let b = if index + 1 < bytes.len() { bytes[index + 1] } else { 0 };
            let c = if index + 2 < bytes.len() { bytes[index + 2] } else { 0 };
            out.push(TABLE[(a >> 2) as usize] as char);
            out.push(TABLE[((a & 0x03) << 4 | (b >> 4)) as usize] as char);
            if index + 1 < bytes.len() {
                out.push(TABLE[((b & 0x0f) << 2 | (c >> 6)) as usize] as char);
            }
            if index + 2 < bytes.len() {
                out.push(TABLE[(c & 0x3f) as usize] as char);
            }
            index += 3;
        }
        out
    }

    #[test]
    fn device_user_code_prefers_primary_field() {
        let response = DeviceUserCodeResponse {
            device_auth_id: "dev".into(),
            user_code: "ABC-123".into(),
            usercode: "legacy".into(),
        };
        assert_eq!(preferred_user_code(&response), "ABC-123");
    }

    #[test]
    fn pending_poll_accepts_known_statuses() {
        assert!(is_pending_device_poll(403, ""));
        assert!(is_pending_device_poll(404, ""));
        assert!(is_pending_device_poll(400, "{\"error\":\"authorization_pending\"}"));
        assert!(!is_pending_device_poll(401, "nope"));
    }

    #[test]
    fn valid_oauth_text_does_not_claim_translation_endpoint_is_ready() {
        for message in [
            GPT_LOGGED_IN_PENDING_TRANSLATION_PROBE,
            GPT_TOKEN_REFRESHED_PENDING_TRANSLATION_PROBE,
        ] {
            assert!(message.contains("開始翻譯前會先試翻一句"));
            assert!(message.contains("會消耗你的 ChatGPT 帳號額度"), "要誠實說明會用到額度");
            assert!(!message.contains("可直接使用"));
            for word in ["Codex", "端點", "權杖", "探測", "預檢", "429", "Token", "token"] {
                assert!(!message.contains(word), "玩家看得到的文字不可出現「{word}」：{message}");
            }
        }
    }

    #[test]
    fn jwt_claims_extract_email_and_account() {
        let header = encode_base64_url(br#"{"alg":"none"}"#);
        let payload = encode_base64_url(
            br#"{"email":"user@example.com","https://api.openai.com/auth":{"chatgpt_account_id":"acct_123"}}"#,
        );
        let token = format!("{header}.{payload}.sig");
        let claims = decode_jwt_claims(&token).unwrap();
        assert_eq!(claims.email, "user@example.com");
        assert_eq!(claims.auth.chatgpt_account_id, "acct_123");
    }

    #[test]
    fn build_auth_file_keeps_existing_refresh_token() {
        let token = TokenResponse {
            access_token: "access".into(),
            refresh_token: String::new(),
            id_token: String::new(),
            expires_in: 3600,
        };
        let existing = CodexAuthFile {
            access_token: "old".into(),
            refresh_token: "refresh".into(),
            account_id: "acct_123".into(),
            email: "user@example.com".into(),
            expired: 1,
        };
        let auth = build_auth_file(token, Some(&existing)).unwrap();
        assert_eq!(auth.refresh_token, "refresh");
        assert_eq!(auth.account_id, "acct_123");
        assert_eq!(auth.email, "user@example.com");
    }

    #[test]
    fn saved_login_requires_refresh_and_identity() {
        with_fake_appdata(|| {
            // 直接寫進 legacy 根（faked APPDATA 生效的那個），不透過 auth_path() 的
            // 回傳值決定寫哪裡——auth_path() 兩邊（可攜式根／legacy 根）都沒有東西時
            // 會落在可攜式根，那是測試執行檔真正所在的資料夾，寫進去等於把測試殘留
            // 汙染到建置產物。這裡先在 legacy 根放好檔案，驗證 auth_path() 正確
            // 找得到既有安裝（第九輪以前）的資料。
            let path = crate::engine::paths::legacy_roaming_root().join("codex_auth.json");
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(
                &path,
                r#"{"refresh_token":"refresh","account_id":"acct_123","email":"user@example.com"}"#,
            )
            .unwrap();
            assert_eq!(auth_path(), path, "legacy 根已有檔案時，auth_path() 應沿用它");
            let auth = read_auth_file();
            assert_eq!(auth.refresh_token, "refresh");
            assert!(has_saved_gpt_login());
        });
    }
}
