use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use super::security::{validate_api_base_url, validate_api_key};

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ApiSettingsPublic {
    pub has_key: bool,
    /// 僅遮罩顯示，不回傳完整金鑰
    pub key_masked: String,
    pub base_url: String,
    /// managed＝開發者代管；custom＝使用者自備 API。
    pub ai_mode: String,
    /// 按視窗關閉時改為縮小，不結束程式
    pub minimize_on_close: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct SecretsFile {
    #[serde(default)]
    deepseek_api_key: String,
    #[serde(default = "default_base")]
    deepseek_base: String,
    /// 模型名稱。任何 OpenAI 相容端點都能用，換服務商只要改這兩欄。
    #[serde(default = "default_model")]
    model: String,
    /// AI 來源必須明確選擇，不能再用「有沒有金鑰」暗中切換。
    #[serde(default = "default_ai_mode")]
    ai_mode: String,
    /// 關閉視窗＝縮小（預設 true，避免誤關中斷長任務）
    #[serde(default = "default_true")]
    minimize_on_close: bool,
}

impl Default for SecretsFile {
    fn default() -> Self {
        Self {
            deepseek_api_key: String::new(),
            deepseek_base: default_base(),
            model: default_model(),
            ai_mode: default_ai_mode(),
            minimize_on_close: true,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_base() -> String {
    "https://api.deepseek.com".into()
}

fn default_model() -> String {
    "deepseek-chat".into()
}

fn default_ai_mode() -> String {
    "managed".into()
}

fn normalize_ai_mode(mode: &str) -> Option<&'static str> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "managed" => Some("managed"),
        "custom" => Some("custom"),
        _ => None,
    }
}

/// 開發者提供的代管翻譯端點（Cloudflare Worker）。
///
/// 這是 **URL，不是機密**——真正的 DeepSeek 金鑰放在 Worker 的 secret 裡，
/// 永遠不會進到這支執行檔。使用者沒有自填金鑰時，AI 就走這條免費代管路徑。
pub const MANAGED_BASE_URL: &str = "https://modpack-i18n.jolin34563.workers.dev";

/// AI 連線設定（金鑰 + 端點 + 模型）。
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// 代管模式為空字串（金鑰在 Worker 端）。
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    /// true＝走開發者代管 Worker；false＝使用者自填金鑰直連上游。
    pub managed: bool,
}

/// 讀取「使用者自填」的金鑰設定；沒填回 `None`。
///
/// 用於判斷 UI 上的「金鑰：已設定／未設定」與是否直連上游。代管模式不算「自填」。
pub fn load_api_config() -> Option<ApiConfig> {
    let s = read_file();
    let api_key = s.deepseek_api_key.trim().to_string();
    if api_key.is_empty() {
        return None;
    }
    let base_url = validate_api_base_url(&s.deepseek_base).unwrap_or_else(|_| default_base());
    let model = if s.model.trim().is_empty() {
        default_model()
    } else {
        s.model.trim().to_string()
    };
    Some(ApiConfig {
        api_key,
        base_url,
        model,
        managed: false,
    })
}

/// 決定這次翻譯要用哪個 AI 端點。
///
/// 使用者必須明確選擇來源：自訂模式需要本機已儲存金鑰；代管模式走 Worker，
/// 並由 `deepseek::Engine::connect` 夾帶 Discord session 供伺服器再次驗證。
pub fn resolve_ai_config() -> Result<ApiConfig, String> {
    if get_ai_mode() == "custom" {
        return load_api_config()
            .ok_or_else(|| "已選擇自訂 API，但尚未儲存 API 金鑰。請回到進階 AI 設定填寫。".into());
    }
    Ok(ApiConfig {
        api_key: String::new(),
        base_url: MANAGED_BASE_URL.to_string(),
        model: default_model(),
        managed: true,
    })
}

fn secrets_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("modpack-i18n-tool")
        .join("secrets.json")
}

fn read_file() -> SecretsFile {
    let p = secrets_path();
    if !p.is_file() {
        return SecretsFile::default();
    }
    fs::read_to_string(&p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_file(s: &SecretsFile) -> Result<(), String> {
    let p = secrets_path();
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(p, serde_json::to_string_pretty(s).unwrap() + "\n").map_err(|e| e.to_string())
}

/// 儲存設定。**api_key 若空白則保留原本已存的金鑰**（不刪測試用 key）。
pub fn save_api_settings(api_key: &str, base_url: &str) -> Result<(), String> {
    validate_api_key(api_key)?;
    let mut s = read_file();
    let key = api_key.trim();
    if !key.is_empty() {
        s.deepseek_api_key = key.to_string();
    }
    // 若原本就沒有 key，且這次也沒填 → 錯誤
    if s.deepseek_api_key.trim().is_empty() {
        return Err("請填入 API 金鑰（空白不會刪除已儲存的金鑰，但目前尚未有任何金鑰）。".into());
    }
    // 網址留空＝保留原本；不要用空白覆寫
    let bu = base_url.trim();
    if !bu.is_empty() {
        s.deepseek_base = validate_api_base_url(bu)?;
    } else if s.deepseek_base.trim().is_empty() {
        s.deepseek_base = default_base();
    }
    write_file(&s)
}

pub fn get_api_settings_public() -> ApiSettingsPublic {
    let s = read_file();
    let key = s.deepseek_api_key.trim();
    ApiSettingsPublic {
        has_key: !key.is_empty(),
        // 介面永不顯示金鑰片段／服務商網址
        key_masked: String::new(),
        base_url: String::new(),
        ai_mode: normalize_ai_mode(&s.ai_mode)
            .unwrap_or("managed")
            .to_string(),
        minimize_on_close: s.minimize_on_close,
    }
}

pub fn get_ai_mode() -> String {
    normalize_ai_mode(&read_file().ai_mode)
        .unwrap_or("managed")
        .to_string()
}

pub fn set_ai_mode(mode: &str) -> Result<String, String> {
    let normalized = normalize_ai_mode(mode)
        .ok_or_else(|| "AI 來源只能選擇開發者代管或自訂 API。".to_string())?;
    let mut settings = read_file();
    settings.ai_mode = normalized.to_string();
    write_file(&settings)?;
    Ok(normalized.to_string())
}

pub fn get_minimize_on_close() -> bool {
    read_file().minimize_on_close
}

pub fn set_minimize_on_close(v: bool) -> Result<(), String> {
    let mut s = read_file();
    s.minimize_on_close = v;
    write_file(&s)
}

#[cfg(test)]
mod tests {
    use super::normalize_ai_mode;

    #[test]
    fn ai_mode_accepts_only_known_sources() {
        assert_eq!(normalize_ai_mode(" managed "), Some("managed"));
        assert_eq!(normalize_ai_mode("CUSTOM"), Some("custom"));
        assert_eq!(normalize_ai_mode("legacy"), None);
    }
}
