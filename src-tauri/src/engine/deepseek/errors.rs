//! 玩家向訊息與 AI 連線錯誤分類。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

// ═══ 玩家向訊息 ═══════════════════════════════════════════════

/// 給玩家看的額度／驗證說明（不洩模型／主機名，依目前 AI 來源給下一步）
pub(super) fn ai_quota_support_message(detail: &str) -> String {
    let d = sanitize_provider_name(detail);
    if d.contains("GPT") {
        return ai_quota_support_message_for(detail, AiProvider::Codex, false);
    }
    ai_quota_support_message_for(detail, AiProvider::Deepseek, false)
}

pub(super) fn ai_quota_support_message_for(detail: &str, provider: AiProvider, _managed: bool) -> String {
    let d = sanitize_provider_name(detail);
    if matches!(provider, AiProvider::Codex) {
        let title = super::super::run_interrupt::CHATGPT_BUSY_TITLE;
        return format!(
            "{title}\n\
{d}\n\n\
這次 ChatGPT 不接受翻譯請求，可能是翻譯用量已達上限，也可能只是短時間內送出太多次；這不一定代表你平常聊天的額度用完了。\n\
翻譯會消耗你的 ChatGPT 帳號額度，可以到 ChatGPT「設定 → 使用量」查看什麼時候重設。\n\
如果是開始前試翻時停下，還沒寫入任何翻譯；翻到一半才停下時，已翻好的部分都保留，額度重設後按「接續補完」。"
        );
    }
    if matches!(provider, AiProvider::LocalLlm) {
        return format!(
            "【本地模型沒有回應】\n\
{d}\n\n\
請確認已完成本地模型安裝。速度隨這台電腦而異。"
        );
    }
    // B5c：額度與金鑰分開標題（規格 §5.3）；完成卡的原因句由前端依 interruption.cause 寫，這段只進紀錄
    let title = match super::super::retry_policy::classify_message(&d) {
        super::super::retry_policy::FailureClass::AuthInvalid => "【自訂 API 金鑰被服務商拒絕】",
        _ => "【自訂 API 額度用完或帳戶沒有餘額】",
    };
    format!(
        "{title}\n\
{d}\n\n\
自訂 API 使用你填入服務商的金鑰與額度。\n\
請到該服務商後台確認金鑰、餘額與速率限制；也可改用 ChatGPT 或本地模型。"
    )
}

pub(super) fn sanitize_provider_name(s: &str) -> String {
    let mut out = s.to_string();
    for needle in [
        "deepseek",
        "DeepSeek",
        "DEEPSEEK",
        "api.deepseek.com",
        "deepseek-chat",
    ] {
        out = out.replace(needle, "AI 服務");
    }
    out
}

pub(super) fn looks_like_quota_or_auth_error(msg: &str) -> bool {
    // 僅依「當下」錯誤判斷；不含「無回應」——那是暫時連線問題，勿包成額度用完。
    // B4：改走 retry_policy。舊版看到 `exceed` 就判額度用完，於是
    // 「Rate limit exceeded」「maximum context length exceeded」都會把整輪 AI 停掉。
    super::super::retry_policy::is_quota_or_auth(msg)
}

pub(super) fn is_cancel_message(msg: &str) -> bool {
    msg.contains("已依你的要求停止")
}

pub(super) fn is_auth_unavailable_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("auth_unavailable")
        || msg.contains("會員驗證暫時無法連線")
        || msg.contains("登入／會員驗證暫時無法連線")
        || msg.contains("登入/會員驗證暫時無法連線")
}

/// 真需要玩家重登 Discord（不含暫態 auth_unavailable、不含裸 cloudflare HTML）。
pub(super) fn is_auth_relogin_error(msg: &str) -> bool {
    if is_auth_unavailable_error(msg) || is_cancel_message(msg) {
        return false;
    }
    let m = msg.to_ascii_lowercase();
    m.contains("login_required")
        || m.contains("login expired")
        || msg.contains("請先登入 Discord")
        || msg.contains("Discord 登入已失效")
        || msg.contains("請回到工具重新登入")
        || msg.contains("請在工具重新登入 Discord")
        || msg.contains("安全驗證已過期，請回到工具重新驗證")
}

pub(super) fn auth_relogin_message() -> String {
    format!("{}或需重新確認會員資格，請回到工具重新登入後再試。", super::super::run_interrupt::RELOGIN_MARK)
}

pub(super) fn auth_unavailable_message() -> String {
    "Discord 登入／會員驗證暫時無法連線，請稍後再試或重新登入；也可改用自訂 API。".into()
}

pub(super) fn truncate_err_msg(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max_chars).collect::<String>())
}

/// 以最新一筆錯誤分類「不可恢復」中止原因。暫態／限流不走這裡。
pub(super) fn classify_batch_abort(err_peek: &[String]) -> Option<String> {
    let latest = err_peek.last()?;
    if is_cancel_message(latest) {
        return Some(CANCEL_MESSAGE.to_string());
    }
    // auth_unavailable 可恢復，不在此秒殺
    if is_auth_unavailable_error(latest) {
        return None;
    }
    if is_auth_relogin_error(latest) {
        return Some(auth_relogin_message());
    }
    if looks_like_quota_or_auth_error(latest) {
        return Some(ai_quota_support_message(latest));
    }
    None
}

pub(super) fn is_recoverable_batch_error(msg: &str) -> bool {
    if is_cancel_message(msg) || is_auth_relogin_error(msg) {
        return false;
    }
    if looks_like_quota_or_auth_error(msg) && !is_auth_unavailable_error(msg) {
        // 明確額度／金鑰：不可靠重試；限流文案另判
        if msg.contains("請求太頻繁") || msg.contains("稍後再試") {
            return true;
        }
        if msg.contains("當日額度") || msg.contains("餘額不足") || msg.contains("額度可能已用完") {
            return false;
        }
    }
    if is_auth_unavailable_error(msg) {
        return true;
    }
    let m = msg.to_ascii_lowercase();
    m.contains("逾時")
        || m.contains("timeout")
        || m.contains("無回應")
        || m.contains("503")
        || m.contains("502")
        || m.contains("429")
        || m.contains("請求太頻繁")
        || m.contains("暫時無法")
        || m.contains("沒有回傳翻譯內容")
        || m.contains("無法解析")
        || m.contains("空 json")
        || m.contains("上游")
}

/// 從 Worker／上游 JSON 取出 `error.type`（僅診斷用，不含金鑰）。
pub(super) fn extract_proxy_error_type(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    v.get("error")
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChatErrorKind {
    /// 可退避重試（限流／5xx／驗證基建暫態）
    Transient,
    /// 當日預算／餘額——應中止 AI
    Quota,
    /// 需玩家重登 Discord
    Relogin,
    /// 需加入伺服器等硬失敗（不開 cookie 長等待）
    Fatal,
    /// B4：請求超過模型能處理的長度（或本地模型太慢等不到）——拆小再送，不是重送同一批
    TooLarge,
    /// B4：本地模型程式已經不在（當掉、記憶體不足被系統結束）——停下 AI、保留已翻部分
    ProcessGone,
    /// 審查 1a：本地模型等不到回應——拆小重送，而且**不算**「沒有進展」（慢不是壞）
    LocalTimeout,
    /// 審查 F1：本地模型逾時太多次（到頂後連續 3 次，或累計 30 分鐘）——停下 AI、保留已翻
    LocalUnusable,
}

#[derive(Debug, Clone)]
pub(super) struct MappedChatError {
    pub(super) message: String,
    pub(super) kind: ChatErrorKind,
}

impl MappedChatError {
    pub(super) fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ChatErrorKind::Transient,
        }
    }
}

/// 代管／自訂 AI 的 HTTP 錯誤 → 玩家可行動文案。優先看 `error.type`，避免上游狀態被誤標成 Discord。
pub(super) fn map_chat_http_error(code: u16, body: &str, managed: bool) -> Option<MappedChatError> {
    let err_type = extract_proxy_error_type(body);
    let type_ref = err_type.as_deref();
    let snippet = || sanitize_provider_name(&body.chars().take(160).collect::<String>());

    // 1) 明確 type 優先（與 HTTP 狀態解耦）
    match type_ref {
        Some("insufficient_quota") => {
            return Some(MappedChatError {
                message: "免費翻譯的當日額度已用完".into(),
                kind: ChatErrorKind::Quota,
            });
        }
        Some("login_required") => {
            return Some(MappedChatError {
                message: "使用開發者代管 AI 前，請先登入 Discord。".into(),
                kind: ChatErrorKind::Relogin,
            });
        }
        Some("auth_unavailable") => {
            return Some(MappedChatError {
                message: auth_unavailable_message(),
                kind: ChatErrorKind::Transient,
            });
        }
        Some("guild_required") => {
            // 高並行下 Worker member-tier 偶發閃斷；當暫態讓既有 attempts<2 重試。
            return Some(MappedChatError {
                message: "使用開發者代管 AI 前，請先加入 ZeitFrei 官方 Discord 伺服器。".into(),
                kind: ChatErrorKind::Transient,
            });
        }
        Some("client_upgrade_required") => {
            return Some(MappedChatError {
                message: "這個版本已不能使用開發者代管 AI，請更新工具後再試。".into(),
                kind: ChatErrorKind::Fatal,
            });
        }
        Some("server_not_ready") => {
            return Some(MappedChatError {
                message: "免費翻譯暫時無法使用（服務端維護中）。你可以自行填入 AI 金鑰，或稍後再試"
                    .into(),
                kind: ChatErrorKind::Fatal,
            });
        }
        Some("turnstile_unavailable") => {
            return Some(MappedChatError {
                message: "雲端翻譯閘門設定異常。請確認已登入 Discord 並加入官方伺服器；若仍失敗可改用自訂 API，或稍後再試。"
                    .into(),
                kind: ChatErrorKind::Fatal,
            });
        }
        _ => {}
    }

    match code {
        503 if !managed && body.contains("server_not_ready") => Some(MappedChatError {
            message: "免費翻譯暫時無法使用（服務端維護中）。你可以自行填入 AI 金鑰，或稍後再試"
                .into(),
            kind: ChatErrorKind::Fatal,
        }),
        503 if managed => {
            let s = snippet();
            if s.trim().is_empty() {
                Some(MappedChatError::transient(
                    "代管 AI 暫時無法使用（503）。請稍後再試，或改用自訂 API。",
                ))
            } else {
                Some(MappedChatError::transient(format!(
                    "代管 AI 暫時無法使用（503）：{s}"
                )))
            }
        }
        // 無 insufficient_quota type 的 429：當上游／邊緣限流，可重試
        429 if managed => Some(MappedChatError::transient("請求太頻繁，稍後再試")),
        426 if managed => Some(MappedChatError {
            message: "這個版本已不能使用開發者代管 AI，請更新工具後再試。".into(),
            kind: ChatErrorKind::Fatal,
        }),
        428 if managed => {
            // 僅明確驗證語意才當重登；裸 428 當暫態
            if body.to_ascii_lowercase().contains("turnstile")
                || body.to_ascii_lowercase().contains("verification")
            {
                Some(MappedChatError {
                    message: "安全驗證已過期，請回到工具重新驗證。".into(),
                    kind: ChatErrorKind::Relogin,
                })
            } else {
                Some(MappedChatError::transient("服務暫時要求重試（428），稍後再試"))
            }
        }
        401 if managed => {
            // 無 login_required type：多半是轉發上游，禁止標成 Discord
            let s = snippet();
            Some(MappedChatError {
                message: if s.trim().is_empty() {
                    "AI 上游拒絕請求（401）。請稍後再試，或改用自訂 API。".into()
                } else {
                    format!("AI 上游拒絕請求（401）：{s}")
                },
                kind: ChatErrorKind::Fatal,
            })
        }
        403 if managed => Some(MappedChatError {
            message: "使用開發者代管 AI 前，請先加入 ZeitFrei 官方 Discord 伺服器。".into(),
            kind: ChatErrorKind::Fatal,
        }),
        _ => None,
    }
}
