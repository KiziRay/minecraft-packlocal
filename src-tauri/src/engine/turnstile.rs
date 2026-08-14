//! 代管 AI 協定常數與 Turnstile 相容殘件。
//!
//! P0 起：代管閘門改為僅 Discord；Turnstile 不再強制。
//! 保留 command／狀態 API 以免舊前端崩潰，一律回「不需要驗證」。

#![allow(dead_code)]

use serde::Serialize;
use tauri::AppHandle;

pub const MANAGED_AI_PROTOCOL: &str = "3";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnstileStatus {
    pub verified: bool,
    pub expires_at: u64,
    pub message: String,
}

/// 服務端不再強制 Turnstile。
pub fn managed_turnstile_required() -> Result<bool, String> {
    Ok(false)
}

pub fn turnstile_status() -> TurnstileStatus {
    TurnstileStatus {
        verified: true,
        expires_at: 0,
        message: "目前不需要額外安全驗證；登入 Discord 並加入官方伺服器即可。".into(),
    }
}

/// 舊呼叫端若仍要憑證：回空字串（Worker 已不檢查）。
pub fn managed_ai_turnstile_proof() -> Result<String, String> {
    Ok(String::new())
}

pub fn clear_turnstile_proof() {}

pub fn cancel_turnstile_verification() {}

pub fn verify_turnstile_blocking(_app: AppHandle) -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "skipped": true,
        "message": "目前不需要額外安全驗證。"
    })
}
