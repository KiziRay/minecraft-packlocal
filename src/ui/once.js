/**
 * 單次執行守衛：同一件事正在跑時，重複觸發一律忽略。
 *
 * 為什麼需要：主要動作（開始翻譯／補充漏翻／建字體／分析／分享／安裝模型）都是
 * async handler。`setBusy(true)` 確實會鎖住按鈕，但那發生在第一個 `await` **之後**——
 * 在那之前的空窗（要驗證路徑、要探測快取、要問確認對話框）使用者是可以再點一下的，
 * 於是同一個昂貴動作被啟動兩次。這正是 rules/50 R50-4 記過的那類 bug：
 * 兩個下載程序互寫同一個檔案，症狀千奇百怪而且很難重現。
 *
 * 用法：
 *   bind("btn-run", () => runExclusive("run", onRun));
 */

const inFlight = new Map();

/** 超過這個時間仍未結束就強制釋放，避免某條路徑忘了收尾把按鈕永久卡死。 */
const STUCK_RELEASE_MS = 30 * 60 * 1000;

export function isRunning(key) {
  const started = inFlight.get(key);
  if (started == null) return false;
  if (Date.now() - started > STUCK_RELEASE_MS) {
    inFlight.delete(key);
    return false;
  }
  return true;
}

/**
 * 執行 fn；同 key 正在執行中就直接忽略這次呼叫。
 * 回傳 fn 的結果，被忽略時回傳 undefined。
 */
export async function runExclusive(key, fn, { onBusy } = {}) {
  if (isRunning(key)) {
    if (typeof onBusy === "function") onBusy();
    return undefined;
  }
  inFlight.set(key, Date.now());
  try {
    return await fn();
  } finally {
    // 一律用 finally 釋放：拋錯、取消、提早 return 都要能解鎖。
    inFlight.delete(key);
  }
}
