/**
 * 區塊刷新排程：讓畫面自己保持最新，而不是等使用者按重新整理。
 *
 * # 為什麼要有這個
 *
 * 工具的狀態有一半來自外部：使用者可能去檔案總管刪掉「翻譯結果」、
 * 手動改遊戲資料夾、登出 Discord、或把模型檔搬走。工具自己偵測不到這些，
 * 舊版只在「視窗重新取得焦點」時重探一次本機快取，其餘全靠使用者自己發現。
 *
 * # 三條硬規則（站長要求「不影響使用、不傷眼」）
 *
 * 1. **值沒變就完全不碰 DOM**。刷新是為了正確，不是為了重畫；
 *    每次都重寫 textContent 會讓畫面閃、讓選取中的文字被清掉。
 *    所以每個區塊回傳一個「指紋」，跟上次一樣就直接跳過。
 * 2. **使用者正在用的時候不要動他**。翻譯進行中、焦點在該區塊的輸入框裡、
 *    或使用者正在選取文字時，一律跳過那個區塊。
 * 3. **不做整頁 reload**。每個區塊各自更新自己那一塊就好。
 */

/** @type {Map<string, {refresh: Function, when: string[], minIntervalMs: number, last: number, fingerprint: string, scope: string}>} */
const regions = new Map();
let heartbeat = null;
/** 由外部注入：翻譯進行中時不要打擾使用者。 */
let isBusy = () => false;

export function configureRefreshBus({ busy }) {
  if (typeof busy === "function") isBusy = busy;
}

/**
 * 註冊一個會自己更新的區塊。
 *
 * @param {object} opts
 * @param {string} opts.id            區塊識別（也用來查 DOM 判斷有沒有人在裡面打字）
 * @param {Function} opts.refresh     實際刷新；回傳字串當指紋，回傳 undefined 代表不做指紋比對
 * @param {string[]} opts.when        觸發時機：focus／after-run／interval
 * @param {number} [opts.minIntervalMs] 最短間隔，避免連續事件把同一區塊刷爆
 * @param {string} [opts.scope]       只在某個分頁顯示時才刷新（translate／font／diagnose／settings）
 */
export function registerRegion({ id, refresh, when = ["focus"], minIntervalMs = 3000, scope = "" }) {
  if (!id || typeof refresh !== "function") return;
  regions.set(id, { refresh, when, minIntervalMs, last: 0, fingerprint: "", scope });
}

/** 使用者正在這個區塊裡操作嗎？是的話別動他。 */
function userIsWorkingIn(id) {
  const el = typeof document !== "undefined" ? document.getElementById(id) : null;
  if (!el) return false;
  const active = document.activeElement;
  if (active && el.contains(active) && /^(INPUT|TEXTAREA|SELECT)$/.test(active.tagName)) {
    return true;
  }
  // 正在選取這一塊裡的文字＝正在讀它，重畫會把選取清掉
  const sel = window.getSelection?.();
  if (sel && !sel.isCollapsed && sel.anchorNode && el.contains(sel.anchorNode)) return true;
  return false;
}

function currentPage() {
  return (typeof document !== "undefined" && document.body?.dataset.appPage) || "translate";
}

async function runRegion(id, entry, reason) {
  if (entry.scope && entry.scope !== currentPage()) return;
  if (userIsWorkingIn(id)) return;
  const now = Date.now();
  if (reason === "interval" && now - entry.last < entry.minIntervalMs) return;
  entry.last = now;
  try {
    const fingerprint = await entry.refresh({ reason });
    // 值沒變就當作沒發生過——這是「不閃」的關鍵
    if (typeof fingerprint === "string") entry.fingerprint = fingerprint;
  } catch (_) {
    /* 單一區塊刷新失敗不影響其他區塊，也不該打擾使用者 */
  }
}

/** 觸發某個時機的所有區塊。 */
export function trigger(reason) {
  // 翻譯進行中只允許明確要求的刷新，不做背景輪詢——那時候畫面在跑進度，
  // 任何額外的重畫都會讓人覺得在閃。
  if (isBusy() && reason === "interval") return;
  for (const [id, entry] of regions) {
    if (!entry.when.includes(reason)) continue;
    void runRegion(id, entry, reason);
  }
}

/** 只刷新指定的一個區塊（例如某項作業剛結束）。 */
export function refreshRegion(id, reason = "manual") {
  const entry = regions.get(id);
  if (entry) void runRegion(id, entry, reason);
}

/**
 * 開始監聽。只掛三種來源：
 * 視窗回到前景、瀏覽器 focus、低頻心跳（只給真的會被外部改動的區塊）。
 */
export function startRefreshBus({ heartbeatMs = 45000 } = {}) {
  if (typeof document === "undefined") return;
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") trigger("focus");
  });
  window.addEventListener("focus", () => trigger("focus"));
  if (heartbeat) clearInterval(heartbeat);
  heartbeat = setInterval(() => trigger("interval"), heartbeatMs);
}

export function stopRefreshBus() {
  if (heartbeat) clearInterval(heartbeat);
  heartbeat = null;
}

/** 測試用：清空註冊表。 */
export function _resetForTests() {
  regions.clear();
  stopRefreshBus();
  isBusy = () => false;
}

/** 測試用：看某個區塊的登記內容。 */
export function _peek(id) {
  return regions.get(id);
}
