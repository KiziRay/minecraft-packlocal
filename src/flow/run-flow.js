/**
 * B5b 開始前與翻譯中的流程狀態（app.js 只接線）：
 * - §3.1 列的草稿（備份、本地翻不好時）、「這次不用 AI」、重新翻譯進入確認模式、AI 閘門失敗的原動作；
 * - S12 出錯（依包保存，換資料夾整張換）；
 * - S09／S10 翻譯中畫面（進度、白話徽章、次行一次的結論、N-07）。
 * 不開任何對話框；deps 由 app.js 注入（不 import app.js），測試用 mock。
 */

import { AI_FIX, aiReadiness } from "./ai-readiness.js";
import { PRESTART_ACTION, prestartRows } from "./prestart.js";
import { classifyFailure } from "./run-failure.js";
import { needsMemoryBanner, progressNotes, progressView } from "./run-progress.js";
import { commitPrestartChoices } from "./run-start.js";

function same(a, b) {
  const n = (p) => String(p || "").trim().replace(/[\\/]+$/, "").replace(/\\/g, "/").toLowerCase();
  return !!n(a) && n(a) === n(b);
}

/**
 * @param {{
 *   instancePath: () => string,
 *   aiStatus: () => object|null, useAi: () => boolean, provider?: () => string, localNeedBytes?: () => number,
 *   backupChoice: () => string, cloudInfo?: () => ({needsConsent?: boolean, configured?: boolean}|null),
 *   gate: {ensure: (origin?: string) => Promise<any>},
 *   saveBackupChoice: (v: string) => Promise<unknown>, saveCloudChoice: (v: string) => Promise<unknown>,
 *   toast?: (t: string) => void, log?: (t: string, level?: string) => void,
 *   sync?: () => void, aiFix?: (action: string) => void, expandAi?: (open: boolean) => void,
 *   disclosure?: {retire: (k: string) => void, isShown: (k: string) => boolean},
 *   showBanner?: (b: object) => void, hideBanner?: (id: string) => void,
 * }} deps
 */
export function createRunFlow(deps) {
  const log = deps.log || (() => {});
  const sync = () => deps.sync && deps.sync();
  /** 「這次不用 AI」只對按下它的那張卡的動作有效："run"／"supplement"／"repair"；空字串＝沒按。 */
  let skipAiOnce = "";
  /** 本輪在 §3.1 選了不備份並勾了確認：這一輪套用不再跳 D-03（審查 4a）。 */
  let overwriteConfirmed = false;
  let backupDraft = { value: "always", ack: false };
  let cloudDraft = "0";
  let prestartOpen = false;
  /** 閘門失敗：{instancePath, origin, localStartFailed} */
  let aiBlock = null;
  /** S12：{instancePath, origin, classified} */
  let failure = null;
  let run = null;

  function readiness() {
    return aiReadiness({
      useAi: deps.useAi(),
      status: deps.aiStatus(),
      provider: deps.provider ? deps.provider() : "",
      localNeedBytes: deps.localNeedBytes ? deps.localNeedBytes() : 0,
      localDirRemembered: deps.localDirRemembered ? !!deps.localDirRemembered() : false,
    });
  }

  /** 目前這張卡的動作：AI-BLOCKED 的原動作，否則開始翻譯。 */
  function cardOrigin() {
    const block = aiBlock && same(aiBlock.instancePath, deps.instancePath()) ? aiBlock : null;
    return block && (!readiness().ready || skipAiOnce === block.origin) ? block.origin : "run";
  }

  function rows() {
    return prestartRows({
      ai: readiness(),
      skipAiOnce: skipAiOnce !== "" && skipAiOnce === cardOrigin(),
      cloud: deps.cloudInfo ? deps.cloudInfo() : null,
      cloudDraft,
      backupChoice: deps.backupChoice(),
      backupDraft,
    });
  }

  /** 狀態卡輸入（computePackState 的 prestart 欄位）。 */
  function prestartInput() {
    // 閘門失敗後 AI 已經就緒（玩家登入了）就回到原本的狀態；按了「這次不用 AI」仍留在原動作的卡（審查 1a）
    const origin = cardOrigin();
    const blocked = origin !== "run";
    const list = rows();
    return {
      // 接續補完／修復不經過 §3.1：只有 AI 列（審查 4b）
      rows: blocked ? list.filter((r) => r.id === "ai") : list,
      open: prestartOpen,
      aiBlockOrigin: blocked ? origin : "",
    };
  }

  function failureInput() {
    return failure && same(failure.instancePath, deps.instancePath()) ? failure : null;
  }

  /** 換資料夾：整張換（列的草稿保留——備份是全工具共用的選擇）。 */
  function onFolderChanged() {
    skipAiOnce = "";
    overwriteConfirmed = false;
    prestartOpen = false;
    aiBlock = null;
    failure = null;
  }

  /** 狀態卡動作。回 true＝這裡處理了。 */
  function onAction(action) {
    if (action === PRESTART_ACTION.skipAi) {
      skipAiOnce = cardOrigin();
      sync();
      return true;
    }
    if (action === PRESTART_ACTION.change) {
      if (deps.expandAi) deps.expandAi(true);
      return true;
    }
    if (action === PRESTART_ACTION.back) {
      prestartOpen = false;
      sync();
      return true;
    }
    if (Object.values(AI_FIX).includes(action)) {
      if (deps.aiFix) deps.aiFix(action);
      sync();
      return true;
    }
    return false;
  }

  /** §3.1 列的選擇（只改草稿；按開始時才記住）。 */
  function onRowChange(id, value, ack) {
    if (id === "backup") backupDraft = { value: value === "never" ? "never" : "always", ack: value === "never" ? !!ack : false };
    if (id === "cloud") cloudDraft = value === "1" ? "1" : "0";
    sync();
  }

  /**
   * 按主要按鈕「開始翻譯／重新翻譯」時：重新翻譯先進入確認模式（刻意多一步，R-8）。
   * @returns {boolean} true＝這次只打開確認模式，不開跑
   */
  function interceptRun(state) {
    if (state && state.reTranslate && !prestartOpen) {
      prestartOpen = true;
      sync();
      return true;
    }
    return false;
  }

  /**
   * 開跑前：AI 閘門（翻譯、修復、接續補完共用）。只判斷就緒與缺項，不啟動本地模型（G0.5：啟動在輪次開始之後）。
   * 不開任何對話框；沒通過就把原因交給狀態卡。§3.1 的選擇由 commitRunChoices 在三選一之後記住。
   * `skipAi`：同一次點擊從開始翻譯轉成接續補完（三選一）時明確帶入「這次不用 AI」。
   * @returns {Promise<{ok: boolean, useAi: boolean, gate: object}>}
   */
  async function beforeStart(origin, { skipAi = false } = {}) {
    const path = deps.instancePath();
    const current = rows();
    if (origin === "run" && current.some((r) => r.tone === "block" && r.id !== "ai")) {
      sync();
      return { ok: false, useAi: false, gate: null };
    }
    const skipHere = skipAi || skipAiOnce === origin;
    const gate = await deps.gate.ensure(origin, { skipAi: skipHere });
    if (!gate.ready) {
      aiBlock = { instancePath: path, origin };
      log(gate.sentence, "warn");
      sync();
      return { ok: false, useAi: false, gate };
    }
    aiBlock = null;
    failure = null;
    // 接續補完／修復：這次的「不用 AI」用掉了；開始翻譯的留到 commitRunChoices（三選一取消時卡上仍顯示）
    if (origin !== "run" && skipAiOnce === origin) skipAiOnce = "";
    return { ok: true, useAi: !!gate.useAi, gate };
  }

  /**
   * 開始翻譯真的要開跑時（三選一之後、setBusy 之前）：記住 §3.1 的選擇並 toast（審查 1d）。
   * 本輪在 §3.1 選了不備份並勾確認 → 這一輪套用不再跳 D-03（takeOverwriteConfirmed，審查 4a）。
   */
  async function commitRunChoices() {
    const list = rows();
    const backup = list.find((r) => r.id === "backup");
    overwriteConfirmed = !!(backup && backup.value === "never" && backup.ack && backup.ack.checked);
    await commitPrestartChoices(list, {
      saveBackupChoice: deps.saveBackupChoice,
      saveCloudChoice: deps.saveCloudChoice,
      toast: deps.toast || (() => {}),
      log: (m) => log(m, "warn"),
    });
    if (deps.disclosure) {
      deps.disclosure.retire("prestart");
      deps.disclosure.retire("aiChoice");
    }
    prestartOpen = false;
    skipAiOnce = "";
  }

  /**
   * 同一次點擊在三選一改成接續補完（第二輪審查 1b／1d）：先記住 §3.1 的選擇（備份列不會在開始後變成 D-04）、
   * 清掉「這次不用 AI」（改由呼叫端明確帶 skipAi），回傳本輪是否已確認不備份覆蓋（給接續補完的套用用）。
   */
  async function handOffToSupplement() {
    await commitRunChoices();
    return takeOverwriteConfirmed();
  }

  /** 套用時讀一次：本輪是否已在 §3.1 確認不備份覆蓋（讀完就清，下一輪沒勾照跳 D-03）。 */
  function takeOverwriteConfirmed() {
    const value = overwriteConfirmed;
    overwriteConfirmed = false;
    return value;
  }

  /** 開跑後（翻譯、修復、接續補完）失敗：S12。停止不是錯誤。 */
  function recordFailure(origin, text) {
    failure = { instancePath: deps.instancePath(), origin, classified: classifyFailure(text, origin) };
  }

  function clearFailure() {
    failure = null;
  }

  // ---- 翻譯中 ----
  function beginRun({ localMode = false } = {}) {
    run = {
      startedAt: Date.now(),
      notes: new Set(),
      message: "",
      payload: null,
      percent: 0,
      stepIndex: -1,
      stopping: false,
      localStartFirst: !!(localMode && deps.disclosure && deps.disclosure.isShown("localStart")),
      memoryBanner: false,
      localMode: !!localMode,
    };
  }

  function onProgress({ percent, message, payload, stepIndex } = {}) {
    if (!run) return;
    if (Number.isFinite(Number(percent))) run.percent = Math.max(run.percent, Number(percent));
    if (payload) run.payload = payload;
    if (Number.isFinite(Number(stepIndex))) run.stepIndex = Number(stepIndex);
    if (message) noteMessage(message);
  }

  /** 後端紀錄與進度訊息都經過這裡：次行結論記一次、N-07 出一次。技術原句只進紀錄（呼叫端照舊寫）。 */
  function noteMessage(message) {
    if (!run || !message) return;
    run.message = String(message);
    for (const n of progressNotes(message)) run.notes.add(n);
    if (!run.memoryBanner && needsMemoryBanner(message) && deps.showBanner) {
      run.memoryBanner = true;
      deps.showBanner({ id: "N-07", text: "記憶體不太夠，可能很慢或中途停下；可先關其他程式" });
    }
  }

  function markStopping() {
    if (run) run.stopping = true;
  }

  function endRun({ ok = false } = {}) {
    if (run && run.memoryBanner && deps.hideBanner) deps.hideBanner("N-07");
    if (run && ok && deps.disclosure) {
      deps.disclosure.retire("runTips");
      if (run.localMode) deps.disclosure.retire("localStart");
    }
    run = null;
  }

  function progressInput(packName) {
    if (!run) return null;
    return {
      ...progressView({
        packName,
        stepIndex: run.stepIndex,
        stepTotal: 5,
        percent: run.percent,
        elapsedMs: Date.now() - run.startedAt,
        payload: run.payload,
        message: run.message,
        stopping: run.stopping,
        localStartFirst: run.localStartFirst,
        notes: [...run.notes],
      }),
      stopping: run.stopping,
      percent: run.percent,
    };
  }

  return {
    readiness,
    rows,
    prestartInput,
    failureInput,
    onFolderChanged,
    onAction,
    onRowChange,
    interceptRun,
    beforeStart,
    commitRunChoices,
    handOffToSupplement,
    takeOverwriteConfirmed,
    recordFailure,
    clearFailure,
    beginRun,
    onProgress,
    noteMessage,
    markStopping,
    endRun,
    progressInput,
    isStopping: () => !!(run && run.stopping),
  };
}
