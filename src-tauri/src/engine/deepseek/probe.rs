//! 開始前的 AI 可用性探測。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

/// 嚴格探測目前選定的 AI 是否真的能完成最小翻譯。
///
/// 「登入成功」、「/health 正常」或「帳號仍可聊天」都不等於翻譯端點可用：GPT
/// 的端點可能在實際 Responses 請求回 429，本地模型也可能健康檢查通過但 chat
/// endpoint／模型載入失敗。這裡必須走和正式批次相同的 `translate_chunk`，因此
/// custom API、GPT 與本地模型都會在掃描、寫檔前被同一條閘門驗證。
pub fn verify_ai_assistance() -> Result<(), String> {
    let engine = Engine::connect_raw().map_err(|e| sanitize_provider_name(&e))?;
    probe_ai_ready_inner(&engine, true).map_err(|e| {
        let clean = sanitize_provider_name(&e);
        if matches!(engine.provider, AiProvider::Codex) || looks_like_quota_or_auth_error(&clean) {
            ai_quota_support_message_for(&clean, engine.provider, engine.managed)
        } else {
            clean
        }
    })
}

/// 嚴格探測自訂 API 金鑰（儲存後／測試鈕）。
/// 網路失敗、空內容、401／餘額問題一律 Err；代管模式不可呼叫。
pub fn verify_custom_api() -> Result<(), String> {
    if get_ai_mode() != "custom" {
        return Err("目前不是自訂 API 模式。".into());
    }
    verify_ai_assistance()
}

/// 輕量探測：餘額 API 或迷你 chat；網路抖動不阻擋，明確額度／金鑰問題直接回錯。
pub(super) fn probe_ai_ready(engine: &Engine) -> Result<(), String> {
    probe_ai_ready_inner(engine, false)
}

pub(super) fn probe_ai_ready_inner(engine: &Engine, strict: bool) -> Result<(), String> {
    let base = engine.base_url.trim_end_matches('/');

    if matches!(engine.provider, AiProvider::LocalLlm) {
        let health = format!("{base}/health");
        match engine.client.get(&health).send() {
            Ok(resp) if resp.status().is_success() => {}
            Ok(_) => return Err("本地模型尚未就緒。請先完成安裝並等到健康檢查通過。".into()),
            Err(err) if strict => return Err(format!("本地模型連線失敗：{err}")),
            Err(_) => return Err("本地模型尚未就緒。請先完成安裝並等到健康檢查通過。".into()),
        }
        // 健康檢查只證明 server 活著；不能在這裡提早 return。後面仍須送出
        // 一句真正的翻譯請求，才知道模型、chat endpoint、API key 與輸出格式都可用。
    }

    // 代管 Worker 沒有 /user/balance 端點，跳過餘額查詢，直接用迷你 chat 探測。
    if !engine.managed && !matches!(engine.provider, AiProvider::Codex) {
        let bal_url = format!("{base}/user/balance");
        match engine
            .client
            .get(&bal_url)
            .header("Authorization", format!("Bearer {}", engine.api_key))
            .send()
        {
            Ok(resp) => {
                let code = resp.status().as_u16();
                if code == 401 || code == 403 {
                    return Err("金鑰無效或無權限".into());
                }
                if code == 402 {
                    return Err("帳號餘額不足".into());
                }
                if resp.status().is_success() {
                    if let Ok(v) = resp.json::<Value>() {
                        if balance_looks_empty(&v) {
                            return Err("帳號餘額為零或不足".into());
                        }
                    }
                }
            }
            Err(err) if strict => {
                return Err(format!("連線失敗（無回應）：{err}"));
            }
            Err(_) => {}
        }
    }

    // 不用「OK」：模型原樣回 OK 也會被視為有內容，卻不能證明它真的能翻譯。
    // 這筆 `MaskedItem` 不會交給 `fill_missing`，因此沒有任何寫入 LangMap、TM 或共享庫的路徑。
    let probe_texts = vec![AI_TRANSLATION_PROBE_SOURCE.to_string()];
    let prompt = Arc::new(build_system_prompt(
        &glossary::load(None),
        &probe_texts,
        matches!(engine.provider, AiProvider::LocalLlm),
    ));
    let plan = BatchPlan {
        track: BatchTrackKind::Ui,
        items: vec![MaskedItem {
            uid: 0,
            source: AI_TRANSLATION_PROBE_SOURCE.into(),
            masked: AI_TRANSLATION_PROBE_SOURCE.into(),
            tokens: Vec::new(),
            context: None,
        }],
    };
    match translate_chunk(engine, &prompt, &plan) {
        Ok(success)
            if success
                .map
                .get(&0)
                .is_some_and(|translated| quality_fail_reason(AI_TRANSLATION_PROBE_SOURCE, translated).is_ok()) =>
        {
            Ok(())
        }
        Ok(success) if success.map.contains_key(&0) => {
            Err("AI 有回應，但沒有給出可用的繁體中文翻譯。".into())
        }
        Ok(_) if cancel::is_cancelled() => {
            if strict {
                Err("探測已取消".into())
            } else {
                Ok(())
            }
        }
        Ok(_) => Err("探測成功連線但沒有內容回應".into()),
        Err(err) => {
            if looks_like_quota_or_auth_error(&err.message) || strict {
                Err(err.message)
            } else {
                // 網路暫時失敗：不在此硬擋，讓主流程再試
                Ok(())
            }
        }
    }
}

pub(super) fn balance_looks_empty(v: &Value) -> bool {
    if let Some(false) = v.get("is_available").and_then(|x| x.as_bool()) {
        return true;
    }
    if let Some(arr) = v.get("balance_infos").and_then(|x| x.as_array()) {
        let mut any_positive = false;
        for item in arr {
            let total = item
                .get("total_balance")
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| item.get("total_balance").and_then(|x| x.as_f64()))
                .unwrap_or(0.0);
            if total > 0.0001 {
                any_positive = true;
            }
        }
        return !any_positive && !arr.is_empty();
    }
    false
}
