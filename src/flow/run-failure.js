/**
 * B5b S12 出錯（規格 §2.2 S12）：一句白話原因＋依原因的主要按鈕。
 *
 * 本批暫用錯誤字串判斷原因（計畫 B5b#6）；結構化分類碼由後續批次（B5c 起由 with_apply_notice／interruption 帶出）取代。
 * 純函式、不碰 DOM。完整原因照舊寫進紀錄，這裡只給狀態卡一句。
 */

import { isDiscordGateText } from "./ai-readiness.js";

export const FAILURE_ACTION = Object.freeze({
  changeAi: "ai-change",
  repair: "repair",
  issueReport: "issue-report",
});

const RETRY_LABEL = "再試一次";

/**
 * @param {string} text 錯誤訊息（formatInvokeError 後的文字）
 * @param {"run"|"supplement"|"repair"} origin 哪個動作失敗（「再試一次」重跑同一個）
 */
export function classifyFailure(text, origin = "run") {
  const t = String(text || "");
  const retry = { action: origin === "supplement" || origin === "repair" ? origin : "run", label: RETRY_LABEL };
  const ai = (kind, reason) => ({ kind, group: "ai", reason, primary: { action: FAILURE_ACTION.changeAi, label: "換 AI 再試" } });
  // 順序（第二輪審查 6，依後端原句取樣）：磁碟 → Discord → 本地模型 → 額度 → 金鑰 → ChatGPT → 網路 → 可修。
  // 本地模型與額度的原句常夾帶 reqwest 的「error sending request」或「逾時」，所以要排在網路之前；
  // 網路判斷又要在「可修」之前（訊息常帶 .zip 檔名）。
  if (/空間不足|磁碟空間|No space|not enough space|os error 112/i.test(t)) {
    return { kind: "disk", group: "other", reason: "磁碟空間不夠", primary: retry };
  }
  if (isDiscordGateText(t)) return ai("discord", "要先登入 Discord 並加入官方伺服器");
  if (/本地模型啟動不起來/.test(t)) return ai("local-start", "本地模型啟動不起來");
  if (/本地模型|找不到執行程式|找不到翻譯模型|執行程式無法/.test(t)) return ai("local", "本地模型沒有回應");
  // 狀態碼要跟 HTTP／status 字樣或全形括號（後端「（401）」寫法）一起出現才算（「已完成 401 條」「第 402 批」不算）
  const status = (code) => new RegExp(String.raw`(?:(?:HTTP|status)\D{0,3}${code}\b|（${code}）)`, "i").test(t);
  if (/額度|餘額|quota|insufficient|用量已達上限/i.test(t) || status(402)) return ai("quota", "AI 額度用完或帳戶沒有餘額");
  if (/金鑰|api[\s_-]?key|unauthorized/i.test(t) || status(401)) return ai("key", "金鑰被服務商拒絕");
  if (/ChatGPT|\bGPT\b/.test(t)) return ai("gpt", "ChatGPT 暫時不能用");
  if (/無法連上|連線|network|timed out|逾時|error sending request/i.test(t)) {
    return { kind: "network", group: "other", reason: "連不上服務，檢查網路", primary: retry };
  }
  if (/工作階段|資源包.{0,6}(壞|損|不完整)|翻譯檔/.test(t)) {
    return { kind: "repairable", group: "repair", reason: "翻譯檔需要修復", primary: { action: FAILURE_ACTION.repair, label: "修復翻譯檔" } };
  }
  return { kind: "other", group: "other", reason: "發生錯誤", primary: retry };
}

/** S12 狀態（R-1：主要按鈕恰好一顆；次要「問題回報」）。 */
export function failureState(failure) {
  const f = failure && typeof failure === "object" ? failure : classifyFailure("");
  return {
    id: "S12",
    tone: "error",
    sentence: `翻譯沒完成：${f.reason}`,
    extraLine: "完整原因在右側紀錄",
    disclosureKey: "",
    detailLines: [],
    primary: { ...f.primary },
    secondary: [{ action: FAILURE_ACTION.issueReport, label: "問題回報" }],
    more: [],
    disabledReason: "",
    // AI 類：主要按鈕「換 AI 再試」展開 E1，所以 AI 列要在
    showAiRow: f.group === "ai",
    showVersionRow: false,
  };
}
