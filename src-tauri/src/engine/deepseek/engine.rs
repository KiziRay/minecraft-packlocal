//! Engine 連線建立與待送項目整理。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

impl Engine {
    pub(super) fn connect_raw() -> Result<Self, String> {
        let cfg = resolve_ai_config()?;
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(cfg.capabilities.start_parallel + 2)
            .build()
            .map_err(|e| e.to_string())?;
        let url = if matches!(cfg.provider, AiProvider::Codex) {
            CODEX_RESPONSES_URL.to_string()
        } else {
            api_chat_completions_url(&cfg.base_url)
        };
        Ok(Engine {
            client: Arc::new(client),
            base_url: Arc::new(cfg.base_url.clone()),
            url: Arc::new(url),
            api_key: Arc::new(cfg.api_key),
            model: Arc::new(cfg.model),
            managed_session: Arc::new(Mutex::new(String::new())),
            managed: false,
            provider: cfg.provider,
            capabilities: cfg.capabilities,
            degraded: Arc::new(Mutex::new(RequestDegradeState::default())),
            usage: Arc::new(Mutex::new(AiUsageTotals::default())),
            notices: Arc::new(Mutex::new(Vec::new())),
            local_timeouts: Arc::new(Mutex::new(Default::default())),
        })
    }

    /// 用「雲端補完」設定建一個引擎，不管使用者目前選的是哪個模式。
    ///
    /// 給本地模式的品質升級用：小模型過不了品質關的句子改用雲端翻一次，
    /// 交付出去的結果就跟全程走雲端一樣。
    pub(super) fn connect_cloud_fallback() -> Option<Self> {
        // 測試不可讀這台電腦真實的金鑰去打真實雲端
        #[cfg(test)]
        if let Some(engine) = test_hooks::cloud_fallback() {
            return engine;
        }
        let cfg = resolve_cloud_fallback_config()?;
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(cfg.capabilities.start_parallel + 2)
            .build()
            .ok()?;
        let url = if matches!(cfg.provider, AiProvider::Codex) {
            CODEX_RESPONSES_URL.to_string()
        } else {
            api_chat_completions_url(&cfg.base_url)
        };
        Some(Engine {
            client: Arc::new(client),
            base_url: Arc::new(cfg.base_url.clone()),
            url: Arc::new(url),
            api_key: Arc::new(cfg.api_key),
            model: Arc::new(cfg.model),
            managed_session: Arc::new(Mutex::new(String::new())),
            managed: false,
            provider: cfg.provider,
            capabilities: cfg.capabilities,
            degraded: Arc::new(Mutex::new(RequestDegradeState::default())),
            usage: Arc::new(Mutex::new(AiUsageTotals::default())),
            notices: Arc::new(Mutex::new(Vec::new())),
            local_timeouts: Arc::new(Mutex::new(Default::default())),
        })
    }

    pub(super) fn connect() -> Result<Self, String> {
        let engine = Self::connect_raw()?;
        if let Err(probe) = probe_ai_ready(&engine) {
            return Err(ai_quota_support_message_for(
                &probe,
                engine.provider,
                false,
            ));
        }
        engine.reset_usage();
        Ok(engine)
    }

    pub(super) fn current_features(&self) -> RequestFeatures {
        retry_policy::lock_or_recover(&self.degraded).features(self.capabilities)
    }

    pub(super) fn maybe_degrade_for_unsupported(&self, code: u16, body: &str) -> bool {
        if !(400..500).contains(&code) || !mentions_unsupported_params(body) {
            return false;
        }
        let lower = body.to_ascii_lowercase();
        let mentions_response_format = lower.contains("response_format");
        let mentions_token_field =
            lower.contains("max_tokens") || lower.contains("max_completion_tokens");
        let mentions_temperature = lower.contains("temperature");
        let generic_only =
            !mentions_response_format && !mentions_token_field && !mentions_temperature;
        let mut changed = None;
        {
            let mut state = retry_policy::lock_or_recover(&self.degraded);
            if self.capabilities.supports_json_mode
                && !state.drop_response_format
                && (mentions_response_format || generic_only)
            {
                state.drop_response_format = true;
                changed = Some(
                    "AI 相容降級：此服務不接受 response_format，後續改用提示要求 JSON 物件。"
                        .to_string(),
                );
            } else if !state.use_max_completion_tokens
                && self.capabilities.max_tokens_field != MaxTokensField::MaxCompletionTokens
                && (mentions_token_field || generic_only)
            {
                state.use_max_completion_tokens = true;
                changed = Some(
                    "AI 相容降級：此服務不接受目前的 token 欄位，後續改用 max_completion_tokens。"
                        .to_string(),
                );
            } else if !state.drop_temperature && (mentions_temperature || generic_only) {
                state.drop_temperature = true;
                changed = Some(
                    "AI 相容降級：此服務不接受 temperature，後續請求已省略。".to_string(),
                );
            }
        }
        if let Some(note) = changed {
            self.push_notice(note);
            true
        } else {
            false
        }
    }

    pub(super) fn push_notice(&self, note: String) {
        let mut notices = retry_policy::lock_or_recover(&self.notices);
        if !notices.iter().any(|existing| existing == &note) {
            notices.push(note);
        }
    }

    pub(super) fn drain_notices(&self) -> Vec<String> {
        std::mem::take(&mut *retry_policy::lock_or_recover(&self.notices))
    }

    pub(super) fn record_usage(&self, usage: &AiUsageTotals) {
        retry_policy::lock_or_recover(&self.usage).add(usage);
    }

    pub(super) fn usage_snapshot(&self) -> AiUsageTotals {
        retry_policy::lock_or_recover(&self.usage).clone()
    }

    pub(super) fn reset_usage(&self) {
        *retry_policy::lock_or_recover(&self.usage) = AiUsageTotals::default();
    }

    pub(super) fn session_cookie(&self) -> String {
        retry_policy::lock_or_recover(&self.managed_session).clone()
    }
}

/// 已無免費代管；此函式保留給舊呼叫點，恆為 false。
#[allow(dead_code)]
pub fn managed_ai_available() -> bool {
    false
}

pub(super) fn build_masked_items(
    items: &[PendingItem],
    ctx: &[Option<&'static str>],
) -> Vec<MaskedItem> {
    items
        .iter()
        .map(|item| {
            let (masked, tokens) = placeholder::mask(&item.source);
            MaskedItem {
                uid: item.uid,
                source: item.source.clone(),
                masked,
                tokens,
                context: ctx.get(item.uid).copied().flatten(),
            }
        })
        .collect()
}

/// AI 這一輪跑完後，還沒有譯文的句子怎麼收尾。
///
/// # 為什麼要分兩種
///
/// 舊版一律當成「佔位符失敗」：寫回英文原文 ＋ 進負向快取。對真的把 `%s`
/// 弄壞的譯文來說那是對的，但對「這一批 AI 根本沒回應」的句子來說是災難——
/// 英文被寫進 langmap 看起來像翻完了，負向快取又讓「補充漏翻」連試都不試，
/// 所以那些句子**永遠**回不來。
///
/// 回傳 `(佔位符失敗數, 沒拿到回應數)`。
pub(super) fn drain_pending_ai(
    pending: &[PendingItem],
    translations: &mut HashMap<usize, String>,
    report: &mut AiFillReport,
    no_answer: &mut HashSet<usize>,
) -> (usize, usize) {
    let mut broken = 0usize;
    let mut unanswered = 0usize;
    for item in pending {
        match item.reason {
            PendingReason::PlaceholderBroken => {
                // 譯文把格式符號弄壞了：英文原文至少不會讓遊戲當掉，
                // 而且同一句再送一次結果通常一樣，所以負向快取是對的。
                keep_english_skip(translations, report, item.uid, &item.source, true, None);
                broken += 1;
            }
            PendingReason::NoAnswer => {
                // 沒拿到回應＝還沒翻到，不是翻不動。
                // 不寫英文、不進負向快取，留在缺口裡讓補完與補充漏翻能再試。
                // B4：放進「沒回應」清單，不再混進品質暫緩（補充漏翻會跳過品質暫緩）。
                no_answer.insert(item.uid);
                unanswered += 1;
            }
        }
    }
    if broken > 0 {
        report
            .notes
            .push(format!("已略過 {broken} 句佔位符失敗（不重燒）"));
    }
    if unanswered > 0 {
        report.notes.push(format!(
            "有 {unanswered} 句這一輪沒拿到 AI 回應（不是翻不動）。\
它們維持原文並留在缺口裡，下次按「補充漏翻」會再試一次。"
        ));
    }
    (broken, unanswered)
}

pub(super) fn collect_unresolved_items(
    items: &[MaskedItem],
    resolved: &HashMap<usize, String>,
) -> Vec<PendingItem> {
    items
        .iter()
        .filter(|item| !resolved.contains_key(&item.uid))
        // 沒出現在結果裡＝沒拿到回應（批次被放棄或回應缺這一條）
        .map(|item| PendingItem {
            uid: item.uid,
            source: item.source.clone(),
            reason: PendingReason::NoAnswer,
        })
        .collect()
}
