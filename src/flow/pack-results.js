/**
 * B5c：完成結果與「已翻完、還沒套用」依模組整合包保存（規格 §2.1、P2-#14）。
 *
 * - 換資料夾時狀態卡整張換；回到原本那包，原本的完成卡／待套用再現。
 * - 套用前比對：待套用的結果屬於哪個遊戲資料夾（先用路徑比對；B6b 再加識別碼）。
 *   目前選的資料夾不是那一包 → 擋下並白話說明，絕不套到別包。
 * 純資料、不碰 DOM；只存在記憶體（關掉工具就回到用本機結果探測）。
 */

export function pathKey(path) {
  return String(path || "")
    .trim()
    .replace(/[\\/]+$/, "")
    .replace(/\\/g, "/")
    .toLowerCase();
}

function leaf(path) {
  const parts = String(path || "")
    .trim()
    .split(/[\\/]+/)
    .filter(Boolean);
  return parts.length ? parts[parts.length - 1] : "";
}

/**
 * 套用前的歸屬檢查（先用路徑比對）。
 * @returns {{ok: boolean, reason: string}}
 */
export function applyTargetCheck(context, currentPath) {
  const owner = context && context.instancePath;
  if (!owner) return { ok: false, reason: "找不到這份翻譯結果屬於哪個模組整合包，請重新翻譯" };
  if (!pathKey(currentPath)) return { ok: false, reason: "請先選擇遊戲資料夾" };
  if (pathKey(owner) !== pathKey(currentPath)) {
    return {
      ok: false,
      reason: `這份翻譯結果屬於「${leaf(owner)}」，不是「${leaf(currentPath)}」`,
    };
  }
  return { ok: true, reason: "" };
}

export function createPackResults() {
  /** key → {summary, context, applyFailure, gameStillRunning} */
  const entries = new Map();

  function entry(path) {
    return entries.get(pathKey(path)) || null;
  }

  return {
    /** 一輪結束（或套用結果回來）：記下這包的結果與套用來源。 */
    record(path, summary, context) {
      const key = pathKey(path);
      if (!key || !summary) return;
      const ctx = context ? { ...context, instancePath: context.instancePath || path } : { instancePath: path };
      entries.set(key, { summary: { ...summary }, context: ctx, applyFailure: "", gameStillRunning: false });
    },
    get(path) {
      return entry(path);
    },
    summary(path) {
      const e = entry(path);
      return e ? e.summary : null;
    },
    /** 待套用的來源（只有 applyStatus 不是 applied 時才有意義）。 */
    pendingContext(path) {
      const e = entry(path);
      if (!e) return null;
      return e.summary.applyStatus !== "applied" || e.applyFailure ? e.context : null;
    },
    /** 套用成功後更新：狀態改已套用，套用明細換成這次的。 */
    markApplied(path, applySummary) {
      const e = entry(path);
      if (!e) return;
      const next = { ...e.summary, applyStatus: "applied", applyMessage: "" };
      if (applySummary && applySummary.apply) next.apply = applySummary.apply;
      next.fresh = true;
      e.summary = next;
      e.applyFailure = "";
      e.gameStillRunning = false;
    },
    /** 套用又回「還沒套用」（遊戲開著等）：只換狀態與原因。 */
    markPending(path, applyStatus, message, pendingOverwrites = []) {
      const e = entry(path);
      if (!e) return;
      e.summary = { ...e.summary, applyStatus, applyMessage: String(message || ""), pendingOverwrites };
      e.applyFailure = "";
    },
    markApplyFailure(path, reason) {
      const e = entry(path);
      if (e) e.applyFailure = String(reason || "套用失敗");
    },
    setGameStillRunning(path, running) {
      const e = entry(path);
      if (e) e.gameStillRunning = !!running;
    },
    markResourcePackRepaired(path) {
      const e = entry(path);
      if (e) e.summary = { ...e.summary, resourcePackRepaired: true };
    },
    /** 換資料夾離開這包：下次選到只剩一句（完成卡收合）。 */
    collapse(path) {
      const e = entry(path);
      if (e) e.summary = { ...e.summary, fresh: false };
    },
    forget(path) {
      entries.delete(pathKey(path));
    },
  };
}
