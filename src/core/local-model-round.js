/**
 * B4 #8：本地模型「翻完自動關閉」的輪次編號。
 *
 * 每一輪翻譯（翻譯、修復、接續補完）開始時向後端拿一個新的輪次編號；結束時只關閉
 * 「自己這一輪」的模型。翻完立刻按「接續補完」時，第 1 輪遲到的關閉看到第 2 輪已經開始，
 * 就不關——否則會把第 2 輪正在用的模型關掉，補完中途失敗。後端也會再檢查一次（雙重保險）。
 *
 * 不碰 DOM，方便在 node 下測試。
 */

/**
 * @param {{
 *   invoke: (cmd: string, args?: object) => Promise<any>,
 *   isLocalMode: () => boolean,
 *   log: (msg: string) => void,
 *   onStopped?: () => void,
 *   markIdle?: () => Promise<void>,
 * }} deps
 */
export function createLocalModelRounds({ invoke, isLocalMode, log, onStopped, markIdle }) {
  let current = 0;

  /** 新一輪開始：回傳這一輪的編號（沒用本地模型時回 0）。 */
  async function begin() {
    if (!isLocalMode()) return 0;
    try {
      const round = Number(await invoke("local_llm_begin_round_cmd"));
      current = Number.isFinite(round) && round > 0 ? round : current + 1;
    } catch (_) {
      // 後端拿不到編號時前端自己往前推，至少擋住前端這一側的競態
      current += 1;
    }
    return current;
  }

  /** 這一輪結束：只有「還是這一輪」才請後端關閉本地模型。 */
  async function release(round) {
    if (!isLocalMode() || !round) return;
    if (round !== current) {
      log(`本地模型：第 ${round} 輪結束時不關閉，因為第 ${current} 輪已經開始使用它（會在第 ${current} 輪結束後關閉）。`);
      return;
    }
    try {
      // 審查 8：先確定後端知道「翻譯已結束」，再送關閉；否則後端可能以為還在翻而不關
      if (typeof markIdle === "function") await markIdle();
      const res = await invoke("local_llm_release_after_run_cmd", { round });
      if (res && res.message) log(String(res.message));
      if (res && res.stopped && typeof onStopped === "function") onStopped();
    } catch (_) {
      /* 關不掉不影響翻譯結果，也不值得打擾使用者 */
    }
  }

  return {
    begin,
    release,
    current: () => current,
    /** 只給測試用 */
    _setCurrentForTest(v) {
      current = v;
    },
  };
}
