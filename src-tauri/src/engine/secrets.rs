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
/// 使用者有自填金鑰 → 直連他自己的上游（不佔用開發者額度）；
/// 否則 → 開發者代管 Worker（免費、零設定）。**一定會回傳一個可用設定**，
/// 所以預設就能翻譯，不再需要玩家先去弄金鑰。
pub fn resolve_ai_config() -> ApiConfig {
    load_api_config().unwrap_or_else(|| ApiConfig {
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

/// 回傳 (api_key, base_url)；Base URL 若無效則退回官方預設
pub fn load_deepseek_key() -> Option<(String, String)> {
    let s = read_file();
    let key = s.deepseek_api_key.trim().to_string();
    if key.is_empty() {
        return None;
    }
    let base = validate_api_base_url(&s.deepseek_base).unwrap_or_else(|_| default_base());
    Some((key, base))
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
        minimize_on_close: s.minimize_on_close,
    }
}

pub fn get_minimize_on_close() -> bool {
    read_file().minimize_on_close
}

pub fn set_minimize_on_close(v: bool) -> Result<(), String> {
    let mut s = read_file();
    s.minimize_on_close = v;
    write_file(&s)
}
