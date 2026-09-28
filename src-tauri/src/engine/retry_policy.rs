//! B4：AI 請求的重試政策。
//!
//! 一個請求失敗之後只有四種正確反應，選錯就會「空轉」或「丟譯文」：
//! - **等一下再送同一批**：限流、伺服器忙、網路抖動、逾時。等多久先看伺服器給的
//!   `Retry-After`，沒給才用指數退避（有上限、有抖動，避免多條並行同時撞回去）。
//! - **拆小再送**：請求太長（超過模型上下文）。同一批再送一百次結果都一樣。
//! - **停下 AI、保留已翻部分**：額度用完、金鑰無效。重試只會空轉，還可能繼續扣錢。
//! - **其他**：交給既有的重排流程。
//!
//! 判斷只看「當下這個錯誤」的狀態碼與內容，不猜。

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// 暫時性錯誤：同一批最多再送幾次（不含第一次）。
pub const MAX_TRANSIENT_RETRIES: usize = 3;
/// 指數退避的起點與上限（測試縮短，不改邏輯）。
pub const BACKOFF_BASE_MS: u64 = if cfg!(test) { 20 } else { 800 };
pub const BACKOFF_CAP_MS: u64 = if cfg!(test) { 200 } else { 30_000 };
/// 伺服器要求等太久時的上限：超過就不在這批裡乾等，讓這批先失敗、稍後重排。
pub const RETRY_AFTER_CAP: Duration = Duration::from_secs(120);

/// 一次失敗屬於哪一類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// 請求太頻繁（沒有額度字樣的 429）
    RateLimited,
    /// 伺服器忙或暫時故障（5xx、408、overloaded）
    ServerBusy,
    /// 連不上、連線被切斷
    Network,
    /// 等回應逾時
    Timeout,
    /// 額度、餘額、用量上限——重試只會空轉
    QuotaExhausted,
    /// 金鑰無效、沒有權限——重試只會空轉
    AuthInvalid,
    /// 請求超過模型能處理的長度——要拆小
    TooLarge,
    /// 其他（格式錯誤、看不懂的狀態碼）
    Other,
}

impl FailureClass {
    /// 等一下再送同一批會有用嗎？
    pub fn retry_same_batch(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::ServerBusy | Self::Network | Self::Timeout
        )
    }

    /// 要拆小才會好嗎？
    pub fn should_split(self) -> bool {
        matches!(self, Self::TooLarge)
    }

    /// 這一類錯誤要讓 AI 停下（保留已翻部分）嗎？
    pub fn stops_ai(self) -> bool {
        matches!(self, Self::QuotaExhausted | Self::AuthInvalid)
    }

    /// 給日誌看的白話分類名。
    pub fn label_zh(self) -> &'static str {
        match self {
            Self::RateLimited => "請求太頻繁",
            Self::ServerBusy => "伺服器忙碌",
            Self::Network => "連線中斷",
            Self::Timeout => "等待逾時",
            Self::QuotaExhausted => "額度用完",
            Self::AuthInvalid => "金鑰無效或沒有權限",
            Self::TooLarge => "內容太長",
            Self::Other => "其他錯誤",
        }
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// 「內容太長」的字樣。要排在額度判斷前面：`maximum context length exceeded`
/// 裡的 `exceed` 舊版會被當成額度用完，整輪 AI 就被停掉了。
fn says_too_large(lower: &str) -> bool {
    contains_any(
        lower,
        &[
            "context_length_exceeded",
            "maximum context length",
            "context length",
            "context window",
            "context size",
            "too many tokens",
            "request too large",
            "request entity too large",
            "prompt is too long",
            "input is too long",
            "reduce the length",
            "max_tokens is too large",
            "內容太長",
        ],
    )
}

fn says_quota(lower: &str) -> bool {
    contains_any(
        lower,
        &[
            "insufficient_quota",
            "insufficient quota",
            "insufficient balance",
            "insufficient_balance",
            "exceeded your current quota",
            "quota",
            // 不列 billing：Gemini／OpenAI 的每分鐘限流內文也會附「plan and billing details」連結
            // 不用裸的 balance：`load balancer` 的 502 會被誤判成額度用完
            "balance is",
            "balance too low",
            "credit balance",
            "out of credits",
            "no credits",
            "payment required",
            "usage limit",
            "usage_limit",
            "沒有額度",
            "額度",
            "餘額",
            "余额",
        ],
    )
}

fn says_auth(lower: &str) -> bool {
    contains_any(
        lower,
        &[
            "invalid api key",
            "invalid_api_key",
            "incorrect api key",
            "invalid authentication",
            "unauthorized",
            "permission denied",
            "金鑰無效",
            "無權限",
        ],
    )
}

fn says_rate_limit(lower: &str) -> bool {
    contains_any(
        lower,
        &["rate limit", "rate_limit", "too many requests", "請求太頻繁", "slow down"],
    )
}

fn says_busy(lower: &str) -> bool {
    contains_any(
        lower,
        &[
            "overloaded",
            "server_error",
            "service unavailable",
            "bad gateway",
            "gateway timeout",
            "暫時無法使用",
            // 「服務錯誤 N」不列：400 也叫服務錯誤，5xx 由 has_status 判
        ],
    )
}

/// 429 的「額度」字樣其實是每分鐘／每秒的速率限制（例如 Gemini 的
/// `Quota exceeded … per minute`）：等一下就好，不是額度用完。
/// 帳單、餘額、`insufficient_quota` 這類才是真的用完。
fn is_short_window_limit(lower: &str) -> bool {
    // 1) 真的用完（餘額、insufficient_quota）優先
    let hard = contains_any(
        lower,
        &[
            "insufficient_quota",
            "insufficient quota",
            "insufficient balance",
            "insufficient_balance",
            "余额不足",
            "餘額不足",
            "credit balance",
            "payment required",
        ],
    );
    if hard {
        return false;
    }
    // 2) 每日上限（Gemini 的 PerDay quotaId）＝今天用完，不是等一下就好
    let per_day = contains_any(lower, &["perday", "per day", "per_day", "daily"]);
    // 3) 短時間窗的限流字樣
    let short = contains_any(
        lower,
        &[
            "rate_limit_exceeded",
            "rate limit",
            "rate_limit",
            "perminute",
            "per minute",
            "per_minute",
            "per min",
            "persecond",
            "per second",
            "per_second",
            "requests per",
            "tokens per",
            "(rpm)",
            "(tpm)",
        ],
    );
    let retry_soon = contains_any(lower, &["retry in", "try again in", "retrydelay"]);
    short || (retry_soon && !per_day)
}

/// 訊息裡明確寫著的 HTTP 狀態碼（`HTTP 402`、`status 402`、`（402）`、`錯誤 402`）。
///
/// 不比對裸數字：包裝訊息「第 402 批失敗：…」裡的 402 是批次編號，舊版會被判成額度用完。
fn has_status(lower: &str, code: u16) -> bool {
    let c = code.to_string();
    [
        format!("http {c}"),
        format!("http/1.1 {c}"),
        format!("status {c}"),
        format!("status: {c}"),
        format!("status code {c}"),
        format!("（{c}）"),
        format!("({c})"),
        format!("錯誤 {c}"),
        format!("error {c}"),
        format!("code {c}"),
    ]
    .iter()
    .any(|pattern| lower.contains(pattern.as_str()))
}

/// 依 HTTP 狀態碼＋回應內容分類。
pub fn classify_http(code: u16, body: &str) -> FailureClass {
    let lower = body.to_lowercase();
    if code == 413 || says_too_large(&lower) {
        return FailureClass::TooLarge;
    }
    if code == 429 && is_short_window_limit(&lower) {
        return FailureClass::RateLimited;
    }
    if code == 402 || says_quota(&lower) {
        return FailureClass::QuotaExhausted;
    }
    if code == 401 || code == 403 || says_auth(&lower) {
        return FailureClass::AuthInvalid;
    }
    if code == 429 || says_rate_limit(&lower) {
        return FailureClass::RateLimited;
    }
    if code == 408 {
        return FailureClass::Timeout;
    }
    if code >= 500 || says_busy(&lower) {
        return FailureClass::ServerBusy;
    }
    FailureClass::Other
}

/// 只有訊息文字時的分類（GPT 錯誤、已經組好的中文訊息）。
pub fn classify_message(msg: &str) -> FailureClass {
    let lower = msg.to_lowercase();
    if says_too_large(&lower) {
        return FailureClass::TooLarge;
    }
    if has_status(&lower, 429) && is_short_window_limit(&lower) {
        return FailureClass::RateLimited;
    }
    if says_quota(&lower) || has_status(&lower, 402) {
        return FailureClass::QuotaExhausted;
    }
    if says_auth(&lower)
        || has_status(&lower, 401)
        || (has_status(&lower, 403) && (lower.contains("key") || lower.contains("forbidden")))
    {
        return FailureClass::AuthInvalid;
    }
    if says_rate_limit(&lower) || has_status(&lower, 429) {
        return FailureClass::RateLimited;
    }
    if contains_any(&lower, &["逾時", "timeout", "timed out"]) {
        return FailureClass::Timeout;
    }
    if contains_any(
        &lower,
        &["連線失敗", "無法連線", "無回應", "connection", "connect error", "broken pipe", "reset by peer"],
    ) {
        return FailureClass::Network;
    }
    if says_busy(&lower) || (500u16..=599).any(|c| has_status(&lower, c)) {
        return FailureClass::ServerBusy;
    }
    FailureClass::Other
}

/// 這個錯誤代表「重試只會空轉」嗎（額度用完、金鑰無效）？
pub fn is_quota_or_auth(msg: &str) -> bool {
    classify_message(msg).stops_ai()
}

/// 解析 `Retry-After`：秒數，或 HTTP 日期（IMF-fixdate，例如 `Wed, 21 Oct 2015 07:28:00 GMT`）。
/// 看不懂就回 `None`，交給指數退避。
pub fn parse_retry_after(value: Option<&str>, now: SystemTime) -> Option<Duration> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(secs) = raw.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    if let Ok(secs) = raw.parse::<f64>() {
        if secs.is_finite() && secs >= 0.0 {
            return Some(Duration::from_millis((secs * 1000.0) as u64));
        }
        return None;
    }
    let at = parse_http_date(raw)?;
    Some(at.duration_since(now).unwrap_or(Duration::ZERO))
}

/// `retry-after-ms`（部分 OpenAI 相容服務會給毫秒）。
pub fn parse_retry_after_ms(value: Option<&str>) -> Option<Duration> {
    let raw = value?.trim();
    raw.parse::<u64>().ok().map(Duration::from_millis)
}

/// IMF-fixdate：`Sun, 06 Nov 1994 08:49:37 GMT`。
fn parse_http_date(raw: &str) -> Option<SystemTime> {
    let parts: Vec<&str> = raw.split_whitespace().collect();
    if parts.len() != 6 || !parts[5].eq_ignore_ascii_case("GMT") {
        return None;
    }
    let day: u32 = parts[1].parse().ok()?;
    let month = match parts[2].to_ascii_lowercase().as_str() {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    };
    let year: i64 = parts[3].parse().ok()?;
    let hms: Vec<u64> = parts[4].split(':').filter_map(|p| p.parse().ok()).collect();
    if hms.len() != 3 || hms[0] > 23 || hms[1] > 59 || hms[2] > 60 || day == 0 || day > 31 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    if days < 0 {
        return None;
    }
    let secs = days as u64 * 86_400 + hms[0] * 3600 + hms[1] * 60 + hms[2];
    Some(UNIX_EPOCH + Duration::from_secs(secs))
}

/// 公曆日期 → 1970-01-01 起算的天數（Howard Hinnant 的演算法）。
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 第 `retry_index` 次重試（0 起算）前要等多久。
///
/// - 伺服器有說（`Retry-After`）就照它，但不超過 [`RETRY_AFTER_CAP`]。
/// - 沒說就指數退避：`base × 2^n`，上限 [`BACKOFF_CAP_MS`]，取「一半固定＋一半隨機」，
///   讓同時失敗的幾條並行不會在同一瞬間又一起撞回去。
pub fn backoff_delay(retry_index: usize, server_hint: Option<Duration>, jitter_seed: u64) -> Duration {
    if let Some(hint) = server_hint {
        let jitter = Duration::from_millis(jitter_seed % 250);
        return hint.min(RETRY_AFTER_CAP) + jitter;
    }
    let exp = BACKOFF_BASE_MS
        .saturating_mul(1u64 << retry_index.min(16))
        .min(BACKOFF_CAP_MS);
    let half = exp / 2;
    let jitter = if half == 0 { 0 } else { jitter_seed % (half + 1) };
    Duration::from_millis(half + jitter)
}

/// 退避用的隨機種子（不需要密碼學等級，只要每條執行緒、每次呼叫不同）。
pub fn jitter_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(0);
    let thread_bits = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::thread::current().id().hash(&mut h);
        h.finish()
    };
    nanos.wrapping_mul(6_364_136_223_846_793_005) ^ thread_bits
}

/// 模型回的 `"i"`：接受數字，也接受數字字串（`"12"`）——有些模型會把 id 加上引號，
/// 舊版一律丟掉，整批就變成「沒回應」。
pub fn parse_row_id(v: &Value) -> Option<usize> {
    if let Some(n) = v.as_u64() {
        return usize::try_from(n).ok();
    }
    if let Some(f) = v.as_f64() {
        if f >= 0.0 && f.fract() == 0.0 && f < usize::MAX as f64 {
            return Some(f as usize);
        }
        return None;
    }
    let s = v.as_str()?.trim();
    s.parse::<usize>().ok()
}

/// 按比例的重試上限：`total × percent%`，夾在 `[min, max]`，而且不超過 `total`。
///
/// 取代寫死的上限（例如 400）：小整合包 400 等於全重試，大整合包 400 卻只試到一角。
pub fn proportional_cap(total: usize, percent: usize, min: usize, max: usize) -> usize {
    let scaled = total.saturating_mul(percent) / 100;
    scaled.clamp(min, max.max(min)).min(total)
}

/// 取鎖；如果別的執行緒拿著鎖時當掉（鎖中毒），照樣拿回資料繼續用。
///
/// 舊版寫法是 `lock().map(..).unwrap_or_default()`：鎖一中毒就回空值——
/// 整個階段已收到的譯文會在這一行靜靜消失。資料本身沒有壞，只是有人當掉，
/// 所以正確的做法是復原而不是丟掉。
pub fn lock_or_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| {
        crate::dev_log!("ai", "偵測到鎖中毒（有工作執行緒當掉），已復原並沿用資料");
        poisoned.into_inner()
    })
}

#[cfg(test)]
#[path = "retry_policy_tests.rs"]
mod tests;
