//! AI 補譯層。**只在本機整理完、確定缺中文時才會用到。**
//!
//! 送出前會依序擋掉不必送的東西，AI 只翻真正剩下的：
//! 1. 相同英文去重
//! 2. 術語表直接命中（官方台灣譯名，免費且一致）
//! 3. 翻譯記憶命中（上次或別的整合包翻過）
//!
//! 收回來後每一條都過佔位符把關，`%s` 被吃掉的譯文一律退回原文——
//! 少一句中文只是可惜，少一個 `%s` 是遊戲當場報錯。

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use super::cancel;
use super::cancel::CANCEL_MESSAGE;
use super::codex_chat::{self, CODEX_RESPONSES_URL};
use super::eligibility;
use super::glossary::{self, Glossary, TermConsistencyStats};
use super::jar_scan::LangMap;
use super::mech_tokens::{is_poisoned_mech_translation, skip_before_ai};
use super::local_quality;
use super::placeholder::{self, GuardStats};
use super::sentence_split;
use super::shared_glossary;
use super::shared_tm;
use super::secrets::{
    api_chat_completions_url, get_ai_mode, resolve_ai_config, resolve_cloud_fallback_config,
    AiProvider, MaxTokensField, ProviderCapabilities,
};
use super::translation_quality::{is_usable_zh, quality_fail_reason, QualityFailReason};
use super::tm::{Tm, TmSaveGuard};
use super::translation_mode::TranslationQuality;
use super::translation_scope::TranslationScope;
use super::turnstile::MANAGED_AI_PROTOCOL;
use super::retry_policy;

/// 連續幾輪「整組都沒譯出」且屬可恢復失敗時提前結束（保留已得譯文）
const EMPTY_ROUNDS_ABORT: usize = 3;
/// 單批計劃失敗後最多再排入佇列幾次
const MAX_PLAN_REQUEUE: usize = 2;
/// 空 content／空 JSON 時 chunk 內最多再試幾次（含首次後的重試上限）
const EMPTY_CONTENT_RETRY_LIMIT: usize = 4;
/// 單句嚴格重試硬上限，避免近千句一句一請求燒錢
const STRICT_PLACEHOLDER_RETRY_CAP: usize = 48;
const MAX_PROMPT_GLOSSARY_TERMS: usize = 150;
const MIN_COMPLETION_TOKENS: usize = 512;
const MAX_COMPLETION_TOKENS: usize = 8192;
/// 依條數估 completion 下限（關閉思考後仍要夠裝 JSON 譯文）
const TOKENS_PER_ITEM: usize = 48;
/// finish_reason=length 時同尺寸最多再試幾次（0＝立刻拆半重排）
const LENGTH_TRUNCATION_RETRY_LIMIT: usize = 0;
const NAME_MAX_CHARS: usize = 48;
const UI_MAX_CHARS: usize = 200;
const SOLO_MIN_CHARS: usize = 2000;
const STORY_MIN_CHARS: usize = 200;
/// 每次開始翻譯前唯一送出的最小真實翻譯。它只活在記憶體，絕不進 LangMap、TM 或共享庫。
const AI_TRANSLATION_PROBE_SOURCE: &str = "Iron Sword";

// T1：原 deepseek.rs（約 6000 行）依職責拆成下列子模組，行為零變更。
// 子模組以 `use super::*;` 共用本檔的 import 與常數；本檔再把子模組項目全部帶回，
// 對外路徑 `engine::deepseek::X` 與拆分前相同。
mod report;
mod errors;
mod fill;
mod plain;
mod resolve;
mod escalate;
mod types;
mod engine;
mod prompt;
mod batches;
mod request;
mod chunk;
mod parse;
mod probe;

pub use self::report::*;
use self::errors::*;
pub use self::fill::*;
pub use self::plain::*;
use self::resolve::*;
use self::escalate::*;
use self::types::*;
pub use self::engine::*;
pub(crate) use self::prompt::*;
use self::batches::*;
use self::request::*;
use self::chunk::*;
pub use self::parse::*;
pub use self::probe::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod b4_tests;

/// 測試注入點：AI 引擎、雲端補完引擎（只在 `cargo test` 編譯；每條執行緒各自一份，平行測試互不干擾）。
#[cfg(test)]
mod test_hooks {
    use super::Engine;
    use std::cell::RefCell;

    thread_local! {
        static PREFLIGHT: RefCell<Option<Result<Engine, String>>> = const { RefCell::new(None) };
        static CLOUD: RefCell<Option<Engine>> = const { RefCell::new(None) };
    }

    pub fn set_preflight(result: Option<Result<Engine, String>>) {
        PREFLIGHT.with(|c| *c.borrow_mut() = result);
    }

    pub fn set_cloud(engine: Option<Engine>) {
        CLOUD.with(|c| *c.borrow_mut() = engine);
    }

    /// 永遠回 `Some`：沒注入就是「AI 不可用」，絕不落到真實流程。
    pub fn preflight() -> Option<Result<Engine, String>> {
        Some(PREFLIGHT.with(|c| c.borrow().clone()).unwrap_or_else(|| Err("測試沒有注入 AI 引擎".into())))
    }

    /// 永遠回 `Some`：沒注入就是「沒有雲端可用」。
    pub fn cloud_fallback() -> Option<Option<Engine>> {
        Some(CLOUD.with(|c| c.borrow().clone()))
    }
}
