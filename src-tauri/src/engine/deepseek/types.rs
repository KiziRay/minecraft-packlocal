//! 連線與批次的資料型別（Engine、批次計畫、執行狀態）。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

// ═══ 連線 ═════════════════════════════════════════════════════

#[derive(Clone)]
pub(super) struct Engine {
    pub(super) client: Arc<reqwest::blocking::Client>,
    pub(super) base_url: Arc<String>,
    pub(super) url: Arc<String>,
    /// 自訂 API 金鑰；GPT 模式可為空。
    pub(super) api_key: Arc<String>,
    pub(super) model: Arc<String>,
    pub(super) managed_session: Arc<Mutex<String>>,
    pub(super) managed: bool,
    pub(super) provider: AiProvider,
    pub(super) capabilities: ProviderCapabilities,
    pub(super) degraded: Arc<Mutex<RequestDegradeState>>,
    pub(super) usage: Arc<Mutex<AiUsageTotals>>,
    pub(super) notices: Arc<Mutex<Vec<String>>>,
    /// B4 第二輪 F1／F2：本地模型逾時的累計與等待延長（每次翻譯呼叫一份，新一輪從 ×1 開始）
    pub(super) local_timeouts: Arc<Mutex<crate::engine::local_llm::timeouts::TimeoutTracker>>,
}

#[derive(Debug, Clone)]
pub(super) struct PendingItem {
    pub(super) uid: usize,
    pub(super) source: String,
    pub(super) reason: PendingReason,
}

/// 一句話為什麼還躺在待譯清單裡。**這兩種必須分開處理。**
///
/// 舊版把兩種都當成「佔位符失敗」：寫回英文 ＋ 進負向快取。
/// 結果是「這一批 AI 根本沒回應」的句子被永久標記成翻不動——
/// 玩家按「補充漏翻」也救不回來，因為負向快取讓它連試都不試。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingReason {
    /// AI 有回應，但譯文把 `%s`／`§` 之類的格式符號弄壞了。
    ///
    /// 寫回英文是對的：壞掉的格式符號會讓遊戲當掉，英文至少能玩。
    /// 進負向快取也是對的：同一句再送一次，結果通常一樣。
    PlaceholderBroken,
    /// 根本沒拿到回應（批次重試額度用盡、連線中斷、回應裡缺這一條）。
    ///
    /// 這種**不是**翻不動，是還沒翻到。不可以寫回英文假裝完成，
    /// 也不可以進負向快取——要留在缺口裡，讓補完／補充漏翻能再試。
    NoAnswer,
}

#[derive(Debug, Clone)]
pub(super) struct MaskedItem {
    pub(super) uid: usize,
    pub(super) source: String,
    pub(super) masked: String,
    pub(super) tokens: Vec<String>,
    pub(super) context: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RequestFeatures {
    pub(super) send_response_format: bool,
    pub(super) max_tokens_field: MaxTokensField,
    pub(super) send_temperature: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct RequestDegradeState {
    pub(super) drop_response_format: bool,
    pub(super) use_max_completion_tokens: bool,
    pub(super) drop_temperature: bool,
}

impl RequestDegradeState {
    pub(super) fn features(self, capabilities: ProviderCapabilities) -> RequestFeatures {
        let max_tokens_field = if self.use_max_completion_tokens {
            MaxTokensField::MaxCompletionTokens
        } else {
            capabilities.max_tokens_field
        };
        RequestFeatures {
            send_response_format: capabilities.supports_json_mode && !self.drop_response_format,
            max_tokens_field,
            send_temperature: !self.drop_temperature,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum BatchTrackKind {
    Name,
    Ui,
    Story,
    Solo,
}

impl BatchTrackKind {
    /// 每批送幾條。
    ///
    /// 本地模型另有一套較小的值。原因不是品質假設，是硬限制：llama-server 的上下文視窗
    /// 是 prompt ＋ 輸出共用，一批 48 條加上術語表很容易把視窗吃滿，結果是漏條、
    /// 格式跑掉或直接截斷。每條字串本來就彼此獨立、不需要跨條上下文，所以縮小批次
    /// 只影響「一次送幾條」，不影響單條的翻譯品質；模組包語境是靠 system prompt 的
    /// 術語表提供的，與批次大小無關。
    pub(super) fn batch_size_for(self, strict_single: bool, local: bool) -> usize {
        if strict_single {
            return 1;
        }
        if local {
            return match self {
                Self::Name => LOCAL_MAX_BATCH_ITEMS,
                Self::Ui => LOCAL_MAX_BATCH_ITEMS,
                Self::Story => 6,
                Self::Solo => 1,
            };
        }
        match self {
            Self::Name => 48,
            Self::Ui => 48,
            Self::Story => 20,
            Self::Solo => 1,
        }
    }

    pub(super) fn timeout_secs(self) -> u64 {
        match self {
            Self::Name => 60,
            Self::Ui => 90,
            Self::Story | Self::Solo => 240,
        }
    }

    /// Name／Ui 大批空回應重排時可拆半；Story／Solo 已夠小不拆。
    pub(super) fn may_split_on_empty_requeue(self) -> bool {
        matches!(self, Self::Name | Self::Ui)
    }
}

/// AIMD 只認明確 congestion（429／5xx／逾時），不含空 content 等 Transient。
pub(super) fn should_mark_round_congestion(err_congestion: bool) -> bool {
    err_congestion
}

pub(super) fn empty_content_backoff_ms(attempt: usize) -> u64 {
    400 + attempt as u64 * 500
}

pub(super) fn choice_finish_reason(v: &Value) -> &str {
    v["choices"][0]
        .get("finish_reason")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
}

pub(super) fn is_length_truncated(v: &Value) -> bool {
    choice_finish_reason(v).eq_ignore_ascii_case("length")
}

/// DeepSeek 官方：思考模式預設開啟；批次 JSON 翻譯需明確關閉。
pub(super) fn should_send_thinking_disabled(engine: &Engine) -> bool {
    matches!(engine.provider, AiProvider::Deepseek)
}

/// 本地模型多半是 Qwen3 系列的「混合推理」模型，llama-server（--jinja）用
/// `chat_template_kwargs.enable_thinking` 這個鍵關閉思考過程——跟 DeepSeek 官方的
/// `thinking` 欄位是兩套完全不同的 API 慣例，不能共用同一個判斷式。
pub(super) fn should_disable_local_thinking(engine: &Engine) -> bool {
    matches!(engine.provider, AiProvider::LocalLlm)
}

pub(super) fn is_empty_response_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("沒有回傳翻譯內容")
        || m.contains("回傳為空 json")
        || m.contains("空 json 物件")
        || m.contains("finish_reason=length")
}

/// 從 choices[0] 拼空 content 診斷（finish_reason／refusal／鍵名），方便日誌對症。
pub(super) fn diagnose_empty_choice(v: &Value) -> String {
    let choice = &v["choices"][0];
    let finish = choice
        .get("finish_reason")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    let message = &choice["message"];
    let refusal = message
        .get("refusal")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    let mut parts: Vec<String> = Vec::new();
    if !finish.is_empty() {
        parts.push(format!("finish_reason={finish}"));
    }
    if !refusal.is_empty() {
        let short: String = refusal.chars().take(80).collect();
        parts.push(format!("refusal={short}"));
    }
    let choice_keys = object_field_keys(choice, 12);
    if !choice_keys.is_empty() {
        parts.push(format!("choice_keys={choice_keys}"));
    }
    let message_keys = object_field_keys(message, 12);
    if !message_keys.is_empty() {
        parts.push(format!("message_keys={message_keys}"));
    }
    if parts.is_empty() {
        "服務有連線但沒有回傳翻譯內容".into()
    } else {
        format!(
            "服務有連線但沒有回傳翻譯內容（{}）",
            parts.join(", ")
        )
    }
}

/// 消毒：只列 JSON object 鍵名，不寫內容／金鑰。
pub(super) fn object_field_keys(v: &Value, max: usize) -> String {
    let Some(map) = v.as_object() else {
        return String::new();
    };
    let mut keys: Vec<&str> = map.keys().map(|k| k.as_str()).collect();
    keys.sort_unstable();
    keys.into_iter().take(max).collect::<Vec<_>>().join(",")
}

/// content 空時，僅當 reasoning 可解析為譯文 JSON 才採用（reasoner／相容閘道）。
pub(super) fn looks_like_translation_payload(text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    parse_translation_object(text)
        .map(|m| !m.is_empty())
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
pub(super) struct BatchPlan {
    pub(super) track: BatchTrackKind,
    pub(super) items: Vec<MaskedItem>,
}

#[derive(Debug)]
pub(super) struct ChunkError {
    pub(super) message: String,
    pub(super) congestion: bool,
    pub(super) retries: usize,
    pub(super) kind: ChatErrorKind,
}

#[derive(Debug)]
pub(super) struct ChunkSuccess {
    pub(super) map: HashMap<usize, String>,
    pub(super) retries: usize,
}

#[derive(Debug, Clone)]
pub(super) struct QueuedPlan {
    pub(super) batch_no: usize,
    pub(super) plan: BatchPlan,
    pub(super) requeues: usize,
}

/// 空回應重排：Name／Ui 且 >1 條時拆半，降低截斷／空物件機率。
pub(super) fn split_queued_for_retry(queued: QueuedPlan) -> Vec<QueuedPlan> {
    let next_requeues = queued.requeues.saturating_add(1);
    let track = queued.plan.track;
    let batch_no = queued.batch_no;
    let items = queued.plan.items;
    if track.may_split_on_empty_requeue() && items.len() > 1 {
        let mid = items.len() / 2;
        let right = items[mid..].to_vec();
        let left = items[..mid].to_vec();
        return vec![
            QueuedPlan {
                batch_no,
                plan: BatchPlan {
                    track,
                    items: left,
                },
                requeues: next_requeues,
            },
            QueuedPlan {
                batch_no,
                plan: BatchPlan {
                    track,
                    items: right,
                },
                requeues: next_requeues,
            },
        ];
    }
    vec![QueuedPlan {
        batch_no,
        plan: BatchPlan { track, items },
        requeues: next_requeues,
    }]
}

/// B4：拆批最多拆幾層（12→6→3→2→1）。
pub(super) const MAX_SPLIT_DEPTH: usize = 4;

/// 不論哪一軌都拆半（內容太長、輸出被截斷時用）。
pub(super) fn split_queued_forced(queued: QueuedPlan) -> Vec<QueuedPlan> {
    let next_requeues = queued.requeues.saturating_add(1);
    let QueuedPlan { batch_no, plan, .. } = queued;
    if plan.items.len() <= 1 {
        return vec![QueuedPlan {
            batch_no,
            plan,
            requeues: next_requeues,
        }];
    }
    let mid = plan.items.len() / 2;
    let (left, right) = plan.items.split_at(mid);
    [left, right]
        .into_iter()
        .map(|part| QueuedPlan {
            batch_no,
            plan: BatchPlan {
                track: plan.track,
                items: part.to_vec(),
            },
            requeues: next_requeues,
        })
        .collect()
}

/// 失敗批放回佇列**前端**（優先重送）。`split`：`None`＝原樣；`Some(false)`＝只拆 Name／Ui；
/// `Some(true)`＝不論哪一軌都拆半。
pub(super) fn push_requeue_plans_front(pending: &mut VecDeque<QueuedPlan>, queued: QueuedPlan, split: Option<bool>) {
    let parts = match split {
        Some(true) => split_queued_forced(queued),
        Some(false) => split_queued_for_retry(queued),
        None => {
            let mut again = queued;
            again.requeues = again.requeues.saturating_add(1);
            vec![again]
        }
    };
    for part in parts.into_iter().rev() {
        pending.push_front(part);
    }
}

/// B4：run_batches 為什麼提早結束。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BatchStop {
    /// 使用者按了停止
    Cancelled,
    /// AI 這一輪不能再用（額度、金鑰、登入、本地程式消失、連線中斷太久）——白話原因
    AiUnavailable(String),
    /// 連續很多批都沒有新譯文——白話原因
    NoProgress(String),
}

#[derive(Debug)]
pub(super) struct BatchRuntimeState {
    pub(super) system_prompt: Arc<String>,
    pub(super) parallel_cap: usize,
    pub(super) current_parallel: usize,
    pub(super) warmed_up: bool,
    /// B4：每批再切成幾份（沒回應的句子同輪縮小批次重送時 >1）
    pub(super) batch_divisor: usize,
    /// B4：最近一次 run_batches 提早結束的原因（呼叫端據此停止後續 AI）
    pub(super) stop: Option<BatchStop>,
}

impl BatchRuntimeState {
    pub(super) fn new(system_prompt: String, parallel_cap: usize) -> Self {
        let cap = parallel_cap.max(1);
        Self {
            system_prompt: Arc::new(system_prompt),
            parallel_cap: cap,
            current_parallel: 1,
            warmed_up: false,
            batch_divisor: 1,
            stop: None,
        }
    }

    pub(super) fn finish_round(&mut self, congestion: bool, made_progress: bool) {
        if congestion {
            self.current_parallel = (self.current_parallel / 2).max(1);
            self.warmed_up = true;
            return;
        }
        if !made_progress {
            return;
        }
        if !self.warmed_up {
            self.current_parallel = 4.min(self.parallel_cap).max(1);
            self.warmed_up = true;
        } else {
            self.current_parallel = (self.current_parallel + 2).min(self.parallel_cap);
        }
    }
}
