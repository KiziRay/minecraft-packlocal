/**
 * B5c 完成與套用的流程接線（app.js 只接線）：
 * - 一輪結束：整理結果、依包保存、N-05、成功套用後自動檢查並修好資源包清單（G1.21 規則不變）；
 * - S11「套用到遊戲」：只套「目前這包」的結果（套用前比對資料夾，換包不會套到上一包）；
 *   按下先查遊戲是否開著，開著就在按鈕旁寫原因（不跳對話框）；
 * - 複製資料夾被擋：D-10 確認後直接接著套用；
 * - 完成卡的次要動作：人工補翻、分享給朋友、開啟結果資料夾、併入用詞建議。
 * deps 由 app.js 注入（不 import app.js），測試用 mock。
 */

import { applyTargetCheck, pathKey } from "./pack-results.js";
import { RESULT_ACTION, fontBanner, isPartial, runPlanNote, summarizeProbe, summarizeRun } from "./result-card.js";

/**
 * 使用回饋 N-09（規格 §3.4）：非第一次完成、頻率同現況（送出後 21 天、提醒後 14 天、機率 0.2）。
 * @returns {boolean}
 */
export function feedbackNudgeDue({ completions = 0, lastSubmit = 0, lastNudge = 0, now = Date.now(), roll = Math.random() } = {}) {
  if (completions < 2) return false;
  const day = 24 * 60 * 60 * 1000;
  if (lastSubmit && now - lastSubmit < 21 * day) return false;
  if (lastNudge && now - lastNudge < 14 * day) return false;
  return roll <= 0.2;
}

export const FEEDBACK_BANNER = Object.freeze({ id: "N-09", text: "上次的翻譯還好嗎？花 30 秒告訴我們", actionLabel: "填寫" });

/**
 * @param {{
 *   store: ReturnType<import("./pack-results.js").createPackResults>,
 *   applyPending: {handle: Function, applyNow: Function},
 *   invoke: Function, appendLog: (t: string, level?: string) => void, sync: () => void,
 *   currentPath: () => string, probe: () => object|null, selectedOutputDir: () => string,
 *   packName: () => string|null, aiMode: () => string, onlineConfigured: () => boolean,
 *   disclosure: {isShown: (k: string) => boolean, retire: (k: string) => void},
 *   canShare: () => boolean, canMergeTerms: () => boolean, backupChoice: () => string,
 *   showBanner: (b: object) => void, offerFork: (path: string) => Promise<boolean>,
 *   openShare: () => void, openManualFix: () => void, openResultFolder: () => void,
 *   mergeTerms: () => void, openIssueReport: () => void, onApplied?: () => void,
 * }} deps
 */
export function createResultActions(deps) {
  const log = deps.appendLog || (() => {});
  const sync = () => deps.sync && deps.sync();
  const seenFontBanners = new Set();
  /**
   * 審查 1a：本機結果探測只屬於探測當時的遊戲資料夾（probe.ownerPath）。路徑不符就當沒有——
   * 換包（含手動輸入路徑）後、新探測回來前，不會用上一包的結果畫狀態卡或套用。
   */
  function probeFor(path) {
    const probe = deps.probe ? deps.probe() : null;
    if (!probe || !path) return null;
    return pathKey(probe.ownerPath) === pathKey(path) ? probe : null;
  }

  /** 完成卡已經完整展開過一次（離開這張卡或下一輪開始時，說明 result-reasons 退場）。 */
  let shownFull = false;
  function retireReasonsIfShown() {
    if (shownFull && deps.disclosure) deps.disclosure.retire("resultReasons");
    shownFull = false;
  }

  function showFontBanner(summary, path) {
    const banner = fontBanner(summary);
    if (!banner || !deps.showBanner) return;
    // 同包同一組資源包一次
    const key = `${path}|${banner.key}`;
    if (seenFontBanners.has(key)) return;
    seenFontBanners.add(key);
    deps.showBanner(banner);
  }

  /** 套用成功後檢查資源包清單；壞了就自動修（可逆：先另存設定檔；遊戲開著後端拒絕，G1.21）。 */
  async function autoRepairResourcePacks(path) {
    let report;
    try {
      report = await deps.invoke("verify_resource_packs_cmd", { instancePath: path });
    } catch (_) {
      return false;
    }
    const disabled = Array.isArray(report?.presentButDisabled) ? report.presentButDisabled : [];
    if (!report?.listEmpty && disabled.length === 0) return false;
    log(report.summary || "資源包清單可能不完整。", "warn");
    try {
      const fixed = await deps.invoke("repair_resource_packs_cmd", { instancePath: path });
      log((fixed?.summary || "已修好資源包清單。") + "（修改前已另存遊戲設定檔）");
      deps.store.markResourcePackRepaired(path);
      return true;
    } catch (e) {
      log("資源包清單沒有修好：" + String(e?.message || e), "warn");
      return false;
    }
  }

  /**
   * 一輪（翻譯／接續補完／修復／貼回翻譯後重新套用）結束：記下這包的結果；沒套用就走套用流程。
   * @returns {Promise<boolean>} 已套用到遊戲
   */
  async function finishRun(result, { origin = "run", instancePath, outputDir, packName = null, overwriteConfirmed = false, localModelClosed = false } = {}) {
    const path = String(instancePath || "").trim();
    retireReasonsIfShown();
    const summary = summarizeRun(result, { origin, finishedNow: true, localModelClosed, backupChoice: deps.backupChoice ? deps.backupChoice() : "" });
    const context = { instancePath: path, outputDir, packName };
    deps.store.record(path, summary, context);
    // 審查 6：只有完整跑完（不是中途停下）才算一次完成；回饋橫幅等完成卡收合或下次啟動才出
    if (!isPartial(summary) && deps.onCleanCompletion) deps.onCleanCompletion();
    showFontBanner(summary, path);
    sync();
    if (summary.applyStatus === "applied") {
      if (await autoRepairResourcePacks(path)) sync();
      return true;
    }
    return deps.applyPending.handle(result, context, 0, { overwriteConfirmed });
  }

  /** apply-pending 的回呼：套用成功（按「套用到遊戲」或 D-04／D-03 確認後）。 */
  async function onApplied(result, context) {
    const path = context && context.instancePath;
    if (!path) return;
    const applied = summarizeRun(result, { origin: "apply", finishedNow: true, backupChoice: deps.backupChoice ? deps.backupChoice() : "" });
    if (!deps.store.get(path)) {
      const base = probeFor(path) ? summarizeProbe(probeFor(path)) : applied;
      deps.store.record(path, { ...base, applyStatus: "applied" }, context);
    }
    deps.store.markApplied(path, applied);
    await autoRepairResourcePacks(path);
    if (deps.onApplied) deps.onApplied();
    sync();
  }

  function onPending(result, context) {
    const path = context && context.instancePath;
    if (!path) return;
    const summary = summarizeRun(result, { origin: "apply", finishedNow: true });
    if (!deps.store.get(path)) {
      const base = probeFor(path) ? summarizeProbe(probeFor(path)) : summary;
      deps.store.record(path, { ...base, applyStatus: summary.applyStatus, applyMessage: summary.applyMessage }, context);
    } else {
      deps.store.markPending(path, summary.applyStatus, summary.applyMessage, summary.pendingOverwrites);
    }
    sync();
  }

  function onFailed(reason, context) {
    const path = context && context.instancePath;
    if (!path) return;
    if (!deps.store.get(path)) deps.store.record(path, { ...(probeFor(path) ? summarizeProbe(probeFor(path)) : summarizeRun({})), applyStatus: "notApplied" }, context);
    deps.store.markApplyFailure(path, reason);
    sync();
  }

  /** 這一包的套用來源：這一輪留下的；沒有就用本機結果探測（S19a、S11 探測版）。 */
  function contextFor(path) {
    const saved = deps.store.pendingContext(path) || (deps.store.get(path) && deps.store.get(path).context);
    if (saved) return saved;
    // 沒有這一包自己的結果就沒有套用來源（不再猜「目前的結果位置」：那可能還是上一包的）
    const probe = probeFor(path);
    const outputDir = probe && (probe.outputDir || probe.output_dir);
    if (!outputDir) return null;
    const packName = probe.packName || probe.pack_name || probe.canonicalZip || probe.canonical_zip || null;
    return { instancePath: probe.ownerPath, outputDir, packName };
  }

  /** S11／S19a 主要按鈕「套用到遊戲」。 */
  async function apply() {
    const path = deps.currentPath();
    const context = contextFor(path);
    if (!context) {
      log("這個模組整合包在這台電腦沒有可套用的翻譯結果。", "warn");
      return false;
    }
    const check = applyTargetCheck(context, path);
    if (!check.ok) {
      log(check.reason, "warn");
      onFailed(check.reason, { ...context, instancePath: path });
      return false;
    }
    // 按下先檢查遊戲；仍開著就在按鈕旁寫「Minecraft 還開著」（不再跳「遊戲好像還開著」）
    let running = false;
    try {
      const verdict = await deps.invoke("is_game_running_cmd", { instancePath: path });
      running = !!(verdict && verdict.running);
    } catch (_) {
      running = false; // 查不到就放行（失效安全同現況）
    }
    // 審查 1b：查遊戲期間換了資料夾就停（不把這包的結果套到別處）
    if (pathKey(deps.currentPath()) !== pathKey(path)) return false;
    if (!deps.store.get(path)) {
      const base = probeFor(path) ? summarizeProbe(probeFor(path)) : summarizeRun({});
      deps.store.record(path, { ...base, applyStatus: "notApplied" }, context);
    }
    deps.store.setGameStillRunning(path, running);
    if (running) {
      sync();
      return false;
    }
    return deps.applyPending.applyNow(context);
  }

  /** S11（複製來的被擋）：D-10 確認當成新的模組整合包後，直接接著套用（規格 §2.3）。 */
  async function forkThenApply() {
    const path = deps.currentPath();
    if (!path) return false;
    const ok = await deps.offerFork(path);
    if (!ok) return false;
    return apply();
  }

  /** 換資料夾離開 prev：下次選到只剩一句。 */
  function onFolderChanged(prev) {
    const was = prev ? deps.store.summary(prev) : null;
    if (prev) deps.store.collapse(prev);
    if (was && was.fresh && deps.onCardCollapsed) deps.onCardCollapsed();
    retireReasonsIfShown();
  }

  /** computePackState 的 B5c 輸入。 */
  function stateInput(path, { busy = false } = {}) {
    const entry = deps.store.get(path);
    const probe = probeFor(path);
    const summary = entry ? entry.summary : null;
    return {
      result: summary,
      probeSummary: probe ? summarizeProbe(probe) : null,
      lastRunPlanNote: summary ? runPlanNote(summary) : "",
      resultCtx: {
        aiMode: deps.aiMode ? deps.aiMode() : "",
        onlineConfigured: deps.onlineConfigured ? !!deps.onlineConfigured() : false,
        firstTime: deps.disclosure ? deps.disclosure.isShown("resultReasons") : true,
        applyFailure: entry ? entry.applyFailure : "",
        gameStillRunning: entry ? entry.gameStillRunning : false,
        canShare: !busy && (deps.canShare ? !!deps.canShare() : false),
        canMergeTerms: deps.canMergeTerms ? !!deps.canMergeTerms() : false,
      },
    };
  }

  /** 完成卡畫過一次完整原因後：之後只寫條數（熟手；規格 §4.2 result-reasons）。 */
  function noteCardShown(state) {
    if (!state || !deps.disclosure) return;
    const full = Array.isArray(state.detailLines) && state.detailLines.some((l) => l === "已幫你做的事：" || l === "還是英文的部分：");
    if (full && ["S14", "S17"].includes(state.id)) shownFull = true;
  }

  /** 狀態卡動作。回 true＝這裡處理了。 */
  function onAction(action) {
    switch (action) {
      case RESULT_ACTION.apply:
        void apply();
        return true;
      case RESULT_ACTION.fork:
        void forkThenApply();
        return true;
      case RESULT_ACTION.manualFix:
        deps.openManualFix();
        return true;
      case RESULT_ACTION.share:
        deps.openShare();
        return true;
      case RESULT_ACTION.openResult:
        deps.openResultFolder();
        return true;
      case RESULT_ACTION.mergeTerms:
        deps.mergeTerms();
        return true;
      default:
        return false;
    }
  }

  /** 移除翻譯或刪除結果後：這包保存的完成結果作廢（狀態卡改說 S19，回來時不會再說已套用）。 */
  function forget(path) {
    if (path) deps.store.forget(path);
  }

  return { forget, finishRun, onApplied, onPending, onFailed, apply, forkThenApply, onFolderChanged, stateInput, noteCardShown, onAction, autoRepairResourcePacks, contextFor };
}
