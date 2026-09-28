/**
 * 首次流程（規格 D-01、§4.2 tour、§6）：同意頁本版一次 → 接新手引導。
 *
 * 「本版」＝同意內容的版本，不是工具版號：內容改了（例如改成誠實說明會放入翻好的模組檔）
 * 就要讓每個人再看一次；內容沒改，更新工具不會重問。存在既有白名單路徑 consent.hideVersion。
 * 舊版存的是 "1"（勾「下次不再顯示」），不等於本版內容，所以升級後會看到一次新的 D-01。
 */

export const CONSENT_CONTENT_VERSION = "2.0-d01";

export const CONSENT_COPY = Object.freeze({
  title: "開始前先知道這幾件事？",
  lines: Object.freeze([
    "會翻成繁體中文並套用到遊戲（含放入翻好的模組檔），第一次會問要不要備份。",
    "可能翻錯，圖片上的字翻不到。",
    "翻好的句子會分享到共享庫，不含金鑰與帳號。",
  ]),
  accept: "我了解，開始使用",
  details: "詳細說明",
});

export function isConsentAccepted(stored) {
  return String(stored ?? "").trim() === CONSENT_CONTENT_VERSION;
}

/** 按下「我了解」之後：沒看過引導就接引導，看過就直接回主畫面。 */
export function afterConsent({ tourSeen = false } = {}) {
  return tourSeen ? "none" : "tour";
}

/** 跳過引導時的 toast（規格 §3.6，≤24 字）。 */
export const TOUR_SKIPPED_TOAST = "已跳過引導，可到 設定→關於→重看引導 找回";
