/**
 * B5b 翻譯中進度（規格 §3.2、ux-persona-3 §2.2／§3）：只顯示必要資訊，技術字句只留紀錄。
 *
 * 由上而下：①狀態句（唯一）②已完成 X／Y 條 ③約還要 T ④停止鈕（stop-button 管）⑤重試／放慢時「已翻好的不會遺失」。
 * 不顯示批次、拆半、上下文、層數、同時處理數、HTTP 代碼、重試次數。純函式、不碰 DOM。
 */

export const REASSURE = "已翻好的不會遺失";
export const STOPPING_SENTENCE = "正在停止，寫出已翻好的部分…";
export const N07_TEXT = "記憶體不太夠，可能很慢或中途停下；可先關其他程式";

const BADGES = Object.freeze({
  throttled: "服務商要求放慢，已自動放慢",
  slow: "較慢，已自動放寬等待時間",
});

const COUNT_UNITS = new Set(["條", "句", "項", "筆"]);

function clean(text) {
  return String(text || "");
}

/** 後端狀態＋原始訊息 → 白話徽章（沒有就空字串）。 */
export function plainBadge(state, message = "") {
  const s = String(state || "");
  if (s === "throttled") return BADGES.throttled;
  if (isSlowMessage(message)) return BADGES.slow;
  return "";
}

function isSlowMessage(message) {
  return /等了\s*\d+\s*秒還沒回應|回應太慢/.test(clean(message));
}

/** 第一次出現就記下、之後整輪都顯示在次行的白話結論（一次）。 */
export function progressNotes(message) {
  const m = clean(message);
  const notes = [];
  if (/純 CPU/.test(m)) notes.push("沒有可用的顯示卡，會比較慢");
  if (/同時處理數從\s*\d+\s*降為\s*1/.test(m)) notes.push("記憶體較少，改成一次翻一批");
  return notes;
}

/** N-07 橫幅：本地模型記憶體不太夠（本輪結束收掉）。 */
export function needsMemoryBanner(message) {
  return /記憶體仍不夠/.test(clean(message));
}

/** 已完成 X／Y 條；單位不是條數（例如檔案、模組）時不顯示。 */
export function countLine(payload) {
  const p = payload && typeof payload === "object" ? payload : {};
  const done = Number(p.done);
  const total = Number(p.total);
  const unit = String(p.unit || "").trim();
  if (!Number.isFinite(done) || !Number.isFinite(total) || total <= 0 || !COUNT_UNITS.has(unit)) return "";
  return `已完成 ${Math.min(done, total).toLocaleString("en-US")}／${total.toLocaleString("en-US")} 條`;
}

/** 依完成比例與已進行時間推算剩餘時間；未滿 5% 寫「正在估算…」。 */
export function etaText({ percent = 0, elapsedMs = 0 } = {}) {
  const p = Number(percent) || 0;
  const elapsed = Math.max(0, Number(elapsedMs) || 0);
  if (p < 5 || elapsed <= 0) return "正在估算…";
  if (p >= 100) return "";
  const remainMin = Math.ceil((elapsed * (100 - p)) / p / 60000);
  if (remainMin <= 1) return "約還要不到 1 分鐘";
  if (remainMin < 60) return `約還要 ${remainMin} 分鐘`;
  const h = Math.floor(remainMin / 60);
  const m = remainMin % 60;
  return m ? `約還要 ${h} 小時 ${m} 分` : `約還要 ${h} 小時`;
}

/** 狀態句（唯一呈現）。 */
export function runSentence({ packName = "", stepIndex = -1, stepTotal = 5, message = "", state = "", stopping = false, localStartFirst = false } = {}) {
  if (stopping || state === "cancelling") return STOPPING_SENTENCE;
  const m = clean(message);
  if (/顯示記憶體不足以載入整個模型/.test(m)) return "顯示卡記憶體不夠，改用較穩的方式啟動";
  if (/本地模型啟動中|本地模型啟動設定|正在啟動本地模型/.test(m)) {
    return localStartFirst ? "正在啟動本地模型（第一次較久）" : "正在啟動本地模型";
  }
  const unanswered = m.match(/AI 沒回應的\s*(\d+)\s*句/);
  if (unanswered) return `再翻一次剛才沒回應的 ${unanswered[1]} 句`;
  const topup = m.match(/翻不好的\s*(\d+)\s*句，改用線上 AI 補完/);
  if (topup) return `用線上 AI 補完 ${topup[1]} 句（照你的設定）`;
  const waited = m.match(/已等\s*(\d+)\s*秒，最多等\s*(\d+)\s*秒/);
  if (waited) {
    const left = Math.max(1, Math.ceil((Number(waited[2]) - Number(waited[1])) / 60));
    return `連線中斷，自動重試中（最多再等約 ${left} 分鐘）`;
  }
  if (/連線中斷|自動重試中/.test(m) && (state === "retrying" || /連線中斷/.test(m))) return "連線中斷，自動重試中";
  const name = String(packName || "").trim() || "這個模組整合包";
  const total = Math.max(1, Number(stepTotal) || 5);
  const idx = Number(stepIndex);
  if (Number.isFinite(idx) && idx >= 0) return `正在翻「${name}」：第 ${Math.min(idx + 1, total)}／${total} 段`;
  return `正在翻「${name}」`;
}

/**
 * 翻譯中卡片的一次完整畫面。
 * @returns {{sentence: string, count: string, eta: string, badge: string, reassure: string, notes: string[]}}
 */
export function progressView(input) {
  const src = input && typeof input === "object" ? input : {};
  const payload = src.payload && typeof src.payload === "object" ? src.payload : {};
  const state = String(payload.state || "");
  const message = clean(src.message);
  const stopping = !!src.stopping || state === "cancelling";
  const sentence = runSentence({ ...src, message, state, stopping });
  const badge = stopping ? "" : plainBadge(state, message);
  const retrying = /連線中斷|自動重試中|剛才沒回應/.test(sentence);
  return {
    sentence,
    count: stopping ? "" : countLine(payload),
    eta: stopping ? "" : etaText({ percent: src.percent, elapsedMs: src.elapsedMs }),
    badge,
    reassure: !stopping && (retrying || !!badge) ? REASSURE : "",
    notes: Array.isArray(src.notes) ? src.notes.filter(Boolean) : [],
  };
}
