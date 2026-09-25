use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use super::codex_auth::has_saved_gpt_login;
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
    /// GPT OAuth 模式的模型選擇。
    pub gpt_model: String,
    /// gpt＝GPT OAuth；custom＝使用者自備 API。舊值 managed 會遷移。
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
    /// GPT OAuth 模式使用的 Codex 模型。
    #[serde(default = "default_gpt_model")]
    gpt_model: String,
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
            gpt_model: default_gpt_model(),
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

/// 新安裝的預設 AI 來源。
///
/// 曾經是 `local`：新使用者選完資料夾按下第一顆按鈕，換來的是「請先登入 Discord」
/// 加上數 GB 下載——最貴的一條路被放在最前面。改為 `custom` 後，沒設定金鑰時
/// 前端會直接提供「先用基本翻譯跑一次」的出口，使用者永遠有下一步可走。
fn default_ai_mode() -> String {
    "custom".into()
}

fn default_gpt_model() -> String {
    "gpt-5.6-luna".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiProvider {
    Codex,
    Deepseek,
    Glm,
    Openai,
    Qwen,
    Other,
    LocalLlm,
}

impl AiProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Deepseek => "deepseek",
            Self::Glm => "glm",
            Self::Openai => "openai",
            Self::Qwen => "qwen",
            Self::Other => "other",
            Self::LocalLlm => "local",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxTokensField {
    MaxTokens,
    MaxCompletionTokens,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderRateTable {
    pub prompt_cache_hit_per_mtok_usd: Option<f64>,
    pub prompt_cache_miss_per_mtok_usd: Option<f64>,
    pub completion_per_mtok_usd: Option<f64>,
}

impl ProviderRateTable {
    pub fn estimate_usd(
        self,
        prompt_cache_hit_tokens: usize,
        prompt_cache_miss_tokens: usize,
        completion_tokens: usize,
    ) -> Option<f64> {
        let hit = self.prompt_cache_hit_per_mtok_usd?;
        let miss = self.prompt_cache_miss_per_mtok_usd?;
        let completion = self.completion_per_mtok_usd?;
        Some(
            (prompt_cache_hit_tokens as f64 / 1_000_000.0) * hit
                + (prompt_cache_miss_tokens as f64 / 1_000_000.0) * miss
                + (completion_tokens as f64 / 1_000_000.0) * completion,
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderCapabilities {
    pub supports_json_mode: bool,
    pub max_tokens_field: MaxTokensField,
    pub start_parallel: usize,
    pub rates: ProviderRateTable,
}

impl ProviderCapabilities {
    pub fn estimated_cost_usd(
        self,
        prompt_cache_hit_tokens: usize,
        prompt_cache_miss_tokens: usize,
        completion_tokens: usize,
    ) -> Option<f64> {
        self.rates.estimate_usd(
            prompt_cache_hit_tokens,
            prompt_cache_miss_tokens,
            completion_tokens,
        )
    }

    pub fn cost_note(
        self,
        provider: AiProvider,
        prompt_cache_hit_tokens: usize,
        prompt_cache_miss_tokens: usize,
        completion_tokens: usize,
    ) -> Option<String> {
        let usd = self.estimated_cost_usd(
            prompt_cache_hit_tokens,
            prompt_cache_miss_tokens,
            completion_tokens,
        )?;
        Some(format!("AI token 成本估算（{}）：US${usd:.4}", provider.label()))
    }
}

fn normalize_ai_mode(mode: &str) -> Option<&'static str> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "gpt" => Some("gpt"),
        "custom" => Some("custom"),
        "local" => Some("local"),
        _ => None,
    }
}

/// 舊版 `managed`：有自訂金鑰→custom，否則 local。未知值→local。
pub fn migrate_legacy_ai_mode(mode: &str, has_key: bool) -> &'static str {
    match normalize_ai_mode(mode) {
        Some(m) => m,
        None if mode.trim().eq_ignore_ascii_case("managed") => {
            if has_key {
                "custom"
            } else {
                "local"
            }
        }
        None => "local",
    }
}

fn effective_ai_mode(s: &SecretsFile) -> &'static str {
    migrate_legacy_ai_mode(&s.ai_mode, !s.deepseek_api_key.trim().is_empty())
}

const GPT_MODEL_LUNA: &str = "gpt-5.6-luna";
/// ChatGPT 帳號能用的 GPT 模型——**只有 Luna 一個**。
///
/// 實測 400：`{"detail":"The 'gpt-5.6-sol' model is not supported when using
/// Codex with a ChatGPT account."}`。Terra／Sol／5.4／5.5 都是同樣結果，
/// 讓使用者選只會讓他們在翻到一半時失敗，所以清單縮成一個、UI 也拿掉選單。
///
/// 舊設定檔裡存著 sol／terra 的人不必做任何事：`normalize_gpt_model` 現在
/// 認不得它們回 `None`，`get_gpt_model()` 的 `.unwrap_or(GPT_MODEL_LUNA)`
/// 會自動換成 Luna。
const GPT_MODEL_ALLOWLIST: &[&str] = &[GPT_MODEL_LUNA];

fn normalize_gpt_model(model: &str) -> Option<&'static str> {
    match model.trim().to_ascii_lowercase().as_str() {
        GPT_MODEL_LUNA => Some(GPT_MODEL_LUNA),
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

fn provider_kind(provider: &str) -> AiProvider {
    match provider {
        PROVIDER_DEEPSEEK => AiProvider::Deepseek,
        PROVIDER_GLM => AiProvider::Glm,
        PROVIDER_OPENAI => AiProvider::Openai,
        PROVIDER_QWEN => AiProvider::Qwen,
        _ => AiProvider::Other,
    }
}

pub fn provider_capabilities(provider: AiProvider) -> ProviderCapabilities {
    match provider {
        AiProvider::Codex => ProviderCapabilities {
            supports_json_mode: false,
            max_tokens_field: MaxTokensField::MaxTokens,
            start_parallel: 8,
            rates: ProviderRateTable::default(),
        },
        AiProvider::Deepseek => ProviderCapabilities {
            supports_json_mode: true,
            max_tokens_field: MaxTokensField::MaxTokens,
            start_parallel: 16,
            rates: ProviderRateTable::default(),
        },
        AiProvider::Openai => ProviderCapabilities {
            supports_json_mode: true,
            max_tokens_field: MaxTokensField::MaxCompletionTokens,
            start_parallel: 16,
            rates: ProviderRateTable::default(),
        },
        AiProvider::Glm => ProviderCapabilities {
            supports_json_mode: true,
            max_tokens_field: MaxTokensField::MaxTokens,
            start_parallel: 16,
            rates: ProviderRateTable::default(),
        },
        AiProvider::Qwen => ProviderCapabilities {
            supports_json_mode: true,
            max_tokens_field: MaxTokensField::MaxTokens,
            start_parallel: 16,
            rates: ProviderRateTable::default(),
        },
        AiProvider::Other => ProviderCapabilities {
            supports_json_mode: false,
            max_tokens_field: MaxTokensField::MaxTokens,
            start_parallel: 16,
            rates: ProviderRateTable::default(),
        },
        AiProvider::LocalLlm => ProviderCapabilities {
            supports_json_mode: false,
            max_tokens_field: MaxTokensField::MaxTokens,
            // 送出端的併發要跟 llama-server 開的 slot 數一致，否則多開的 slot
            // 沒人用（還是序列），或送太多請求排隊反而更慢。
            // 兩邊都由 `local_llm::server::parallel_slots_for()` 依硬體換算。
            start_parallel: crate::engine::local_llm::recommended_parallel_slots() as usize,
            rates: ProviderRateTable::default(),
        },
    }
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

/// Cloudflare Worker 公開 URL（更新、共享 TM、診斷回報）。不是 AI 金鑰。
pub const MANAGED_BASE_URL: &str = "https://modpack-i18n.jolin34563.workers.dev";

/// AI 連線設定（金鑰 + 端點 + 模型）。
#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub provider: AiProvider,
    pub capabilities: ProviderCapabilities,
}

/// 讀取「使用者自填」的金鑰設定；沒填回 `None`。
///
/// 用於判斷 UI 上的「金鑰：已設定／未設定」與是否直連上游。代管模式不算「自填」。
pub fn load_api_config() -> Option<ApiConfig> {
    let s = read_file();
    // 記憶體優先：使用者選了「不記住金鑰」時，設定檔裡本來就是空的
    let api_key = effective_api_key(&s.deepseek_api_key);
    if api_key.is_empty() {
        return None;
    }
    let provider_name = current_provider(&s);
    let provider = provider_kind(provider_name);
    let (base_url, model) = if let Some((base, model)) = preset(provider_name) {
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
        provider,
        capabilities: provider_capabilities(provider),
    })
}

/// 本地模型處理不了的句子，可以改用哪個雲端端點補完？
///
/// 這是「本地翻譯品質要跟雲端一樣」的實作基礎。小模型的**能力**不可能等於大模型，
/// 但**交付出去的結果**可以一樣——把小模型過不了品質關的那些句子，改用雲端翻一次即可。
///
/// 與 `resolve_ai_config` 的差別：那個看「使用者現在選了什麼模式」，
/// 這個看「有沒有任何一條雲端路可走」，不管目前是不是本地模式。
/// 兩條都沒設就回 `None`——沒有免費代管可用（`managed_ai_available()` 已固定為 false）。
/// 玩家有沒有同意「本地翻不好時改用線上 AI 補完」。
///
/// 預設**開啟**——不開的話品質就不會跟線上 AI 一樣，那正是站長要求的目標。
/// 但這件事會花到玩家自己的 API 用量，所以設定裡一定看得到、關得掉。
pub fn cloud_topup_enabled() -> bool {
    let raw = super::app_settings::read_settings()
        .get("translate")
        .and_then(|t| t.get("localCloudTopUp"))
        .cloned();
    cloud_topup_from_value(raw)
}

/// 使用者對「本地不夠好時改用雲端補量」的選擇。
///
/// 「還沒選過」必須跟「選了關」分開（P0-05）：前者要跳一次明確的同意，
/// 後者要安靜地維持關閉。兩者混成同一個布林，就沒辦法只問一次。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudTopUpChoice {
    Enabled,
    Disabled,
    /// 從來沒問過這位使用者。行為等同關閉，但 UI 應該主動徵詢一次。
    NotChosen,
}

impl CloudTopUpChoice {
    pub fn is_enabled(self) -> bool {
        matches!(self, CloudTopUpChoice::Enabled)
    }
    pub fn needs_consent_prompt(self) -> bool {
        matches!(self, CloudTopUpChoice::NotChosen)
    }
}

/// 純判斷版本，供測試。見 [`cloud_topup_enabled`] 的說明。
fn cloud_topup_from_value(raw: Option<serde_json::Value>) -> bool {
    cloud_topup_choice_from_value(raw).is_enabled()
}

fn cloud_topup_choice_from_value(raw: Option<serde_json::Value>) -> CloudTopUpChoice {
    match raw {
        // 前端的 setSetting 一律把值轉成字串再存，所以這裡兩種型別都要吃。
        // 只認 bool 的話，玩家把開關關掉會完全沒有作用——而這個開關關係到
        // 會不會用掉他自己的 API 用量，失效是不能接受的。
        Some(serde_json::Value::Bool(true)) => CloudTopUpChoice::Enabled,
        Some(serde_json::Value::Bool(false)) => CloudTopUpChoice::Disabled,
        Some(serde_json::Value::String(s)) => {
            let v = s.trim().to_ascii_lowercase();
            if v.is_empty() {
                CloudTopUpChoice::NotChosen
            } else if matches!(v.as_str(), "0" | "false" | "off" | "no") {
                CloudTopUpChoice::Disabled
            } else {
                CloudTopUpChoice::Enabled
            }
        }
        // 沒設定過＝**預設關閉**（P0-05）。
        //
        // 舊版這裡是 `true`：使用者選了「本地模型」——一個明確表示不想把文字送上網、
        // 也不想付費的選擇——工具卻會在本地翻不好時自動改打雲端 API，用掉他自己的額度。
        // 「品質才會跟線上一樣」不足以正當化未經同意的資料外送與花費。
        // 要用可以，但必須是他自己開的。
        _ => CloudTopUpChoice::NotChosen,
    }
}

/// 使用者目前的選擇（含「還沒問過」）。供 UI 決定要不要跳同意視窗。
pub fn cloud_topup_choice() -> CloudTopUpChoice {
    let raw = super::app_settings::read_settings()
        .get("translate")
        .and_then(|t| t.get("localCloudTopUp"))
        .cloned();
    cloud_topup_choice_from_value(raw)
}

pub fn resolve_cloud_fallback_config() -> Option<ApiConfig> {
    if !cloud_topup_enabled() {
        return None;
    }
    // 自填金鑰優先：使用者自己付錢的通道，額度與模型都由他掌握
    if let Some(cfg) = load_api_config() {
        return Some(cfg);
    }
    if has_saved_gpt_login() {
        return Some(ApiConfig {
            api_key: String::new(),
            base_url: String::new(),
            model: get_gpt_model(),
            provider: AiProvider::Codex,
            capabilities: provider_capabilities(AiProvider::Codex),
        });
    }
    None
}

/// 決定這次翻譯要用哪個 AI 端點（自訂金鑰直連，或 GPT OAuth）。
pub fn resolve_ai_config() -> Result<ApiConfig, String> {
    match get_ai_mode().as_str() {
        "custom" => load_api_config().ok_or_else(|| {
            "已選擇自訂 API，但尚未儲存 API 金鑰。請回到工作台填寫。".into()
        }),
        "gpt" => {
            if !has_saved_gpt_login() {
                return Err("已選 GPT，請先登入 GPT 帳號。".into());
            }
            Ok(ApiConfig {
                api_key: String::new(),
                base_url: String::new(),
                model: get_gpt_model(),
                provider: AiProvider::Codex,
                capabilities: provider_capabilities(AiProvider::Codex),
            })
        }
        "local" => {
            crate::engine::local_llm::ensure_ready_for_translate(None)?;
            let base_url = crate::engine::local_llm::active_chat_base_url()?;
            Ok(ApiConfig {
                // 這把 key 必須跟 server.rs 啟動 llama-server 時傳的 --api-key 完全一致，
                // 否則系統上任何其他工具設的 LLAMA_API_KEY 環境變數會被撿到，
                // 所有翻譯請求都會被拒絕、但 /health 不受影響——看起來像「裝好了但翻不了」。
                api_key: crate::engine::local_llm::LOCAL_LLM_API_KEY.into(),
                base_url,
                model: "local".into(),
                provider: AiProvider::LocalLlm,
                capabilities: provider_capabilities(AiProvider::LocalLlm),
            })
        }
        _ => Err("請選擇自訂 API、GPT 或本地模型。".into()),
    }
}

fn secrets_path() -> PathBuf {
    super::paths::resolve_file(Path::new("secrets.json"))
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
    fs::write(&p, serde_json::to_string_pretty(s).unwrap() + "\n").map_err(|e| e.to_string())?;
    harden_secrets_file_acl(&p);
    Ok(())
}

/// 盡力把 secrets 檔限縮成目前使用者可讀寫（Windows icacls；失敗不阻寫入）。
fn harden_secrets_file_acl(path: &std::path::Path) {
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

/// 「這次執行才有效」的 API 金鑰。
///
/// 使用者要求金鑰不要落地：預設情況下金鑰只放在這裡（記憶體），工具一關就沒了，
/// 下次要重新輸入。只有使用者明確選擇「記住金鑰」時才會寫進設定檔。
///
/// 讀取順序永遠是「記憶體優先、設定檔其次」——這樣既有使用者升級後仍然能用
/// 原本存好的金鑰，新輸入的金鑰也不會被舊值蓋掉。
static SESSION_API_KEY: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// 只存在記憶體的金鑰。回傳前一個值（極少用到，主要方便測試）。
pub fn set_session_api_key(key: Option<String>) {
    if let Ok(mut guard) = SESSION_API_KEY.lock() {
        *guard = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    }
}

fn session_api_key() -> Option<String> {
    SESSION_API_KEY.lock().ok().and_then(|g| g.clone())
}

/// 取得目前可用的金鑰：記憶體優先，其次才是設定檔存的。
pub fn effective_api_key(stored: &str) -> String {
    session_api_key().unwrap_or_else(|| stored.trim().to_string())
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
    save_api_settings_full(api_key, base_url, provider, model, remember_api_key())
}

/// 是否把金鑰寫進設定檔。預設 **false**（不落地），由前端設定覆寫。
static REMEMBER_API_KEY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn set_remember_api_key(on: bool) {
    REMEMBER_API_KEY.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn remember_api_key() -> bool {
    REMEMBER_API_KEY.load(std::sync::atomic::Ordering::Relaxed)
}

/// 清除自訂 API 金鑰：記憶體裡這一輪的金鑰與設定檔裡記住的金鑰都拿掉。
/// 服務商、端點、模型等其他設定保留，下次只要重新貼上金鑰。
pub fn clear_api_key() -> Result<(), String> {
    set_session_api_key(None);
    let mut s = read_file();
    if s.deepseek_api_key.trim().is_empty() {
        return Ok(());
    }
    s.deepseek_api_key = String::new();
    write_file(&s)
}

pub fn save_api_settings_full(
    api_key: &str,
    base_url: &str,
    provider: &str,
    model: &str,
    remember: bool,
) -> Result<(), String> {
    validate_api_key(api_key)?;
    let provider = normalize_provider(provider)
        .ok_or_else(|| "不支援的 API 服務商，請重新選擇。".to_string())?;
    let mut s = read_file();
    let key = api_key.trim();
    if !key.is_empty() {
        // 金鑰一律先放進記憶體：這一輪翻譯馬上就要用
        set_session_api_key(Some(key.to_string()));
        if remember {
            s.deepseek_api_key = key.to_string();
        } else {
            // 不記住：確保設定檔裡不留舊金鑰（使用者可能是從「記住」改成「不記住」）
            s.deepseek_api_key = String::new();
        }
    }
    // 若記憶體與設定檔都沒有 key → 錯誤
    if session_api_key().is_none() && s.deepseek_api_key.trim().is_empty() {
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
    // 遮罩顯示也要看記憶體那把：不記住金鑰時設定檔是空的，但這一輪確實可用
    let effective = effective_api_key(&s.deepseek_api_key);
    let key = effective.as_str();
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
        gpt_model: get_gpt_model(),
        ai_mode: effective_ai_mode(&s).to_string(),
        minimize_on_close: s.minimize_on_close,
    }
}

pub fn get_ai_mode() -> String {
    let mut s = read_file();
    let mode = effective_ai_mode(&s).to_string();
    if s.ai_mode != mode {
        s.ai_mode = mode.clone();
        let _ = write_file(&s);
    }
    mode
}

pub fn set_ai_mode(mode: &str) -> Result<String, String> {
    let normalized = normalize_ai_mode(mode)
        .ok_or_else(|| "AI 來源只能選擇 GPT、自訂 API 或本地模型。".to_string())?;
    let mut settings = read_file();
    settings.ai_mode = normalized.to_string();
    write_file(&settings)?;
    Ok(normalized.to_string())
}

pub fn get_gpt_model() -> String {
    normalize_gpt_model(&read_file().gpt_model)
        .unwrap_or(GPT_MODEL_LUNA)
        .to_string()
}

pub fn set_gpt_model(model: &str) -> Result<String, String> {
    let normalized = normalize_gpt_model(model).ok_or_else(|| {
        format!(
            "GPT 模型只能選擇：{}。",
            GPT_MODEL_ALLOWLIST.join("、")
        )
    })?;
    let mut settings = read_file();
    settings.gpt_model = normalized.to_string();
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
        api_chat_completions_url, cloud_topup_choice_from_value, cloud_topup_from_value,
        CloudTopUpChoice, infer_provider, migrate_legacy_ai_mode,
        normalize_ai_mode, normalize_gpt_model, normalize_provider, preset, provider_capabilities,
        validate_model, AiProvider, MaxTokensField,
    };
    use serde_json::json;

    #[test]
    fn turning_off_cloud_top_up_actually_works_whatever_type_it_was_stored_as() {
        // 這個開關關係到「會不會用掉玩家自己的 API 用量」，失效是不能接受的。
        // 前端的 setSetting 一律把值轉成字串再存，所以只認 bool 的話，
        // 玩家把它關掉會完全沒有作用——錢照花，而且他不會知道。
        assert!(!cloud_topup_from_value(Some(json!("0"))), "字串 \"0\" 要算關");
        assert!(!cloud_topup_from_value(Some(json!(false))), "布林 false 要算關");
        assert!(!cloud_topup_from_value(Some(json!("false"))));
        assert!(!cloud_topup_from_value(Some(json!("off"))));

        assert!(cloud_topup_from_value(Some(json!("1"))));
        assert!(cloud_topup_from_value(Some(json!(true))));
    }

    #[test]
    fn cloud_top_up_is_off_until_the_user_actually_says_yes() {
        // P0-05：這個開關決定「會不會把整合包文字送上網並花掉使用者自己的 API 額度」。
        // 沒問過就不能當成同意——舊版預設 true，等於選了本地模型的人也會被靜默送上雲端。
        assert!(!cloud_topup_from_value(None), "沒設定過必須是關");
        assert!(!cloud_topup_from_value(Some(json!(null))));
        assert!(!cloud_topup_from_value(Some(json!(""))));

        // 「還沒問過」與「問過而且選了關」要分得出來，才知道該不該跳同意
        assert_eq!(cloud_topup_choice_from_value(None), CloudTopUpChoice::NotChosen);
        assert_eq!(cloud_topup_choice_from_value(Some(json!(""))), CloudTopUpChoice::NotChosen);
        assert_eq!(
            cloud_topup_choice_from_value(Some(json!(false))),
            CloudTopUpChoice::Disabled
        );
        assert_eq!(
            cloud_topup_choice_from_value(Some(json!("off"))),
            CloudTopUpChoice::Disabled
        );
        assert_eq!(cloud_topup_choice_from_value(Some(json!(true))), CloudTopUpChoice::Enabled);
        assert_eq!(cloud_topup_choice_from_value(Some(json!("1"))), CloudTopUpChoice::Enabled);

        // 只有「還沒問過」該跳同意；已經選了關的人不該被反覆騷擾
        assert!(CloudTopUpChoice::NotChosen.needs_consent_prompt());
        assert!(!CloudTopUpChoice::Disabled.needs_consent_prompt());
        assert!(!CloudTopUpChoice::Enabled.needs_consent_prompt());

        // 只有 Enabled 才真的會送上雲端
        assert!(CloudTopUpChoice::Enabled.is_enabled());
        assert!(!CloudTopUpChoice::Disabled.is_enabled());
        assert!(!CloudTopUpChoice::NotChosen.is_enabled());
    }

    #[test]
    fn ai_mode_accepts_only_known_sources() {
        assert_eq!(normalize_ai_mode("GPT"), Some("gpt"));
        assert_eq!(normalize_ai_mode("CUSTOM"), Some("custom"));
        assert_eq!(normalize_ai_mode("LOCAL"), Some("local"));
        assert_eq!(normalize_ai_mode("managed"), None);
        assert_eq!(normalize_ai_mode("legacy"), None);
    }

    #[test]
    fn migrate_legacy_managed_by_key() {
        assert_eq!(migrate_legacy_ai_mode("managed", true), "custom");
        assert_eq!(migrate_legacy_ai_mode("managed", false), "local");
        assert_eq!(migrate_legacy_ai_mode("custom", false), "custom");
        assert_eq!(migrate_legacy_ai_mode("weird", false), "local");
    }

    #[test]
    fn gpt_model_accepts_only_allowlisted_values() {
        assert_eq!(normalize_gpt_model("gpt-5.6-luna"), Some("gpt-5.6-luna"));
        assert_eq!(normalize_gpt_model(" GPT-5.6-LUNA "), Some("gpt-5.6-luna"));
        // ChatGPT 帳號只支援 Luna，其餘一律不認——舊設定檔存著 sol 的人會被
        // get_gpt_model() 的 unwrap_or 自動換成 Luna，不必自己去改設定。
        assert_eq!(normalize_gpt_model("gpt-5.6-sol"), None);
        assert_eq!(normalize_gpt_model("gpt-5.6-terra"), None);
        assert_eq!(normalize_gpt_model("gpt-5.4"), None);
        assert_eq!(normalize_gpt_model("gpt-4.1"), None);
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

    #[test]
    fn capability_table_uses_expected_parallel_caps() {
        assert_eq!(provider_capabilities(AiProvider::Codex).start_parallel, 8);
        assert_eq!(provider_capabilities(AiProvider::Deepseek).start_parallel, 16);
        assert_eq!(provider_capabilities(AiProvider::Other).start_parallel, 16);
        // 本地模型的併發依這台電腦的硬體換算（見 local_llm::parallel_slots_for），
        // 不是固定值；這裡只釘住「保守範圍」與「跟伺服器端一致」。
        let local = provider_capabilities(AiProvider::LocalLlm).start_parallel;
        assert!((1..=3).contains(&local), "本地模型併發要保守：實得 {local}");
        assert_eq!(
            local,
            crate::engine::local_llm::recommended_parallel_slots() as usize
        );
    }

    #[test]
    fn capability_table_uses_provider_specific_token_fields() {
        assert_eq!(
            provider_capabilities(AiProvider::Openai).max_tokens_field,
            MaxTokensField::MaxCompletionTokens
        );
        assert_eq!(
            provider_capabilities(AiProvider::Deepseek).max_tokens_field,
            MaxTokensField::MaxTokens
        );
    }
}
