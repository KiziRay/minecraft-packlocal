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
    /// 自訂服務商預設：deepseek、glm、openai、qwen 或 other。
    pub provider: String,
    /// 只有「其他 OpenAI 相容服務」才回傳自訂模型名稱。
    pub model: String,
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
    /// API 服務商。留空時由舊設定的 Base URL 自動推斷，維持舊版相容性。
    #[serde(default)]
    api_provider: String,
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
            api_provider: String::new(),
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
    "deepseek-v4-flash".into()
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

const PROVIDER_DEEPSEEK: &str = "deepseek";
const PROVIDER_GLM: &str = "glm";
const PROVIDER_OPENAI: &str = "openai";
const PROVIDER_QWEN: &str = "qwen";
const PROVIDER_OTHER: &str = "other";

fn normalize_provider(provider: &str) -> Option<&'static str> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "deepseek" | "deep-seek" => Some(PROVIDER_DEEPSEEK),
        "glm" | "zhipu" | "zhipuai" | "智譜" | "智谱" => Some(PROVIDER_GLM),
        "openai" => Some(PROVIDER_OPENAI),
        "qwen" | "dashscope" | "通義" | "通义" => Some(PROVIDER_QWEN),
        "other" | "custom" | "openai-compatible" | "openai_compatible" => Some(PROVIDER_OTHER),
        _ => None,
    }
}

fn infer_provider(base_url: &str) -> &'static str {
    let lower = base_url.trim().to_ascii_lowercase();
    if lower.contains("api.deepseek.com") {
        PROVIDER_DEEPSEEK
    } else if lower.contains("open.bigmodel.cn") {
        PROVIDER_GLM
    } else if lower.contains("api.openai.com") {
        PROVIDER_OPENAI
    } else if lower.contains("dashscope.aliyuncs.com") {
        PROVIDER_QWEN
    } else {
        PROVIDER_OTHER
    }
}

fn current_provider(s: &SecretsFile) -> &'static str {
    normalize_provider(&s.api_provider).unwrap_or_else(|| infer_provider(&s.deepseek_base))
}

fn preset(provider: &str) -> Option<(&'static str, &'static str)> {
    match provider {
        PROVIDER_DEEPSEEK => Some(("https://api.deepseek.com", "deepseek-v4-flash")),
        PROVIDER_GLM => Some(("https://open.bigmodel.cn/api/paas/v4", "glm-5.2")),
        PROVIDER_OPENAI => Some(("https://api.openai.com/v1", "gpt-5-mini")),
        PROVIDER_QWEN => Some((
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "qwen-plus",
        )),
        PROVIDER_OTHER => None,
        _ => None,
    }
}

fn validate_model(model: &str) -> Result<String, String> {
    let value = model.trim();
    if value.is_empty() {
        return Err("請填寫模型名稱。".into());
    }
    if value.len() > 160 || value.chars().any(|c| c == '\0' || c == '\r' || c == '\n') {
        return Err("模型名稱格式不正確或太長。".into());
    }
    Ok(value.to_string())
}

/// 建立 OpenAI 相容的聊天完成端點。
///
/// DeepSeek 與多數 OpenAI 相容服務使用 `/v1/chat/completions`，
/// 智譜 GLM 的官方 Base URL 已包含 `/v4`，因此端點是 `/chat/completions`。
/// 這只是端點接線，不會改變翻譯 prompt 或結果處理。
pub fn api_chat_completions_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let lower = base.to_ascii_lowercase();
    if lower.ends_with("/chat/completions") {
        base.to_string()
    } else if lower.ends_with("/v1")
        || lower.ends_with("/api/paas/v4")
        || lower.contains("open.bigmodel.cn/api/paas/v4")
    {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
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
    let provider = current_provider(&s);
    let (base_url, model) = if let Some((base, model)) = preset(provider) {
        (base.to_string(), model.to_string())
    } else {
        let base_url = validate_api_base_url(&s.deepseek_base).ok()?;
        let model = validate_model(&s.model).ok()?;
        (base_url, model)
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
    let current = read_file();
    let provider = current_provider(&current).to_string();
    let model = current.model;
    save_api_settings_with_provider(api_key, base_url, &provider, &model)
}

/// 儲存自訂 API。常見服務商只需要金鑰；只有「其他」需要額外填端點與模型。
pub fn save_api_settings_with_provider(
    api_key: &str,
    base_url: &str,
    provider: &str,
    model: &str,
) -> Result<(), String> {
    validate_api_key(api_key)?;
    let provider = normalize_provider(provider)
        .ok_or_else(|| "不支援的 API 服務商，請重新選擇。".to_string())?;
    let mut s = read_file();
    let key = api_key.trim();
    if !key.is_empty() {
        s.deepseek_api_key = key.to_string();
    }
    // 若原本就沒有 key，且這次也沒填 → 錯誤
    if s.deepseek_api_key.trim().is_empty() {
        return Err("請填入 API 金鑰（空白不會刪除已儲存的金鑰，但目前尚未有任何金鑰）。".into());
    }
    s.api_provider = provider.to_string();
    if let Some((base, preset_model)) = preset(provider) {
        s.deepseek_base = base.to_string();
        s.model = preset_model.to_string();
    } else {
        let bu = base_url.trim();
        if bu.is_empty() {
            return Err("選擇其他服務時，請填寫 Base URL。".into());
        }
        s.deepseek_base = validate_api_base_url(bu)?;
        s.model = validate_model(model)?;
    }
    write_file(&s)
}

pub fn get_api_settings_public() -> ApiSettingsPublic {
    let s = read_file();
    let key = s.deepseek_api_key.trim();
    let provider = current_provider(&s);
    let is_other = provider == PROVIDER_OTHER;
    ApiSettingsPublic {
        has_key: !key.is_empty(),
        // 只回傳固定長度井字號，不回傳實際金鑰或實際長度。
        key_masked: if key.is_empty() {
            String::new()
        } else {
            "########".to_string()
        },
        base_url: if is_other {
            s.deepseek_base.trim().to_string()
        } else {
            String::new()
        },
        provider: provider.to_string(),
        model: if is_other {
            s.model.trim().to_string()
        } else {
            String::new()
        },
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
    use super::{
        api_chat_completions_url, infer_provider, normalize_ai_mode, normalize_provider, preset,
        validate_model,
    };

    #[test]
    fn ai_mode_accepts_only_known_sources() {
        assert_eq!(normalize_ai_mode(" managed "), Some("managed"));
        assert_eq!(normalize_ai_mode("CUSTOM"), Some("custom"));
        assert_eq!(normalize_ai_mode("legacy"), None);
    }

    #[test]
    fn provider_presets_use_supported_endpoints() {
        assert_eq!(preset("deepseek"), Some(("https://api.deepseek.com", "deepseek-v4-flash")));
        assert_eq!(preset("glm"), Some(("https://open.bigmodel.cn/api/paas/v4", "glm-5.2")));
        assert_eq!(preset("openai"), Some(("https://api.openai.com/v1", "gpt-5-mini")));
        assert_eq!(preset("qwen"), Some((
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "qwen-plus"
        )));
        assert_eq!(preset("other"), None);
    }

    #[test]
    fn provider_names_and_legacy_urls_are_normalized() {
        assert_eq!(normalize_provider("zhipuai"), Some("glm"));
        assert_eq!(normalize_provider("dashscope"), Some("qwen"));
        assert_eq!(infer_provider("https://api.deepseek.com"), "deepseek");
        assert_eq!(infer_provider("https://open.bigmodel.cn/api/paas/v4"), "glm");
        assert_eq!(infer_provider("https://api.openai.com/v1"), "openai");
        assert_eq!(infer_provider("https://dashscope.aliyuncs.com/compatible-mode/v1"), "qwen");
        assert_eq!(infer_provider("https://example.com/v1"), "other");
    }

    #[test]
    fn model_name_rejects_empty_and_newlines() {
        assert!(validate_model(" ").is_err());
        assert!(validate_model("glm-5.2").is_ok());
        assert!(validate_model("bad\nmodel").is_err());
    }

    #[test]
    fn chat_endpoint_matches_provider_base_url_shape() {
        assert_eq!(
            api_chat_completions_url("https://api.deepseek.com"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            api_chat_completions_url("https://open.bigmodel.cn/api/paas/v4/"),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
        assert_eq!(
            api_chat_completions_url("https://example.com/v1"),
            "https://example.com/v1/chat/completions"
        );
        assert_eq!(
            api_chat_completions_url("https://example.com/v1/chat/completions"),
            "https://example.com/v1/chat/completions"
        );
    }
}
