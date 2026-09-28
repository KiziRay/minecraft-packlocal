import { $, TAURI, dialog, emit, invoke, listen } from "./core/dom.js";
import {
  loadSettings,
  getSettingsHealthNotice,
  getSetting,
  setSetting,
  setSettingPath,
  getSettingPath,
  applyExternalSetting,
} from "./core/settings-store.js";
import {
  BACKUP_CHOICE_PATH,
  createApplyPendingFlow,
  isApplyPending,
  isBrokenRecordError,
  isForkableError,
  offerForkInstance,
  offerRecordReset,
} from "./ui/apply-pending.js";
import {
  BACKUP_STORAGE_KEY,
  CONSENT_STORAGE_KEY,
  CONSENT_STORAGE_KEY_LEGACY,
  FONT_PREFS_STORAGE_KEY,
  LAST_INSTANCE_PATH_KEY,
  OUTPUT_CUSTOM_ROOT_KEY,
  OUTPUT_STORAGE_MODE_KEY,
  THEME_STORAGE_KEY,
  USAGE_FEEDBACK_CLIENT_ID_KEY,
  USAGE_FEEDBACK_LAST_NUDGE_AT_KEY,
  USAGE_FEEDBACK_LAST_SUBMIT_AT_KEY,
} from "./core/storage.js";
import { forceRevealUi, prefersReducedMotion, revealInitialContent, revealPagePanel } from "./core/status.js";
import { isSupportedMinecraftVersion, unsupportedVersionMessage } from "./core/version.js";
import { GPT_COPY } from "./ai/copy.js";
import {
  ensureLocalLlmReady,
  forgetLocalLlmAfterExternalDelete,
  localLlmStatus,
  syncSetupButtonLabel,
  wireLocalLlm,
} from "./ai/local-llm.js";
import { createOnboarding } from "./onboarding/onboarding.js";
import { createSfxControls } from "./settings/sfx.js";
import { wireUpdateChecker } from "./core/update.js";
import { initWebviewScale, onScalePersisted, setWebviewAutoScale, setWebviewScalePercent } from "./ui-scale.js";
import { CLOUD_TOPUP_CONSENT } from "./core/cloud-topup-consent.js";
import {
  SETTINGS_ACTION_EVENT,
  SETTINGS_UPDATED_EVENT,
  routeSettingsAction,
  routeSettingsUpdate,
} from "./core/settings-sync.js";
import { wireHelpTips } from "./ui/help-tip.js";
import { wireTabKeys } from "./ui/tab-keys.js";
import { choiceDialog, confirmDialog, isConfirmOpen } from "./ui/confirm.js";
import { settingsHealthBanner } from "./ui/banner.js";
import { DISPLAY_CATEGORY, buildIssuePayload, describeIssueResult } from "./ui/issue-report.js";
import { isAriaDisabled } from "./flow/status-card.js";
import { createPackActions, describeImportReport } from "./flow/pack-actions.js";
import { createDisclosure } from "./flow/disclosure.js";
import {
  announceMainState,
  deleteResultsAfterApplyEnabled as readDeleteResultsSetting,
  isDataMigrating,
  wireSettingsNotices,
} from "./flow/settings-bridge.js";
import { CONSENT_CONTENT_VERSION, TOUR_SKIPPED_TOAST, afterConsent, isConsentAccepted } from "./flow/first-run.js";
import { createRemovalFlow } from "./flow/removal-flow.js";
import { runExclusive } from "./ui/once.js";
import { createLocalModelRounds } from "./core/local-model-round.js";
import { STOP_LABELS, resetStopButton } from "./ui/stop-button.js";
import {
  configureRefreshBus,
  registerRegion,
  refreshRegion,
  startRefreshBus,
} from "./core/refresh-bus.js";
import {
  clampStepIndexForward,
  formatCount,
  formatStepMeta,
  hiddenLogCount,
  progressLogDedupeKey,
  shortenProgressMessage,
  visibleLogLines,
} from "./ui-progress-logic.js";

const {
  startOnboarding,
  stopOnboarding,
  resumeOnboarding,
  layoutOnboarding,
  isOnboardingActive,
  previousOnboardingStep,
  nextOnboardingStep,
} = createOnboarding({
  $,
  closeGuideOverlaySafe,
  isInstanceReady: () => document.body.dataset.instanceReady === "1",
  // 跳過引導（Esc／跳過／點背景）時告訴玩家去哪找回（規格 §3.6）
  onSkipped: () => showAppToast(TOUR_SKIPPED_TOAST, 3000),
});
/** 說明漸進退場（規格 §4）：讀寫走設定檔（白名單 ui.disclosure.*）。 */
const disclosure = createDisclosure({
  read: (key) => getSetting(key, null),
  write: (key, value) => setSetting(key, value),
});
const { initSfxControls, applySfxPrefs, maybePlaySfxError, maybePlaySfxSuccess } = createSfxControls();

let latestAiStatus = null;
let issueReportIdempotencyKey = "";
let refreshAiStatusInFlight = null;
let gptStatusInFlight = null;
let gptStatusCache = null;
let gptStatusCheckedAt = 0;
let aiModeRevision = 0;
let discordLoginUrl = "";
let aiModeChangePromise = Promise.resolve();
let aiModeWriteChain = Promise.resolve();
let currentAiMode = "local";
let lastWorkbenchPage = "translate";
let translationState = "idle";
/**
 * 「complete」是怎麼來的：`run`＝這次真的跑過翻譯；`cache`＝只是開工具時探測到本機舊結果。
 *
 * 兩者以前共用同一個 translationState，於是「開工具、還原上次整合包、什麼都沒做」
 * 也會被當成剛完成一次翻譯——右欄停在「尚未開始」卻同時排了使用回饋彈窗。
 */
let resultSource = "none";
let pendingUsageFeedbackNudge = false;
let usageFeedbackDelayTimerId = 0;
let feedbackStep = 1;
let issueSubmitBusy = false;
/** 這次開啟的回報已經送出成功：送出鈕停用，避免重複開討論串。 */
let issueSubmitted = false;
let lastIssueStatus = "";
let shareConfirmationOpen = false;
let shareUploadInFlight = false;
let lastShareUrl = "";
let lastShareInstancePath = "";
let hasShareableFiles = false;
let shareableProbeToken = 0;
let stopRequestInFlight = false;
let sfxVolume = 0.55;
let sfxMuted = false;
let sfxAudioCtx = null;
let sfxLastPlayAt = 0;
let sfxLastErrorAt = 0;
let sfxLastSuccessAt = 0;
let apiKeyDraft = "";
let apiKeySavedMask = "";
/** 分頁的 roving tabindex 同步（wireTabKeys 接上後才有作用）。 */
let syncWorkbenchTabKeys = () => {};
let apiKeyEditing = false;
let gptLoginInFlight = false;
let hasApplyBackups = false;
let backupProbeToken = 0;
let backupProbeTimer = 0;
let translationHelperStatus = null;
let instanceValidation = { ok: false, reason: "尚未選擇遊戲資料夾。" };
/** 偵測到 Minecraft＜1.13 時為 true，禁用開始翻譯 */
let versionBlocked = false;
let versionBlockReason = "";
let coverageSkippedSeen = new Set();
let coverageMetrics = {
  glossary: 0,
  tm: 0,
  shared: 0,
  ai: 0,
  pending: null,
  packPending: null,
  qualitySkipped: 0,
  staysUnchanged: 0,
  prior: 0,
  skipped: 0,
  batchDone: null,
  batchTotal: null,
  batchRetry: null,
  batchRetryBatches: null,
  batchFail: null,
  cacheHitTokens: null,
  cacheMissTokens: null,
  completionTokens: null,
  cacheHitPercent: null,
  coveragePercent: null,
  summary: "尚未開始",
};
/** 主譯結算後鎖定命中明細，忽略隊列／後期免費命中覆寫 */
let coverageSettlementLocked = false;
/**
 * 進階統計的「先前已結束階段」累計基準。
 *
 * 「翻譯」「補充漏翻」「修復工作階段」各自呼叫後端的 `Engine::connect()`，各自建立
 * 一個全新的用量計數器——這是合理的連線生命週期設計，不是 bug。bug 在前端：
 * 新階段回報的（較小的）數字直接蓋掉畫面上（較大的）舊數字，看起來像統計被清空重來。
 * 這裡把「結束的階段」的最終值先併進基準，畫面顯示永遠是「基準＋目前階段」，
 * 除非使用者按下「開始翻譯」開一輪全新的翻譯，才把基準也歸零。
 */
let coverageCarryBase = {
  glossary: 0,
  tm: 0,
  shared: 0,
  ai: 0,
  skipped: 0,
  qualitySkipped: 0,
  staysUnchanged: 0,
  prior: 0,
  cacheHitTokens: 0,
  cacheMissTokens: 0,
  completionTokens: 0,
};
const COVERAGE_CARRY_FIELDS = Object.keys(coverageCarryBase);
/** 顯示值＝先前已結束階段的基準＋目前階段。內部的 coverageMetrics 本身維持只放「目前這一階段」，
 * 這樣 Math.max 比對與下一次 carryForward 都還是拿「這階段自己的值」，不會被基準污染。 */
function carried(field) {
  return (Number(coverageCarryBase[field]) || 0) + (Number(coverageMetrics[field]) || 0);
}
const CONTENT_FADE_MS = 260;
let startupContentRevealed = false;
let pageTransitionToken = 0;
let lastProgressPayload = null;
let lastActiveStepKey = null;
let lastActiveStepTotal = null;
let pendingProgressPayload = null;
let pendingLogRender = false;
let uiFlushTimer = 0;
let uiFlushRaf = 0;
let lastUiFlushAt = 0;
const UI_FLUSH_MS = 100;


function clearVersionBlock() {
  versionBlocked = false;
  versionBlockReason = "";
}

function setVersionBlock(reason) {
  versionBlocked = true;
  versionBlockReason = reason || "Minecraft 版本過舊，無法翻譯。";
}


/**
 * `carryForward=true`（補充漏翻／修復工作階段）：目前累計值先併入基準再歸零當前階段，
 * 畫面顯示的數字不會下降，只是換一個新階段繼續往上加。
 * `carryForward=false`（開始翻譯，預設）：基準與當前值都清零，這才是真正的新翻譯。
 */
function resetCoverageMetrics(summary = "等待翻譯開始", { carryForward = false } = {}) {
  if (carryForward) {
    for (const key of COVERAGE_CARRY_FIELDS) {
      coverageCarryBase[key] += Number(coverageMetrics[key]) || 0;
    }
  } else {
    for (const key of COVERAGE_CARRY_FIELDS) coverageCarryBase[key] = 0;
  }
  coverageSkippedSeen = new Set();
  coverageSettlementLocked = false;
  coverageMetrics = {
    glossary: 0,
    tm: 0,
    shared: 0,
    ai: 0,
    pending: null,
    packPending: null,
    qualitySkipped: 0,
    staysUnchanged: 0,
    prior: 0,
    skipped: 0,
    batchDone: null,
    batchTotal: null,
    batchRetry: null,
    batchRetryBatches: null,
    batchFail: null,
    cacheHitTokens: null,
    cacheMissTokens: null,
    completionTokens: null,
    cacheHitPercent: null,
    coveragePercent: null,
    summary,
  };
  renderCoverageMetrics();
}

function renderCoverageMetrics() {
  const setText = (id, value) => {
    const el = $(id);
    if (el) el.textContent = String(value);
  };
  setText("metric-glossary", carried("glossary"));
  setText("metric-tm", carried("tm"));
  setText("metric-shared", carried("shared"));
  setText("metric-ai", carried("ai"));
  setText("metric-skipped", carried("skipped"));
  setText("metric-quality-skipped", carried("qualitySkipped"));
  setText("metric-prior", carried("prior"));
  setText("metric-pending", coverageMetrics.pending == null ? "—" : coverageMetrics.pending);
  setText("metric-pack-pending", coverageMetrics.packPending == null ? "—" : coverageMetrics.packPending);
  if (coverageMetrics.coveragePercent != null) {
    let coverageEl = $("metric-coverage");
    if (!coverageEl) {
      const summaryEl = $("metric-summary");
      if (summaryEl?.parentElement) {
        coverageEl = document.createElement("p");
        coverageEl.id = "metric-coverage";
        coverageEl.className = "metric-coverage";
        summaryEl.parentElement.insertBefore(coverageEl, summaryEl.nextSibling);
      }
    }
    if (coverageEl) coverageEl.textContent = `可玩文字覆蓋率 ${coverageMetrics.coveragePercent}%（估算）`;
  }
  const batchText =
    coverageMetrics.batchDone != null && coverageMetrics.batchTotal != null
      ? `${formatCount(coverageMetrics.batchDone)} / ${formatCount(coverageMetrics.batchTotal)}${
          coverageMetrics.batchRetry != null || coverageMetrics.batchFail != null
            ? ` (+${formatCount(coverageMetrics.batchRetry || 0)} retry / ${formatCount(coverageMetrics.batchFail || 0)} fail)`
            : ""
        }`
      : "—";
  setText("metric-batches", batchText);
  setText(
    "metric-cache-rate",
    coverageMetrics.cacheHitPercent == null ? "—" : `${coverageMetrics.cacheHitPercent}%`
  );
  // token 三格：舊版直接拿當前階段的值蓋掉畫面，跨階段（翻譯→補充漏翻）數字會往下掉；
  // 現在一律加回先前階段的基準，「目前有沒有回報過」才用 == null 判斷，基準本身有就不該是 "—"。
  const tokenHitKnown = coverageMetrics.cacheHitTokens != null || coverageCarryBase.cacheHitTokens > 0;
  const tokenMissKnown = coverageMetrics.cacheMissTokens != null || coverageCarryBase.cacheMissTokens > 0;
  const tokenOutKnown = coverageMetrics.completionTokens != null || coverageCarryBase.completionTokens > 0;
  setText("metric-token-hit", tokenHitKnown ? formatCount(carried("cacheHitTokens")) : "—");
  setText("metric-token-miss", tokenMissKnown ? formatCount(carried("cacheMissTokens")) : "—");
  setText("metric-token-out", tokenOutKnown ? formatCount(carried("completionTokens")) : "—");
  const translated = carried("glossary") + carried("tm") + carried("shared") + carried("ai");
  // 摘要一律以四格加總為準（含 finalHit lock 後 aiHit 再更新），pending 另顯示於待補格
  if (translated > 0) {
    const summary = `已命中／補譯 ${translated} 條`;
    coverageMetrics.summary = summary;
    setText("metric-summary", summary);
  } else {
    setText("metric-summary", coverageMetrics.summary || "等待資料");
  }
  const countEl = $("prog-count");
  if (countEl) {
    const roundPending = coverageMetrics.pending;
    const packPending = coverageMetrics.packPending;
    if (roundPending != null || packPending != null) {
      const roundNum = roundPending == null ? 0 : Number(roundPending) || 0;
      const packNum = packPending == null ? roundNum : Number(packPending) || 0;
      const totalBase = packPending != null ? packNum : roundNum;
      const total = translated + totalBase;
      let pendingNote = "";
      if (roundPending != null && packPending != null) {
        pendingNote = `（本輪待補 ${formatCount(roundNum)} · 全包待補 ${formatCount(packNum)}）`;
      } else if (roundPending != null) {
        pendingNote = `（本輪待補 ${formatCount(roundNum)}）`;
      } else if (packPending != null) {
        pendingNote = `（全包待補 ${formatCount(packNum)}）`;
      }
      // 「待補」不等於「沒翻到」，而且裡面混了兩種完全不同的東西：
      //  1. 原樣保留：附魔等級的羅馬數字、單位符號、字型圖示、品牌名——維持原文
      //     才是正確結果（實測某整合包 1361 條待補裡有 863 條屬於這類，佔 63%）
      //  2. 品質未通過：AI 有回覆但沒過檢查，暫時保留原文
      // 兩者分開講，使用者才不會把「本來就不該翻」誤會成「工具漏翻」。
      const staysUnchanged = carried("staysUnchanged");
      const qualitySkippedSoFar = carried("qualitySkipped");
      const asides = [];
      if (staysUnchanged > 0) {
        asides.push(`${formatCount(staysUnchanged)} 項是附魔等級、單位符號、品牌名等原樣保留才正確`);
      }
      if (qualitySkippedSoFar > 0) {
        asides.push(`${formatCount(qualitySkippedSoFar)} 句品質未通過、暫時保留原文`);
      }
      if (packNum > 0 && asides.length > 0) {
        pendingNote += `；另有 ${asides.join("、")}`;
      }
      countEl.textContent = `已翻譯 ${formatCount(translated)} / ${formatCount(total)} 條目${pendingNote}`;
    } else if (translated > 0) {
      countEl.textContent = `已翻譯 ${formatCount(translated)} 條目`;
    } else {
      countEl.textContent = coverageMetrics.summary || "尚未開始";
    }
  }
  const summaryEl = $("metric-summary");
  if (summaryEl && coverageMetrics.pending != null && coverageMetrics.packPending != null) {
    const base = summaryEl.textContent.replace(/\s*·\s*本輪待補.*$/, "").trim();
    const roundNote = `本輪待補 ${formatCount(coverageMetrics.pending)} · 全包待補 ${formatCount(coverageMetrics.packPending)}`;
    if (!base.includes("本輪待補")) {
      summaryEl.textContent = base ? `${base} · ${roundNote}` : roundNote;
    }
  }
}

function appendCoverageCompletionTip() {
  const note = "若遊戲仍英文，查看報告港繁／待譯欄。";
  if (coverageMetrics.summary && !coverageMetrics.summary.includes("港繁")) {
    coverageMetrics.summary += ` · ${note}`;
  }
  const noteEl = document.querySelector(".metric-note");
  if (noteEl && !noteEl.textContent.includes("港繁")) {
    noteEl.textContent = `${noteEl.textContent.replace(/\s*$/, "")} ${note}`;
  }
}

function consumeCoverageMessage(message) {
  const text = String(message || "");
  if (!text) return;
  let changed = false;
  // 忽略「AI 翻譯 N 句（已扣掉…）」隊列文案，避免覆寫主結算
  if (/AI 翻譯\s+\d+\s+句（已扣掉/.test(text)) {
    return;
  }
  const libFillHit = text.match(
    /補譯\s+(\d+)\s+條（術語\s+(\d+)［庫術語\s+(\d+)］、庫文庫\s+(\d+)、共享術語\s+(\d+)、共享庫\s+(\d+)、翻譯記憶\s+(\d+)、AI\s+(\d+)）/
  );
  if (libFillHit) {
    coverageMetrics.glossary = Math.max(coverageMetrics.glossary, Number(libFillHit[2]) || 0);
    coverageMetrics.shared = Math.max(
      coverageMetrics.shared,
      (Number(libFillHit[5]) || 0) + (Number(libFillHit[6]) || 0)
    );
    coverageMetrics.tm = Math.max(coverageMetrics.tm, Number(libFillHit[7]) || 0);
    coverageMetrics.ai = Math.max(coverageMetrics.ai, Number(libFillHit[8]) || 0);
    coverageMetrics.summary = `已命中／補譯 ${libFillHit[1]} 條`;
    coverageSettlementLocked = true;
    appendCoverageCompletionTip();
    changed = true;
  }
  const finalHit = text.match(/補譯\s+(\d+)\s+條（術語表\s+(\d+)、共享庫\s+(\d+)、翻譯記憶\s+(\d+)、AI\s+(\d+)）/);
  if (finalHit) {
    coverageMetrics.glossary = Math.max(coverageMetrics.glossary, Number(finalHit[2]) || 0);
    coverageMetrics.shared = Math.max(coverageMetrics.shared, Number(finalHit[3]) || 0);
    coverageMetrics.tm = Math.max(coverageMetrics.tm, Number(finalHit[4]) || 0);
    coverageMetrics.ai = Math.max(coverageMetrics.ai, Number(finalHit[5]) || 0);
    coverageMetrics.summary = `已命中／補譯 ${finalHit[1]} 條`;
    coverageSettlementLocked = true;
    appendCoverageCompletionTip();
    changed = true;
  }
  const freeHit = text.match(
    /免費命中\s+(\d+)\s+句（術語表\s+(\d+)、共享庫\s+(\d+)、翻譯記憶\s+(\d+)）.*?只剩\s+(\d+)\s+句/
  );
  if (freeHit && !coverageSettlementLocked) {
    coverageMetrics.glossary = Math.max(coverageMetrics.glossary, Number(freeHit[2]) || 0);
    coverageMetrics.shared = Math.max(coverageMetrics.shared, Number(freeHit[3]) || 0);
    coverageMetrics.tm = Math.max(coverageMetrics.tm, Number(freeHit[4]) || 0);
    coverageMetrics.pending = Number(freeHit[5]) || coverageMetrics.pending;
    coverageMetrics.summary = `免費命中 ${freeHit[1]} 句`;
    changed = true;
  }
  const sharedHit = text.match(/社群共享庫命中\s+(\d+)\s+條/);
  if (sharedHit && !coverageSettlementLocked) {
    coverageMetrics.shared = Math.max(coverageMetrics.shared, Number(sharedHit[1]) || 0);
    changed = true;
  }
  const aiHit = text.match(/AI\s+(?:新補|新譯)\s*(?:約\s*)?(\d+)\s*(?:條|句)/);
  if (aiHit) {
    coverageMetrics.ai = Math.max(coverageMetrics.ai, Number(aiHit[1]) || 0);
    changed = true;
  }
  const batchHit = text.match(/(\d+)\s*[／/]\s*(\d+)\s*批/);
  if (batchHit) {
    coverageMetrics.batchDone = Number(batchHit[1]) || coverageMetrics.batchDone;
    coverageMetrics.batchTotal = Number(batchHit[2]) || coverageMetrics.batchTotal;
    changed = true;
  }
  const retryFailHit =
    text.match(/重試\s+(\d+)（(\d+)\s*批）\s*·\s*批失敗\s+(\d+)/) ||
    text.match(/重試\s+(\d+)（(\d+)\s*批）\s*·\s*失敗\s+(\d+)/);
  if (retryFailHit) {
    coverageMetrics.batchRetry = Number(retryFailHit[1]) || coverageMetrics.batchRetry;
    coverageMetrics.batchRetryBatches =
      Number(retryFailHit[2]) || coverageMetrics.batchRetryBatches;
    coverageMetrics.batchFail = Number(retryFailHit[3]) || coverageMetrics.batchFail;
    changed = true;
  }
  const tokenHit =
    text.match(/token\s+命中\s+(\d+)／未命中\s+(\d+)／輸出\s+(\d+)/) ||
    text.match(/AI token：快取命中\s+(\d+)、未命中\s+(\d+)、輸出\s+(\d+)/);
  if (tokenHit) {
    const hit = Number(tokenHit[1]) || 0;
    const miss = Number(tokenHit[2]) || 0;
    coverageMetrics.cacheHitTokens = hit;
    coverageMetrics.cacheMissTokens = miss;
    coverageMetrics.completionTokens = Number(tokenHit[3]) || coverageMetrics.completionTokens;
    const total = hit + miss;
    if (total > 0) coverageMetrics.cacheHitPercent = Math.floor((hit * 100) / total);
    changed = true;
  }
  const pendingHit = text.match(/(?:剩餘|仍缺|待補|尚可 AI 補|尚待本機資料或手動翻譯)(?:英文)?(?:約)?\s*(\d+)\s*條/);
  if (pendingHit && !/【仍待譯】/.test(text)) {
    coverageMetrics.pending = Number(pendingHit[1]) || 0;
    changed = true;
  }
  const qualityHit =
    text.match(/品質未過略過\s*(\d+)\s*句/) ||
    text.match(/(\d+)\s*條因品質未過保留英文/);
  if (qualityHit) {
    coverageMetrics.qualitySkipped = Math.max(
      coverageMetrics.qualitySkipped,
      Number(qualityHit[1]) || 0
    );
    changed = true;
  }
  const priorHit = text.match(/接續上次[：:]\s*本機累計併入\s*(\d+)\s*條|累計併入\s*(\d+)\s*條/);
  if (priorHit) {
    coverageMetrics.prior = Math.max(
      coverageMetrics.prior,
      Number(priorHit[1] || priorHit[2]) || 0
    );
    changed = true;
  }
  const packPendingHit = text.match(/【仍待譯】約\s*(\d+)\s*條/);
  if (packPendingHit) {
    coverageMetrics.packPending = Number(packPendingHit[1]) || 0;
    changed = true;
  }
  for (const match of text.matchAll(/完整度略過：([^；\n]+)/g)) {
    coverageSkippedSeen.add(match[0]);
    coverageMetrics.skipped = coverageSkippedSeen.size;
    changed = true;
  }
  if (/覆蓋範圍說明|本次來源明細|任務|覆寫|Origins|KubeJS|ZIP 文字/.test(text)) {
    coverageMetrics.summary = coverageMetrics.summary === "尚未開始" ? "來源統計更新中" : coverageMetrics.summary;
    changed = true;
  }
  if (changed) renderCoverageMetrics();
}

/** 同意頁本版一次（規格 D-01）：存的是同意內容的版本，Esc 不算同意。 */
function hasHiddenConsentOverlay() {
  try {
    return isConsentAccepted(getSetting(CONSENT_STORAGE_KEY, null));
  } catch (_) {
    return false;
  }
}

function showConsentOverlay() {
  if (hasHiddenConsentOverlay()) return false;
  const ov = $("consent-overlay");
  if (!ov) return false;
  ov.hidden = false;
  ov.setAttribute("aria-hidden", "false");
  return true;
}

/** 只由「我了解，開始使用」呼叫（Esc 與點背景都不會走到這裡）。按下後接新手引導。 */
function hideConsentOverlay() {
  const ov = $("consent-overlay");
  if (!ov) return;
  setSetting(CONSENT_STORAGE_KEY, CONSENT_CONTENT_VERSION);
  ov.hidden = true;
  ov.setAttribute("aria-hidden", "true");
  syncUiState();
  if (afterConsent({ tourSeen: false }) === "tour") {
    // startOnboarding 自己會判斷看過沒；看過就什麼都不做
    window.setTimeout(() => startOnboarding({ force: false }), 60);
  }
  packActions.maybeShowUpdateBanner();
}

/** 說明已整合進設定頁，沒有浮層要關；保留給新手引導呼叫，避免它需要知道這件事。 */
function closeGuideOverlaySafe() {
  document.body.classList.remove("guide-reader-open");
}

function getCurrentTauriWindow() {
  try {
    const api = (TAURI && TAURI.window) || (window.__TAURI__ && window.__TAURI__.window);
    if (!api) return null;
    if (typeof api.getCurrentWindow === "function") return api.getCurrentWindow();
    if (typeof api.getCurrent === "function") return api.getCurrent();
    return api.appWindow || null;
  } catch (_) {
    return null;
  }
}

function initWinbarChrome() {
  const bar = document.querySelector(".winbar");
  if (!bar || bar.dataset.chromeBound === "1") return;
  bar.dataset.chromeBound = "1";

  const minBtn = $("btn-win-min");
  const maxBtn = $("btn-win-max");
  const closeBtn = $("btn-win-close");
  if (minBtn) {
    minBtn.onclick = () => {
      const w = getCurrentTauriWindow();
      if (w && w.minimize) w.minimize().catch(() => {});
    };
  }
  async function syncMaxIcon() {
    const w = getCurrentTauriWindow();
    if (!w || !maxBtn || !w.isMaximized) return;
    try {
      const m = await w.isMaximized();
      maxBtn.textContent = m ? "❐" : "□";
      maxBtn.title = m ? "還原" : "最大化";
    } catch (_) {
      /* ignore */
    }
  }
  if (maxBtn) {
    maxBtn.onclick = () => {
      const w = getCurrentTauriWindow();
      if (w && w.toggleMaximize) {
        w.toggleMaximize().catch(() => {});
        setTimeout(syncMaxIcon, 140);
      }
    };
  }
  if (closeBtn) {
    closeBtn.onclick = () => {
      const w = getCurrentTauriWindow();
      if (w && w.close) w.close().catch(() => {});
    };
  }

  let dragTimer = null;
  let dragArmed = false;
  const noDrag = (t) =>
    t && t.closest && t.closest("button,a,input,select,textarea,[contenteditable],.wb-btn,.winbar-nodrag,.overflow-wrap");
  const cancelDrag = () => {
    if (dragTimer) {
      clearTimeout(dragTimer);
      dragTimer = null;
    }
  };
  bar.addEventListener(
    "mousedown",
    (e) => {
      if (e.button !== 0 || noDrag(e.target)) return;
      if (e.detail >= 2) {
        cancelDrag();
        dragArmed = false;
        return;
      }
      cancelDrag();
      dragArmed = true;
      const w = getCurrentTauriWindow();
      if (!w || !w.startDragging) return;
      dragTimer = setTimeout(() => {
        dragTimer = null;
        if (!dragArmed) return;
        try {
          w.startDragging();
        } catch (_) {
          /* ignore */
        }
      }, 140);
    },
    true
  );
  window.addEventListener(
    "mouseup",
    () => {
      dragArmed = false;
      cancelDrag();
    },
    true
  );
  bar.addEventListener(
    "dblclick",
    (e) => {
      dragArmed = false;
      cancelDrag();
      if (noDrag(e.target)) return;
      e.preventDefault();
      const w = getCurrentTauriWindow();
      if (w && w.toggleMaximize) {
        w.toggleMaximize().catch(() => {});
        setTimeout(syncMaxIcon, 140);
      }
    },
    true
  );

  document.querySelectorAll(".rsz[data-resize]").forEach((el) => {
    el.addEventListener("mousedown", (e) => {
      const dir = el.getAttribute("data-resize");
      const w = getCurrentTauriWindow();
      if (!w || !w.startResizeDragging || !dir) return;
      try {
        e.preventDefault();
      } catch (_) {
        /* ignore */
      }
      w.startResizeDragging(dir).catch(() => {});
    });
  });
  syncMaxIcon();
}

/**
 * 要不要備份改由設定檔的 translate.backupChoice 決定（第一次套用時後端回「需要詢問」，
 * 由 applyPending 問一次並寫回設定）。這裡固定回 true＝「照設定」；
 * 只有開始翻譯時選「不保留、直接覆蓋」會傳 false。
 */
function shouldBackupBeforeApply() {
  return true;
}

/** 翻譯完成但還沒裝進遊戲時的後續處理（備份選擇、覆蓋確認、遊戲開著、還沒啟動過遊戲）。 */
const applyPending = createApplyPendingFlow({
  $,
  invoke,
  confirmDialog,
  choiceDialog,
  appendLog: (text, level) => appendLog(text, level),
  setBusy: (busy, kind) => setBusy(busy, kind),
  saveBackupChoice: async (value) => {
    await setSettingPath(BACKUP_CHOICE_PATH, value);
    void Promise.resolve(
      emit(SETTINGS_UPDATED_EVENT, { path: BACKUP_CHOICE_PATH, value, source: "main" })
    ).catch(() => {});
  },
  onApplied: async () => {
    setTranslationState("complete");
    await refreshBackupState();
  },
});

/** 移除翻譯（D-05）與刪除結果並重翻（D-08）：問→做→回結果，畫面由狀態卡畫。 */
const removalFlow = createRemovalFlow({
  invoke,
  confirmDialog,
  ensureGameClosed: (path, label) => ensureGameClosed(path, label),
  formatError: (e) => formatInvokeError(e),
  onBusy: (on) => setBusy(on, "apply"),
});

/**
 * B5a-1 流程接線（狀態卡、D 區、S19、刪除結果並重翻、橫幅、浮層焦點）在 src/flow/pack-actions.js；
 * 這裡只把 app.js 的狀態與既有函式交給它。
 */
const packActions = createPackActions({
  $,
  doc: document,
  invoke,
  getSetting,
  setSetting,
  disclosure,
  removalFlow,
  getState: () => ({
    consentAccepted: hasHiddenConsentOverlay(),
    validation: instanceValidation,
    versionBlocked,
    versionBlockReason,
    progressBusy,
    shareUploadInFlight,
    busyJobKind: window.__busyJobKind || "",
    dataMigrating: isDataMigrating(),
    localCacheProbe,
    translationState,
    hasApplyBackups,
  }),
  syncUiState: () => syncUiState(),
  onPickInstance: () => onPickInstance(),
  onRun: () => onRun(),
  appendLog: (text, level) => appendLog(text, level),
  appendError: (text) => appendError(text),
  log: (text) => log(text),
  formatInvokeError: (e) => formatInvokeError(e),
  offerRecordResetIfBroken: (e, path) => offerRecordResetIfBroken(e, path),
  selectedOutputDir: () => selectedOutputDir(),
  resultWorkDir: (dir) => resultWorkDir(dir),
  refreshBackupState: () => refreshBackupState(),
  setTranslationState: (state) => setTranslationState(state),
  hideLocalCacheCard: () => hideLocalCacheCard(),
  clearShareableFiles: () => {
    hasShareableFiles = false;
  },
  applyCachedTranslation: () => applyCachedTranslation(),
  pathLeaf: (path) => pathLeaf(path),
  stopOnboarding: (markSeen, opts) => stopOnboarding(markSeen, opts),
  hideIssueOverlay: () => hideIssueOverlay(),
  closeMoreDrawer: () => closeMoreDrawer(),
  refreshIssueReportUi: () => refreshIssueReportUi(),
  openIssueDiscord: () => openIssueDiscord(),
  markWired: (el) => markWired(el),
});

function applyContextFromUi(outputDir) {
  return {
    instancePath: ($("instance")?.value || "").trim(),
    outputDir,
    packName: packNameForTranslate() || null,
  };
}

/** 套用後若後端偵測到同模組內容的其他資料夾，這裡統一抽出提醒文字。 */
function siblingInstanceWarning(result) {
  return (result && (result.siblingInstanceWarning || result.sibling_instance_warning)) || "";
}

function loadBackupPreference() {
  const input = $("backup-before-apply");
  if (!input) return;
  try {
    const saved = localStorage.getItem(BACKUP_STORAGE_KEY);
    if (saved === "0" || saved === "1") input.checked = saved === "1";
  } catch (_) {
    /* 使用預設的安全選項 */
  }
}

function saveBackupPreference() {
  try {
    localStorage.setItem(BACKUP_STORAGE_KEY, shouldBackupBeforeApply() ? "1" : "0");
  } catch (_) {
    /* 儲存失敗不影響本次套用 */
  }
}

function customOutputEnabled() {
  return !!$("choose-output-dir")?.checked;
}

function selectedOutputDir() {
  const input = $("output");
  if (!input) return "";
  const chosen = customOutputEnabled() ? (input.value || "").trim() : "";
  return chosen || (input.dataset.autoPath || "").trim() || (input.value || "").trim();
}

function syncOutputField() {
  const input = $("output");
  if (!input) return;
  const custom = customOutputEnabled();
  const autoPath = (input.dataset.autoPath || "").trim();
  if (!custom && autoPath) input.value = autoPath;
  input.readOnly = progressBusy || !custom;
  input.setAttribute("aria-readonly", input.readOnly ? "true" : "false");
  const hasInstance = !!($("instance")?.value || "").trim();
  const picker = $("btn-output-pick");
  if (picker) picker.hidden = !hasInstance || !custom || progressBusy;
}

function setAutoOutputDir(path) {
  const input = $("output");
  if (!input) return;
  const value = String(path || "").trim();
  input.dataset.autoPath = value;
  if (!customOutputEnabled()) input.value = value;
  const status = $("output-status");
  if (status) status.textContent = value
    ? "翻譯會在這個位置建立「翻譯結果」；完成後套用到遊戲資料夾。"
    : "請先選擇遊戲資料夾。";
  syncOutputField();
  scheduleBackupStateRefresh();
  refreshShareableState();
}

function resultWorkDir(outputDir) {
  const clean = String(outputDir || "").replace(/[\\/]+$/, "");
  return /(?:^|[\\/])翻譯結果$/i.test(clean) ? clean : clean + "\\翻譯結果";
}

async function refreshShareableState() {
  const token = ++shareableProbeToken;
  const outputDir = selectedOutputDir();
  if (!outputDir) {
    hasShareableFiles = false;
    if (token === shareableProbeToken) syncUiState();
    return;
  }
  try {
    const work = resultWorkDir(outputDir);
    hasShareableFiles = !!(await invoke("has_shareable_translation_cmd", { workRoot: work }));
  } catch (_) {
    hasShareableFiles = false;
  }
  if (token === shareableProbeToken) syncUiState();
}

function readOutputStorageMode() {
  try {
    const mode = String(localStorage.getItem(OUTPUT_STORAGE_MODE_KEY) || "managed").trim();
    return mode === "beside" || mode === "custom" ? mode : "managed";
  } catch (_) {
    return "managed";
  }
}

function readOutputCustomRoot() {
  try {
    return String(localStorage.getItem(OUTPUT_CUSTOM_ROOT_KEY) || "").trim();
  } catch (_) {
    return "";
  }
}

function readLastInstancePath() {
  try {
    return String(localStorage.getItem(LAST_INSTANCE_PATH_KEY) || "").trim();
  } catch (_) {
    return "";
  }
}

function writeLastInstancePath(path) {
  const value = String(path || "").trim();
  try {
    if (value) localStorage.setItem(LAST_INSTANCE_PATH_KEY, value);
    else localStorage.removeItem(LAST_INSTANCE_PATH_KEY);
  } catch (_) {
    /* ignore */
  }
}

function outputStorageHint(mode) {
  if (mode === "beside") return "翻譯結果會放在模組整合包旁邊的翻譯輸出資料夾。";
  if (mode === "custom") return "翻譯結果會放在你指定的資料夾，每個模組整合包分開存放。";
  return "工具會在這台電腦的使用者資料夾裡，替每個模組整合包建立獨立資料夾。";
}

/** 依設定解析此整合包的預設結果根（本包「另指定」優先）。 */
async function resolveOutputDirForInstance(instancePath) {
  const path = String(instancePath || "").trim();
  if (!path) return "";
  if (customOutputEnabled()) {
    const chosen = ($("output")?.value || "").trim();
    if (chosen) return chosen;
  }
  const mode = readOutputStorageMode();
  try {
    if (mode === "custom") {
      const root = readOutputCustomRoot();
      if (root) {
        const custom = await invoke("managed_output_for_instance_with_base", {
          instancePath: path,
          baseDir: root,
        }).catch(() => "");
        if (custom) return custom;
      }
    }
    if (mode === "beside") {
      const beside = await invoke("suggest_output_dir", { instancePath: path }).catch(() => "");
      if (beside) return beside;
    }
    return (
      (await invoke("managed_output_for_instance", { instancePath: path }).catch(() => "")) ||
      (await invoke("suggest_output_dir", { instancePath: path }).catch(() => "")) ||
      ""
    );
  } catch (_) {
    return "";
  }
}

let localCacheProbe = null;
let localCacheProbeToken = 0;

function hideLocalCacheCard() {
  localCacheProbe = null;
  const card = $("local-cache-card");
  if (card) card.hidden = true;
}

function logApplyWarnings(result) {
  const warnings = result?.warnings || [];
  for (const w of warnings) {
    if (w) appendLog(String(w), "warn");
  }
}

function showLocalCacheCard(probe) {
  localCacheProbe = probe && probe.status && probe.status !== "none" ? probe : null;
  const card = $("local-cache-card");
  if (!card) return;
  // 正在翻譯時不顯示：進度條已經回答了「有沒有翻譯」這件事，卡片只會製造矛盾訊息。
  // 剛移除翻譯（狀態卡 S19）時也不顯示：「套用到遊戲」只在狀態卡一處。
  if (
    !localCacheProbe ||
    translationState === "running" ||
    packActions.isRemovalShownFor(($("instance")?.value || "").trim())
  ) {
    card.hidden = true;
    return;
  }
  card.hidden = false;
  const badge = $("local-cache-badge");
  if (badge) {
    badge.hidden = false;
    badge.textContent =
      localCacheProbe.status === "ready" ? "可直接分享" : "可接續";
    badge.dataset.state = localCacheProbe.status;
  }
  const msg = $("local-cache-message");
  if (msg) msg.textContent = localCacheProbe.message || "";
  const packEl = $("local-cache-pack");
  if (packEl) {
    const packLabel =
      localCacheProbe.packName ||
      localCacheProbe.pack_name ||
      localCacheProbe.canonicalZip ||
      localCacheProbe.canonical_zip ||
      "";
    packEl.hidden = !packLabel;
    packEl.textContent = packLabel ? `資源包：${packLabel}` : "";
  }
  const pathEl = $("local-cache-path");
  if (pathEl) {
    const shown = localCacheProbe.workRoot || localCacheProbe.outputDir || "";
    pathEl.hidden = !shown;
    pathEl.textContent = shown ? `位置：${shown}` : "";
  }
  const warnEl = $("local-cache-warn");
  if (warnEl) {
    const probeOut = String(localCacheProbe.outputDir || "").trim();
    const selected = String(selectedOutputDir() || "").trim();
    const mismatch =
      customOutputEnabled() &&
      probeOut &&
      selected &&
      probeOut.replace(/\\/g, "/").toLowerCase() !== selected.replace(/\\/g, "/").toLowerCase();
    warnEl.hidden = !mismatch;
    warnEl.textContent = mismatch
      ? "本包「另指定結果資料夾」與探測到的快取位置不同；再次套用／分享以快取位置為準。"
      : "";
  }
  const shareable = !!localCacheProbe.shareable;
  const applyable = !!localCacheProbe.applyable || shareable;
  const pending = Number(localCacheProbe.pendingCount || 0) > 0;
  if ($("btn-cache-apply")) $("btn-cache-apply").hidden = !applyable;
  if ($("btn-cache-supplement")) $("btn-cache-supplement").hidden = !pending && localCacheProbe.status !== "partial";
  if ($("btn-cache-share")) $("btn-cache-share").hidden = !shareable;
}

async function probeLocalPackCache(instancePath, { silent } = {}) {
  const path = String(instancePath || "").trim();
  const token = ++localCacheProbeToken;
  if (!path) {
    hideLocalCacheCard();
    return null;
  }
  try {
    const probe = await invoke("probe_local_pack_cache_cmd", {
      instancePath: path,
      outputDir: selectedOutputDir() || null,
      customBaseDir: readOutputStorageMode() === "custom" ? readOutputCustomRoot() || null : null,
    });
    if (token !== localCacheProbeToken) return null;
    if (probe && probe.status && probe.status !== "none") {
      if (probe.outputDir && !customOutputEnabled()) {
        setAutoOutputDir(probe.outputDir);
      }
      showLocalCacheCard(probe);
      if (probe.shareable) {
        hasShareableFiles = true;
        if (translationState === "idle" || translationState === "ready") {
          resultSource = "cache";
          setTranslationState("complete");
          showCachedResultOnRail(probe);
        } else {
          syncUiState();
        }
      } else {
        await refreshShareableState();
      }
      if (!silent) {
        appendLog(probe.message || "已找到本機翻譯結果。", "info");
      }
    } else {
      hideLocalCacheCard();
      await refreshShareableState();
    }
    return probe;
  } catch (e) {
    if (token !== localCacheProbeToken) return null;
    hideLocalCacheCard();
    if (!silent) appendLog("本機翻譯探測略過：" + formatInvokeError(e), "warn");
    return null;
  }
}

async function applyCachedTranslation() {
  const instancePath = ($("instance")?.value || "").trim();
  const outputDir =
    (localCacheProbe && localCacheProbe.outputDir) || selectedOutputDir();
  if (!instancePath || !outputDir) {
    return appendLog("請先選好遊戲資料夾與結果位置。", "warn");
  }
  if (!(await ensureGameClosed(instancePath, "再次套用"))) return;
  const packName =
    (localCacheProbe &&
      (localCacheProbe.packName || localCacheProbe.pack_name || localCacheProbe.canonicalZip || localCacheProbe.canonical_zip)) ||
    packNameForTranslate() ||
    null;
  setBusy(true, "apply");
  let pendingApply = null;
  try {
    appendLog("正在把本機翻譯結果再次套用到遊戲…");
    const result = await invoke("apply_translation_to_game", {
      instancePath,
      outputDir,
      packName,
    });
    const summary = result?.playerSummary || result?.player_summary || result?.message || "套用完成。";
    if (isApplyPending(result)) {
      pendingApply = result;
    } else {
      appendLog(summary);
      logApplyWarnings(result);
      setTranslationState("complete");
    }
    await refreshShareableState();
  } catch (e) {
    appendError("再次套用失敗：" + formatInvokeError(e));
    offerRecordResetIfBroken(e, instancePath);
  } finally {
    setBusy(false);
  }
  if (pendingApply) await applyPending.handle(pendingApply, { instancePath, outputDir, packName });
}

function wireLocalCacheCard() {
  if ($("btn-cache-open")) {
    $("btn-cache-open").onclick = async () => {
      const work =
        (localCacheProbe && (localCacheProbe.workRoot || localCacheProbe.outputDir)) ||
        resultWorkDir(selectedOutputDir());
      if (!work) return appendLog("還沒有結果位置可打開。", "warn");
      try {
        await invoke("open_path", { path: work });
      } catch (e) {
        appendLog("無法打開：" + formatInvokeError(e), "warn");
      }
    };
  }
  if ($("btn-cache-apply")) $("btn-cache-apply").onclick = () => applyCachedTranslation();
  if ($("btn-cache-supplement")) {
    $("btn-cache-supplement").onclick = () => {
      if (typeof onSupplement === "function") onSupplement();
    };
  }
  if ($("btn-cache-share")) {
    $("btn-cache-share").onclick = () => {
      if (localCacheProbe?.outputDir && !customOutputEnabled()) {
        setAutoOutputDir(localCacheProbe.outputDir);
      }
      // 直接呼叫同一個函式，不用 .click() 代打另一顆按鈕：
      // 那會讓事件同時經過「直接接線」與「文件委派保底」兩條路徑，變成一次點擊兩次動作。
      const btn = $("btn-package");
      if (btn && !btn.disabled) packageShare();
      else appendLog("目前尚無可分享檔案；請確認結果資料夾內容。", "warn");
    };
  }
}

/**
 * 啟動時**不再**自動填入上次的遊戲資料夾。
 *
 * 使用者要求：開工具不該自動帶出上一次的翻譯紀錄，只有他自己選了資料夾之後，
 * 才去偵測「本機是否已有翻譯」。舊行為會在啟動時就回填路徑、驗證、探快取，
 * 讓人以為工具已經在處理某個整合包了——那不是他這次要做的事。
 *
 * 上次的路徑仍然記著，但只用來當「選資料夾」對話框的起始位置（方便，不誤導）。
 */
async function restoreLastInstanceOnStartup() {
  hideLocalCacheCard();
  setTranslationState("idle");
  resetStepPanelForNewInstance();
  showSettingsHealthNotice();
  await offerResumeUnfinishedRun();
}

/** 上次那包的路徑，只在「有沒有沒做完的翻譯」這個問題上用得到。 */
let resumeCandidatePath = "";

function setCardText(id, value) {
  const el = $(id);
  if (el) el.textContent = String(value ?? "");
}

function hideResumeCard() {
  const card = $("resume-card");
  if (card) card.hidden = true;
}

/**
 * 啟動時看看上次那包翻到一半沒有，有的話主動問要不要接著做。
 *
 * 為什麼要問而不是直接接續：使用者這次打開工具不見得是為了同一包。
 * 所以路徑仍然**不自動填**，按了「接續補完」才走跟自己選資料夾一樣的流程。
 *
 * 全程靜默失敗——探測不到就當作沒事，絕不能讓啟動流程卡住。
 */
async function offerResumeUnfinishedRun() {
  const card = $("resume-card");
  if (!card) return;
  const path = readLastInstancePath();
  if (!path) return;
  let probe = null;
  try {
    probe = await invoke("probe_local_pack_cache_cmd", {
      instancePath: path,
      outputDir: null,
      customBaseDir: readOutputStorageMode() === "custom" ? readOutputCustomRoot() || null : null,
    });
  } catch (_) {
    return; // 資料夾被搬走／刪掉是常態，不是錯誤
  }
  const pending = Number(probe?.pendingCount ?? probe?.pending_count ?? 0);
  // 只在「真的還有東西沒翻」時打擾；已完成的那包由既有的「本機已有翻譯」卡片負責
  if (!probe || probe.status !== "partial" || pending <= 0) return;

  resumeCandidatePath = path;
  const percent = Number(probe.completionPercent ?? probe.completion_percent ?? 0);
  const packName = String(probe.packName || probe.pack_name || "").trim();
  const who = packName ? `「${packName}」` : "上一個整合包";
  const howFar = percent > 0 ? `已完成約 ${percent}%，` : "";
  setCardText(
    "resume-message",
    `${who}${howFar}還有約 ${pending} 句沒翻完。要接著把它做完嗎？`
  );
  setCardText("resume-path", path);
  card.hidden = false;
}

function wireResumeCard() {
  const go = $("btn-resume-continue");
  if (go) {
    go.onclick = async () => {
      const path = resumeCandidatePath;
      hideResumeCard();
      if (!path) return;
      try {
        await adoptInstancePath(path, { silentProbe: false });
      } catch (e) {
        appendLog("接續上次的翻譯失敗：" + formatInvokeError(e), "warn");
      }
    };
  }
  const dismiss = $("btn-resume-dismiss");
  // 只關掉這次的提示；紀錄留著，下次開工具還是會問。
  if (dismiss) dismiss.onclick = () => hideResumeCard();
}

/**
 * 選完資料夾就檢查寫入權限，不要等翻完三小時才在套用階段失敗。
 *
 * 站長要求「必要時可以向使用者索取管理員權限」——關鍵是「必要時」：
 * 絕不在啟動時要求，只在真的寫不進去時給一個一鍵解法。
 */
async function checkWriteAccessFor(instancePath) {
  const card = $("write-access-card");
  if (!card) return;
  const path = String(instancePath || "").trim();
  if (!path) {
    card.hidden = true;
    return;
  }
  let report = null;
  try {
    report = await invoke("check_write_access_cmd", { instancePath: path });
  } catch (_) {
    card.hidden = true;
    return;
  }
  if (!report || report.writable) {
    card.hidden = true;
    return;
  }
  setCardText("write-access-message", report.message || "這個資料夾目前寫不進去。");
  setCardText("write-access-path", report.path || path);
  // 只有真的是權限問題才給提權按鈕；磁碟滿了提權也沒用
  const admin = $("btn-relaunch-admin");
  if (admin) admin.hidden = !report.needsAdmin;
  card.hidden = false;
}

function wireWriteAccessCard() {
  const admin = $("btn-relaunch-admin");
  if (admin) {
    admin.onclick = async () => {
      const instancePath = ($("instance")?.value || "").trim();
      try {
        const out = await invoke("relaunch_as_admin_cmd", { instancePath });
        if (!out?.relaunching) {
          // UAC 被取消不是錯誤——他只是不想提權，讓他改選資料夾就好
          appendLog("已取消以管理員身分開啟。你也可以改選一個放在自己資料夾底下的整合包。");
        }
      } catch (e) {
        appendLog("無法以管理員身分重新開啟：" + formatInvokeError(e), "warn");
      }
    };
  }
  const dismiss = $("btn-write-access-dismiss");
  if (dismiss) {
    dismiss.onclick = () => {
      const card = $("write-access-card");
      if (card) card.hidden = true;
      void onPickInstance();
    };
  }
}

/** 設定檔壞掉時把原因講清楚，不要讓偏好無聲無息回到預設值。 */
function showSettingsHealthNotice() {
  let notice = null;
  try {
    notice = getSettingsHealthNotice();
  } catch (_) {
    notice = null;
  }
  if (!notice) return;
  // 完整說明進紀錄；畫面只留橫幅 N-02 一句（規格 §3.4）
  appendLog(`${notice.title}：${notice.body}`, "warn");
  const banner = settingsHealthBanner(notice);
  if (banner) packActions.bannerArea().show(banner);
}

/**
 * 換整合包（或清空選擇）時，把右側的步驟與統計整個歸零。
 *
 * 使用者反映：換到一個沒有翻譯結果的資料夾時，「本機已有翻譯」卡片確實消失了，
 * 但步驟欄位還停在上一包的燈號與花費時間，看起來像這一包已經翻過。
 */
function resetStepPanelForNewInstance() {
  lastStepIdx = -1;
  lastActiveStepKey = null;
  lastActiveStepTotal = null;
  lastProgressLogKey = "";
  resetStepTimings();
  resetCoverageMetrics("尚未開始");
  setProgress(0, "尚未開始");
  const root = $("linear-steps");
  if (root) {
    root.querySelectorAll(".lin-step").forEach((el) => {
      el.classList.remove("active", "done", "error");
      const meta = el.querySelector(".lin-step-meta");
      if (meta) {
        meta.textContent = "";
        meta.hidden = true;
      }
    });
  }
  const total = $("step-total-time");
  if (total) {
    total.textContent = "";
    total.hidden = true;
  }
}

function applyTheme(theme) {
  const normalized = theme === "light" ? "light" : "dark";
  document.documentElement.dataset.theme = normalized;
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.setAttribute("content", normalized === "dark" ? "#14161a" : "#eceef2");
  try {
    localStorage.setItem(THEME_STORAGE_KEY, normalized);
  } catch (_) {
    /* 儲存空間不可用時仍維持本次工作階段的主題 */
  }
}

function initTheme() {
  let saved = "dark";
  try {
    const stored = localStorage.getItem(THEME_STORAGE_KEY);
    if (stored === "light" || stored === "dark") saved = stored;
  } catch (_) {
    /* 預設深色 */
  }
  applyTheme(saved);
}


/** UI 日誌：右欄顯示記憶體內全部行（上限 MAX_LOG_LINES）。 */
const progressLogLines = [];
const MAX_LOG_LINES = 2000;
const RUN_LOG_FILE = "執行日誌.txt";
const FONT_LOG_FILE = "字體執行日誌.txt";
const DEV_TRACE_FILE = "開發進度偵測.txt";
const RUN_LOG_MAX_BYTES = 256 * 1024;
let errorLogCount = 0;
const fontLogLines = [];
let fontPreviewFace = null;

function nowStamp() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, "0");
  return p(d.getHours()) + ":" + p(d.getMinutes()) + ":" + p(d.getSeconds());
}

function trimUrlTail(url) {
  let clean = String(url || "");
  let tail = "";
  while (/[),.，。；;!?！？]$/.test(clean)) {
    tail = clean.slice(-1) + tail;
    clean = clean.slice(0, -1);
  }
  return { clean, tail };
}

function appendLinkifiedText(container, text) {
  const raw = String(text || "");
  const pattern = /https?:\/\/[^\s<>"']+/gi;
  let last = 0;
  for (const match of raw.matchAll(pattern)) {
    const idx = match.index || 0;
    if (idx > last) container.appendChild(document.createTextNode(raw.slice(last, idx)));
    const { clean, tail } = trimUrlTail(match[0]);
    if (clean) {
      const link = document.createElement("a");
      link.href = clean;
      link.className = "log-link inline-ext-link";
      link.textContent = clean;
      link.addEventListener("click", (ev) => {
        ev.preventDefault();
        void openExternalUrl(clean);
      });
      container.appendChild(link);
    }
    if (tail) container.appendChild(document.createTextNode(tail));
    last = idx + match[0].length;
  }
  if (last < raw.length) container.appendChild(document.createTextNode(raw.slice(last)));
}

function renderLogLines(el, lines) {
  if (!el) return;
  // 右欄只呈現最新幾則；完整紀錄仍寫進結果資料夾，由「報告」查看。
  const visible = visibleLogLines(lines);
  const hidden = hiddenLogCount(lines);
  el.replaceChildren();
  if (hidden > 0) {
    const note = document.createElement("span");
    note.className = "log-truncated-note";
    note.textContent = `（前 ${hidden} 則已收合，完整紀錄請按「報告」）`;
    el.appendChild(note);
    el.appendChild(document.createTextNode("\n"));
  }
  visible.forEach((line, index) => {
    if (index > 0) el.appendChild(document.createTextNode("\n"));
    appendLinkifiedText(el, line);
  });
  if (!visible.length && hidden === 0) el.textContent = "";
  el.scrollTop = el.scrollHeight;
}

function renderLogNow() {
  const el = $("log");
  if (!el) return;
  el.classList.remove("log-empty");
  renderLogLines(el, progressLogLines);
}

function renderPanelLog(el, lines) {
  if (!el) return;
  el.classList.toggle("log-empty", lines.length === 0);
  renderLogLines(el, lines);
}

function appendPanelLog(lines, msg, level, renderFn, el) {
  const text = String(msg == null ? "" : msg).replace(/\r\n/g, "\n");
  if (!text) return;
  const lv = level || "info";
  for (const line of text.split("\n")) {
    let prefix = "[" + nowStamp() + "] ";
    if (lv === "error") prefix += "【錯誤】";
    else if (lv === "warn") prefix += "【警告】";
    lines.push(prefix + line);
  }
  while (lines.length > MAX_LOG_LINES) lines.shift();
  renderFn(el, lines);
}

function clearFontLog(seedMsg) {
  fontLogLines.length = 0;
  if (seedMsg) fontLogLines.push("[" + nowStamp() + "] " + seedMsg);
  renderPanelLog($("font-log"), fontLogLines);
}

function appendFontLog(msg, level) {
  appendPanelLog(fontLogLines, msg, level, renderPanelLog, $("font-log"));
}

async function flushFontLog(workDir) {
  if (!workDir) return;
  const header = "【字體執行日誌】\n與翻譯「執行日誌.txt」分開。每次字體任務結束時覆寫。\n\n";
  let body = header + fontLogLines.join("\n") + "\n";
  if (body.length > RUN_LOG_MAX_BYTES) {
    body = body.slice(0, 800) + "\n…（已截斷）…\n" + body.slice(-(RUN_LOG_MAX_BYTES - 1200));
  }
  const path = String(workDir).replace(/[\\/]+$/, "") + "\\" + FONT_LOG_FILE;
  await invoke("write_text_file", { path, content: body });
}

function fontWorkDir(outputDir) {
  const clean = String(outputDir || "").replace(/[\\/]+$/, "");
  return /(?:^|[\\/])字體結果$/i.test(clean) ? clean : clean + "\\字體結果";
}

function flushUiFrame() {
  if (pendingProgressPayload) {
    const payload = pendingProgressPayload;
    pendingProgressPayload = null;
    const percent = payload.percent != null ? payload.percent : 0;
    const message = payload.message || "處理中…";
    setProgress(percent, message, { payload });
  }
  if (pendingLogRender) {
    pendingLogRender = false;
    renderLogNow();
  }
  lastUiFlushAt = Date.now();
}

function scheduleUiFlush() {
  if (uiFlushRaf || uiFlushTimer) return;
  const now = Date.now();
  const delay = Math.max(0, UI_FLUSH_MS - (now - lastUiFlushAt));
  const kick = () => {
    uiFlushRaf = window.requestAnimationFrame(() => {
      uiFlushRaf = 0;
      flushUiFrame();
    });
  };
  if (delay > 0) {
    uiFlushTimer = window.setTimeout(() => {
      uiFlushTimer = 0;
      kick();
    }, delay);
  } else {
    kick();
  }
}

function queueProgressPayload(payload) {
  pendingProgressPayload = payload || null;
  scheduleUiFlush();
}

async function paintBeforeInvoke() {
  await new Promise((resolve) => {
    window.requestAnimationFrame(() => window.setTimeout(resolve, 0));
  });
}

function clearLog(seedMsg) {
  progressLogLines.length = 0;
  errorLogCount = 0;
  lastProgressLogKey = "";
  displayPercent = 0;
  lastRealPercent = 0;
  lastRealMessage = "";
  lastProgressPayload = null;
  lastActiveStepKey = null;
  lastActiveStepTotal = null;
  pendingProgressPayload = null;
  setProgressStateBadge(null);
  if (seedMsg) progressLogLines.push("[" + nowStamp() + "] " + seedMsg);
  renderLogNow();
}

function appendLog(msg, level) {
  const text = String(msg == null ? "" : msg).replace(/\r\n/g, "\n");
  if (!text) return;
  if (/用詞不一致提示/.test(text)) {
    showAppToast("已產生用詞不一致提示；可按「併入用詞建議」寫進術語表（選用）。", 4500);
    void refreshConsistencyMergeUi();
  }
  const lv = level || "info";
  if (lv === "error") maybePlaySfxError();
  const lines = text.split("\n");
  for (const line of lines) {
    let prefix = "[" + nowStamp() + "] ";
    if (lv === "error") {
      prefix += "【錯誤】";
      errorLogCount += 1;
    } else if (lv === "warn") {
      prefix += "【警告】";
    }
    // 後端若已帶【錯誤】前綴則不重複
    const body =
      (lv === "error" || lv === "warn") && (line.startsWith("【錯誤】") || line.startsWith("【警告】"))
        ? line.replace(/^【錯誤】/, "").replace(/^【警告】/, "")
        : line;
    progressLogLines.push(prefix + body);
  }
  while (progressLogLines.length > MAX_LOG_LINES) {
    progressLogLines.shift();
  }
  pendingLogRender = true;
  scheduleUiFlush();
}

/** 最近一則錯誤（內容＋時間），用來擋掉同一秒內的重覆播報。 */
let lastErrorText = "";
let lastErrorAt = 0;
const ERROR_DEDUPE_MS = 3000;

/**
 * 錯誤只播一次。
 *
 * 實測（Craft_to_Exile_2 的執行日誌）同一則「尚未安裝本地模型」在同一秒被寫了 5 次：
 * 後端 `emit_error` 兩次、`handleRunFailure` 的 whatFailed ＋ 錯誤本文、
 * 再加上外層 catch 又補一次。使用者只會覺得畫面在鬼打牆。
 * 這裡只擋「3 秒內內容完全相同」的重覆；不同內容一律照常寫入。
 */
function appendError(msg) {
  const text = String(msg == null ? "" : msg).trim();
  if (!text) return;
  const now = Date.now();
  if (text === lastErrorText && now - lastErrorAt < ERROR_DEDUPE_MS) return;
  lastErrorText = text;
  lastErrorAt = now;
  appendLog(text, "error");
}

/** 這次執行的時間戳，用來當執行紀錄的檔名。開始翻譯時重設。 */
let currentRunStamp = "";

function newRunStamp() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}_${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
}

/**
 * 寫出執行紀錄。
 *
 * 使用者實測遇到：翻到一半工具被關掉，重開續翻只跑了幾分鐘就寫檔，
 * **把前面一小時的完整紀錄整個蓋掉**（那份檔案只剩 1981 bytes），
 * 出問題時完全沒有線索。所以除了維持舊的 `執行日誌.txt`（相容「報告」按鈕
 * 與說明文件），另外把每次執行**各存一份**到「翻譯執行紀錄」資料夾。
 */
async function flushRunLog(workDir) {
  if (!workDir) return;
  const header =
    "【執行日誌】\n最近一次的過程狀態。完整歷史在同資料夾的「翻譯執行紀錄」裡，不會被覆寫。\n\n";
  let body = header + progressLogLines.join("\n") + "\n";
  if (body.length > RUN_LOG_MAX_BYTES) {
    body = body.slice(0, 800) + "\n…（已截斷）…\n" + body.slice(-(RUN_LOG_MAX_BYTES - 1200));
  }
  const root = String(workDir).replace(/[\\/]+$/, "");
  const path = root + "\\" + RUN_LOG_FILE;
  await invoke("write_text_file", { path, content: body });
  // 不覆寫的那一份；寫不進去不影響翻譯本身，靜默略過
  try {
    if (!currentRunStamp) currentRunStamp = newRunStamp();
    await invoke("write_run_journal_cmd", {
      workRoot: root,
      stamp: currentRunStamp,
      content: body,
    });
  } catch (_) {
    /* 紀錄寫不進去不該讓翻譯失敗 */
  }
}

/** 相容舊呼叫：整段覆寫改為附加；完成摘要用 setLogFinal */
function log(msg) {
  appendLog(msg);
}

function setLogFinal(msg) {
  appendLog("────────");
  if (errorLogCount > 0) {
    appendLog("本次共記錄 " + errorLogCount + " 筆錯誤／警告相關行。", "warn");
  }
  appendLog(msg);
  maybeHintAiQuota(msg);
}

/** AI 額度用完：提示支持（不提服務商名稱） */
function maybeHintAiQuota(text) {
  const s = String(text || "");
  if (!/額度|餘額|沒有回應|金鑰無效|無權限|沒有有效回應|請我喝珍奶|沒有餘力|429|quota exhausted/.test(s)) {
    return;
  }
  const mode = aiModeFromUi();
  appendLog("────────", "warn");
  if (mode === "gpt") {
    appendLog("GPT 來源使用你的 ChatGPT／OpenAI 帳號額度；請稍後再試、換模型，或改用自訂 API。", "warn");
  } else if (mode === "custom") {
    appendLog("自訂 API 使用你填入服務商的金鑰與額度；請到該服務商後台確認金鑰、餘額與速率限制。", "warn");
  } else if (mode === "local") {
    appendLog("本地模型在這台電腦執行；若沒有回應，請先完成安裝與健康檢查。", "warn");
  } else {
    appendLog("請確認目前 AI 來源的額度與登入狀態。", "warn");
  }
}

/** 使用者按停止不是錯誤，畫面不該變成一片紅字 */
function isCancellation(e) {
  return /已依你的要求停止/.test(formatInvokeError(e));
}

/** 統一處理各流程的失敗／取消收尾 */
function currentBackupChoice() {
  const value = getSettingPath(BACKUP_CHOICE_PATH, "");
  return value === "always" || value === "never" ? value : "";
}

/** 套用紀錄損壞時給「重設套用紀錄」的出口（不在失敗訊息裡要玩家自己找檔案）。 */
function offerRecordResetIfBroken(e, instancePath) {
  if (isForkableError(e)) {
    void offerForkInstance(
      { confirmDialog, invoke, appendLog: (text, level) => appendLog(text, level) },
      instancePath || ($("instance")?.value || "").trim()
    );
    return;
  }
  if (!isBrokenRecordError(e)) return;
  void offerRecordReset(
    { confirmDialog, invoke, appendLog: (text, level) => appendLog(text, level) },
    instancePath || ($("instance")?.value || "").trim()
  ).then((done) => {
    if (done) void refreshBackupState();
  });
}

function handleRunFailure(e, whatFailed) {
  offerRecordResetIfBroken(e);
  if (isCancellation(e)) {
    // 這裡是「已經停下來了」的收尾，不是「正在停」。舊版把訊息寫「已停止」
    // 卻同時掛上 cancelling 徽章（顯示「取消中」），畫面上兩個互相矛盾的狀態
    // 並排，使用者不知道到底停了沒。收尾時清掉狀態徽章，只留一句「已停止」。
    const payload = { ...(lastProgressPayload || {}) };
    delete payload.state;
    setProgress(Math.max(lastRealPercent, Math.floor(displayPercent) || 0), "已停止", {
      payload,
    });
    // 停止掃尾文案由各流程 catch 自行補充；此處只更新進度
    return;
  }
  setProgress(Math.max(lastRealPercent, Math.floor(displayPercent) || 0), whatFailed, {
    failed: true,
    payload: lastProgressPayload,
  });
  // 進度列已經寫了 whatFailed（setProgress 上一行），這裡只補「為什麼」。
  // 舊版兩者都 appendError，加上後端本來就 emit 過同一則，於是同一秒印四五次。
  appendError(formatInvokeError(e));
}

function formatInvokeError(e) {
  const playerize = (value) => {
    const text = String(value || "")
      .replace(/\r\n/g, "\n")
      .split("\n")
      .filter((line) => {
        const s = line.trim();
        return !/^at\s+/i.test(s) && !/^stack\b/i.test(s) && !/^\s*Caused by:\s*at\s+/i.test(s);
      })
      .join("\n")
      .trim();
    if (!text) return "操作失敗，請稍後再試。";
    if (/update_invoke_timeout/i.test(text)) {
      return "更新檢查逾時。請稍後再試；若仍失敗，請用頁尾「回報」。";
    }
    if (/login_required|guild_required|client_upgrade_required/i.test(text)) {
      return "翻譯前請先登入 Discord 並加入官方伺服器。";
    }
    if (/"errorType"\s*:/.test(text) || /^\s*[{[]/.test(text)) {
      return "操作失敗。請稍後再試；若仍失敗，請用頁尾「回報」。";
    }
    if (/error sending request|reqwest|hyper::Error/i.test(text)) {
      return "無法連上服務。請檢查網路後再試；若仍失敗，請用頁尾「回報」。";
    }
    if (/\.gguf\b|runtime zip|https?:\/\//i.test(text)) {
      return "操作失敗。請稍後再試；若仍失敗，請用頁尾「回報」。";
    }
    return text;
  };
  if (e == null) return "未知錯誤";
  if (e.code === "update_invoke_timeout") {
    return "更新檢查逾時。請稍後再試；若仍失敗，請用頁尾「回報」。";
  }
  if (typeof e === "string") return playerize(e);
  if (e.message) return playerize(e.message);
  if (e.error || e.detail || e.reason) return playerize(e.error || e.detail || e.reason);
  if (e.errorType) return "操作失敗。請稍後再試；若仍失敗，請用頁尾「回報」。";
  return "操作失敗，請稍後再試。";
}

function isDiscordGateError(e) {
  const detail = String(formatInvokeError(e || "") || "");
  return /請先登入 Discord|加入 ZeitFrei 官方 Discord|尚未加入官方伺服器|guild_required|login_required|翻譯前請先登入 Discord/.test(
    detail
  );
}

function handleDiscordGateError(e) {
  const detail = formatInvokeError(e);
  appendLog(detail, "warn");
  $("managed-auth-panel")?.scrollIntoView({ behavior: "smooth", block: "nearest" });
}

function ensureUsageFeedbackClientId() {
  try {
    const existing = localStorage.getItem(USAGE_FEEDBACK_CLIENT_ID_KEY);
    if (existing && /^[A-Za-z0-9_-]{8,64}$/.test(existing)) return existing;
  } catch (_) {}

  let id = "";
  try {
    id = crypto?.randomUUID ? crypto.randomUUID() : String(Date.now()) + "_" + Math.random();
  } catch (_) {
    id = String(Date.now()) + "_" + Math.random();
  }
  id = String(id).replace(/[^A-Za-z0-9_-]/g, "");
  if (id.length < 8) id = (id + "ABCDEFGH").slice(0, 12);
  id = id.slice(0, 40);

  try {
    localStorage.setItem(USAGE_FEEDBACK_CLIENT_ID_KEY, id);
  } catch (_) {}
  return id;
}

function isBlockingOverlayOpen() {
  if (isOnboardingActive() || progressBusy || shareUploadInFlight) return true;
  if (isConfirmOpen()) return true;
  // GPT 裝置碼與本地模型安裝這兩個 overlay 也會佔住畫面；漏列會造成回饋視窗疊上去。
  for (const id of [
    "update-overlay",
    "feedback-overlay",
    "issue-overlay",
    "consent-overlay",
    "gpt-login-overlay",
    "local-llm-overlay",
  ]) {
    const el = $(id);
    if (el && !el.hidden) return true;
  }
  const onboardRoot = $("onboard-root");
  if (onboardRoot && !onboardRoot.hidden) return true;
  return false;
}

function resetFeedbackOverlayForm() {
  feedbackStep = 1;
  document.querySelectorAll('input[name="feedback-pain"]').forEach((el) => {
    el.checked = false;
  });
  document.querySelectorAll('input[name="feedback-wish"]').forEach((el) => {
    el.checked = false;
  });
  document.querySelectorAll('input[name="feedback-rating"]').forEach((el) => {
    el.checked = false;
  });
  const noteEl = $("feedback-note");
  if (noteEl) {
    noteEl.value = "";
    noteEl.hidden = true;
  }
  syncFeedbackStepUi();
}

function syncFeedbackStepUi() {
  const step1 = $("feedback-step-1");
  const step2 = $("feedback-step-2");
  const backBtn = $("btn-feedback-back");
  const nextBtn = $("btn-feedback-next");
  const submitBtn = $("btn-feedback-submit");
  if (step1) step1.hidden = feedbackStep !== 1;
  if (step2) step2.hidden = feedbackStep !== 2;
  if (backBtn) backBtn.hidden = feedbackStep <= 1;
  if (nextBtn) nextBtn.hidden = feedbackStep !== 1;
  if (submitBtn) submitBtn.hidden = feedbackStep !== 2;
  updateFeedbackNoteVisibility();
}

function selectedFeedbackPain() {
  const el = document.querySelector('input[name="feedback-pain"]:checked');
  return el ? String(el.value || "").trim() : "";
}

function selectedFeedbackWish() {
  const el = document.querySelector('input[name="feedback-wish"]:checked');
  return el ? String(el.value || "").trim() : "";
}

function selectedFeedbackRating() {
  const el = document.querySelector('input[name="feedback-rating"]:checked');
  if (!el) return null;
  const n = Number(el.value);
  return Number.isFinite(n) ? n : null;
}

function feedbackNeedsNote() {
  const pain = selectedFeedbackPain();
  const wish = selectedFeedbackWish();
  const rating = selectedFeedbackRating();
  return pain === "other" || wish === "other" || rating === 1;
}

function updateFeedbackNoteVisibility() {
  const noteEl = $("feedback-note");
  if (!noteEl) return;
  const show = feedbackStep === 2 && feedbackNeedsNote();
  noteEl.hidden = !show;
  noteEl.required = false;
}

function hideFeedbackOverlay() {
  const ov = $("feedback-overlay");
  if (!ov) return;
  ov.hidden = true;
  ov.setAttribute("aria-hidden", "true");
}

function issueDiscordReady(s = latestAiStatus) {
  const loggedIn = !!(s && (s.loggedIn || s.logged_in));
  const inGuild = !!(s && (s.inGuild || s.in_guild));
  return loggedIn && inGuild;
}

/** 問題回報浮層的欄位 → 實際送出的內容（預覽＝送出，規格 §9 待確認 16）。 */
function issuePayloadFromForm() {
  const version = String(document.querySelector("[data-app-version]")?.textContent || "").trim();
  const mcVersion = ($("target-version")?.value || "").trim();
  const instancePath = ($("instance")?.value || "").trim();
  return buildIssuePayload({
    summary: $("issue-summary")?.value || "",
    displayKind: $("issue-display-kind")?.value || "",
    cause: $("issue-cause")?.value || "",
    detail: $("issue-detail")?.value || "",
    attach: { mc: !!$("issue-attach-mc")?.checked, pack: !!$("issue-attach-pack")?.checked },
    info: {
      toolVersion: version && version !== "—" ? version : "",
      mcVersion,
      packName: instancePath ? pathLeaf(instancePath) : "",
    },
  });
}

function refreshIssueReportUi() {
  const ov = $("issue-overlay");
  if (!ov || ov.hidden) return;
  const ready = issueDiscordReady();
  const gate = $("issue-discord-gate");
  if (gate) gate.hidden = ready;
  const note = $("issue-discord-note");
  const s = latestAiStatus;
  const loggedIn = !!(s && (s.loggedIn || s.logged_in));
  if (note) {
    note.textContent = !loggedIn
      ? "請先登入 Discord 並加入官方伺服器，方便維護、收集建議與調整工具。"
      : "請加入官方伺服器後再送出。";
  }
  if ($("btn-issue-login")) $("btn-issue-login").hidden = loggedIn;
  if ($("btn-issue-join")) $("btn-issue-join").hidden = !loggedIn || ready;
  const displayField = $("issue-display-field");
  if (displayField) displayField.hidden = ($("issue-summary")?.value || "") !== DISPLAY_CATEGORY;
  const payload = issuePayloadFromForm();
  const preview = $("issue-preview");
  if (preview) preview.textContent = payload.preview;
  const detail = ($("issue-detail")?.value || "").trim();
  const status = $("issue-status");
  let hint = lastIssueStatus;
  if (!hint && detail && detail.length <= 10) hint = "詳細說明請超過十個字。";
  if (status && !issueSubmitBusy) {
    status.textContent = hint;
    status.classList.toggle("is-set", !!hint);
  }
  const submit = $("btn-issue-submit");
  if (submit) submit.disabled = issueSubmitBusy || issueSubmitted;
}

function hideIssueOverlay() {
  const ov = $("issue-overlay");
  if (!ov) return;
  ov.hidden = true;
  ov.setAttribute("aria-hidden", "true");
  issueSubmitBusy = false;
}

function setIssueStatus(text) {
  lastIssueStatus = String(text || "");
  const status = $("issue-status");
  if (!status) return;
  status.textContent = lastIssueStatus;
  status.classList.toggle("is-set", !!lastIssueStatus);
}

function showIssueOverlay() {
  const ov = $("issue-overlay");
  if (!ov) return;
  lastIssueStatus = "";
  issueSubmitted = false;
  if ($("btn-issue-open-discord")) $("btn-issue-open-discord").hidden = true;
  if ($("issue-display-kind")) $("issue-display-kind").value = "";
  // 同一張表單若因網路逾時重送，必須沿用同一把 key，讓 Worker 回第一次已建立
  // 的私人討論串結果而不是重開一條。每次重新開回報視窗才視為新的案件。
  issueReportIdempotencyKey = `mcpl-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 12)}`;
  if ($("issue-summary")) $("issue-summary").value = "";
  if ($("issue-cause")) $("issue-cause").value = "";
  if ($("issue-detail")) $("issue-detail").value = "";
  setIssueStatus("");
  ov.hidden = false;
  ov.setAttribute("aria-hidden", "false");
  refreshIssueReportUi();
  void refreshAiStatus().then(() => refreshIssueReportUi());
}

async function submitIssueReportFromOverlay() {
  if (issueSubmitBusy || issueSubmitted) return;
  if (!issueDiscordReady()) {
    setIssueStatus("請先登入並加入官方伺服器。");
    return;
  }
  const payload = issuePayloadFromForm();
  if (!payload.ok) {
    setIssueStatus(payload.error);
    return;
  }
  issueSubmitBusy = true;
  refreshIssueReportUi();
  setIssueStatus("送出中…");
  let result = null;
  try {
    result = await invoke("submit_issue_report_cmd", {
      summary: payload.summary,
      cause: payload.cause,
      detail: payload.detail,
      idempotencyKey: issueReportIdempotencyKey,
    });
  } catch (e) {
    const raw = String(e?.message || e || "").trim();
    result = { ok: false, message: raw && !raw.includes("invoke") ? raw : "目前無法自動送出。" };
  }
  let copied = false;
  if (!result || !result.ok) {
    // 沒送出：把填好的內容放進剪貼簿，玩家貼到 Discord 就好，不用重打
    try {
      await navigator.clipboard.writeText(payload.preview);
      copied = true;
    } catch (_) {
      copied = false;
    }
  }
  const view = describeIssueResult(result, { copied });
  issueSubmitBusy = false;
  issueSubmitted = view.ok;
  // 結果就地寫在浮層裡（案件編號或如實說明），不再另開對話框
  setIssueStatus(view.text);
  appendLog(view.text, view.ok ? "info" : "warn");
  if ($("btn-issue-open-discord")) $("btn-issue-open-discord").hidden = !view.showDiscord;
  refreshIssueReportUi();
}

async function openIssueDiscord() {
  const invite =
    (latestAiStatus && (latestAiStatus.inviteUrl || latestAiStatus.invite_url)) ||
    "https://discord.gg/zeitfrei";
  await openExternalUrl(invite).catch(() => {});
}

function showFeedbackOverlay() {
  const ov = $("feedback-overlay");
  if (!ov) return;
  resetFeedbackOverlayForm();
  ov.hidden = false;
  ov.setAttribute("aria-hidden", "false");
  const status = $("feedback-status");
  if (status) status.textContent = "";
}

function scheduleUsageFeedbackNudge() {
  clearTimeout(usageFeedbackDelayTimerId);
  const delayMs = 8000 + Math.floor(Math.random() * 7001);
  usageFeedbackDelayTimerId = window.setTimeout(() => {
    void usageFeedbackMaybeNudge();
  }, delayMs);
}

async function usageFeedbackMaybeNudge() {
  const ov = $("feedback-overlay");
  if (!ov) return;
  if (translationState !== "complete") return;
  if (isBlockingOverlayOpen()) {
    pendingUsageFeedbackNudge = true;
    return;
  }
  if (!ov.hidden) return;

  const now = Date.now();
  const lastSubmit = (() => {
    try {
      return Number(localStorage.getItem(USAGE_FEEDBACK_LAST_SUBMIT_AT_KEY) || 0);
    } catch (_) {
      return 0;
    }
  })();
  const lastNudge = (() => {
    try {
      return Number(localStorage.getItem(USAGE_FEEDBACK_LAST_NUDGE_AT_KEY) || 0);
    } catch (_) {
      return 0;
    }
  })();

  const submitCooldownMs = 21 * 24 * 60 * 60 * 1000;
  const nudgeCooldownMs = 14 * 24 * 60 * 60 * 1000;
  if (lastSubmit && now - lastSubmit < submitCooldownMs) return;
  if (lastNudge && now - lastNudge < nudgeCooldownMs) return;

  const prob = 0.2;
  if (Math.random() > prob) return;

  try {
    localStorage.setItem(USAGE_FEEDBACK_LAST_NUDGE_AT_KEY, String(now));
  } catch (_) {}
  showFeedbackOverlay();
}

function advanceFeedbackStep() {
  const status = $("feedback-status");
  if (feedbackStep === 1) {
    if (!selectedFeedbackPain()) {
      if (status) status.textContent = "請先選一項困擾（或「沒問題」）。";
      return;
    }
    feedbackStep = 2;
    if (status) status.textContent = "";
    syncFeedbackStepUi();
    return;
  }
}

function retreatFeedbackStep() {
  if (feedbackStep <= 1) return;
  feedbackStep = 1;
  const status = $("feedback-status");
  if (status) status.textContent = "";
  syncFeedbackStepUi();
}

async function submitUsageFeedbackFromOverlay() {
  const status = $("feedback-status");
  const noteEl = $("feedback-note");
  const clientId = ensureUsageFeedbackClientId();
  const painPoint = selectedFeedbackPain();
  const wish = selectedFeedbackWish();
  const rating = selectedFeedbackRating();

  if (!painPoint) {
    if (status) status.textContent = "請回到上一步選擇困擾項目。";
    return;
  }
  if (!wish) {
    if (status) status.textContent = "請選一項希望優先改善的方向。";
    return;
  }

  const note = noteEl && !noteEl.hidden ? String(noteEl.value || "").trim() : "";
  const notePayload = note ? note : null;

  if (status) status.textContent = "送出中…";

  try {
    const payload = {
      clientId,
      painPoint,
      wish,
      note: notePayload,
    };
    if (rating != null) payload.rating = rating;
    const r = await invoke("submit_usage_feedback_cmd", payload);
    if (r && r.ok === true) {
      if (status) status.textContent = "已送出回饋，感謝支持！";
      try {
        localStorage.setItem(USAGE_FEEDBACK_LAST_SUBMIT_AT_KEY, String(Date.now()));
      } catch (_) {}
      hideFeedbackOverlay();
      return;
    }

    if (status) status.textContent = "送出失敗，請稍後再試。";

    try {
      localStorage.setItem(USAGE_FEEDBACK_LAST_NUDGE_AT_KEY, String(Date.now()));
    } catch (_) {}
  } catch (e) {
    if (status) status.textContent = "送出失敗：" + formatInvokeError(e);
  }
}

/** 右側五步：檢查 → 搜尋 → 翻譯 → 補充 → 套用 */
const STEP_ORDER = ["prep", "scan", "translate", "supplement", "done"];
/**
 * 每個步驟的花費時間，跟 coverageCarryBase 用同一套哲學：補充漏翻／修復工作階段是
 * 接續同一次翻譯效果，「檢查」「套用」這些步驟會再跑一次，時間要看得到，不能因為
 * 上一輪的「翻譯」步驟已經記錄過，這一輪就被蓋掉。只有真正開一輪新翻譯才清空。
 */
let stepElapsedMs = {};
let stepStartedAt = {};

function resetStepTimings() {
  stepElapsedMs = {};
  stepStartedAt = {};
  renderStepTotalTime();
}

/** 步驟切換時，把離開的那個步驟的時間定格；同一個步驟重新變成當前步驟則續算，不歸零。 */
/**
 * 步驟計時的唯一負責人。
 *
 * 這裡自己維護 `lastActiveStepKey`，**不能**再依賴別人來更新它。
 *
 * 這是使用者連續回報三次「檢查花費 22 分／43 分」的根因：舊版把
 * `lastActiveStepKey` 的賦值放在 `buildStepMeta()`（渲染函式）裡，而那個函式
 * 第一行就是 `if (!payload) return ""`——只要有一次進度事件沒帶 payload，
 * 這個變數就停在舊值，下面「結束上一步計時」的條件永遠不成立，
 * `stepStartedAt["prep"]` 永遠不會被刪掉。結果「檢查」被當成一直在跑，
 * 顯示的花費 = 已累積 +（現在 − 開始時間）一路長到整輪翻譯的總時間。
 *
 * 教訓：計時是狀態機，不能綁在「畫面有沒有東西可以畫」上。
 */
function trackStepTiming(currentKey) {
  const now = Date.now();
  if (lastActiveStepKey && lastActiveStepKey !== currentKey && stepStartedAt[lastActiveStepKey] != null) {
    stepElapsedMs[lastActiveStepKey] =
      (stepElapsedMs[lastActiveStepKey] || 0) + (now - stepStartedAt[lastActiveStepKey]);
    delete stepStartedAt[lastActiveStepKey];
  }
  if (currentKey && stepStartedAt[currentKey] == null) {
    // 第一次進這個步驟，或之前跑過又回來繼續（例如補充漏翻再次經過「套用」）
    stepStartedAt[currentKey] = now;
  }
  if (currentKey) lastActiveStepKey = currentKey;
}

function stepMetaTimeText(stepKey) {
  const running = stepStartedAt[stepKey] != null;
  const base = stepElapsedMs[stepKey] || 0;
  const total = running ? base + (Date.now() - stepStartedAt[stepKey]) : base;
  if (total <= 0) return "";
  return `花費 ${formatElapsed(total)}`;
}

function renderStepTotalTime() {
  const el = $("step-total-time");
  if (!el) return;
  let total = 0;
  for (const key of STEP_ORDER) {
    total += stepElapsedMs[key] || 0;
    if (stepStartedAt[key] != null) total += Date.now() - stepStartedAt[key];
  }
  el.textContent = total > 0 ? `總花費 ${formatElapsed(total)}` : "";
  el.hidden = total <= 0;
}

/** 翻譯整個結束（成功或失敗）時呼叫：把還在跑的那個步驟計時定格，之後畫面不再累加。 */
function freezeAllStepTimings() {
  const now = Date.now();
  for (const key of STEP_ORDER) {
    if (stepStartedAt[key] != null) {
      stepElapsedMs[key] = (stepElapsedMs[key] || 0) + (now - stepStartedAt[key]);
      delete stepStartedAt[key];
    }
  }
  renderStepTotalTime();
}
/** 步驟只准前進，避免進度文案含「套用」時來回閃爍 */
let lastStepIdx = -1;

function stepFromProgress(percent, message) {
  const p = Number(percent) || 0;
  const m = String(message || "");
  if (p <= 0) return null;
  // 真正完成語境才進 done；翻譯過程中的「套用／寫出」不算
  const donePhrase =
    /全部完成|補翻完成|補譯完成|字體包完成|沒有可再補的缺漏|修復完成|已套用到遊戲/.test(m);
  if (donePhrase && (p >= 95 || /完成|缺漏/.test(m))) return "done";
  if (p >= 100 && /完成/.test(m)) return "done";
  if (/失敗|錯誤/.test(m) && p === 0) return "error";
  if (/字體/.test(m)) return p >= 90 ? "done" : "prep";
  if (/補充[：:]|步驟\s*4|只補仍缺|複查仍缺|補漏/.test(m) || (p >= 82 && p < 97 && /補充|補約|仍缺|待補/.test(m))) {
    return "supplement";
  }
  if (p < 6 || /準備中|啟動|讀取上次|工作階段/.test(m)) return "prep";
  if (p < 33 || /本地蒐集|讀模組|掃描|資源包|KubeJS/.test(m)) {
    if (
      p >= 33 ||
      /本地整理|合併|OpenCC|詞典|快捷選單|整理完成|純 AI|AI 階段|AI 翻譯|補約|缺漏|打包|套用/.test(m)
    ) {
      return "translate";
    }
    return "scan";
  }
  if (p >= 97 || /備份並|直接套用|正在備份|套用到遊戲/.test(m)) return "done";
  if (p < 100) return "translate";
  return "done";
}

function stepFromStage(stage) {
  switch (String(stage || "").trim()) {
    case "prep":
      return "prep";
    case "scan":
    case "local":
      return "scan";
    case "translate":
      return "translate";
    case "extras":
    case "package":
      return "supplement";
    case "apply":
      return "done";
    default:
      return null;
  }
}

function stepFromStepNumber(step) {
  const idx = Math.max(1, Number(step) || 0) - 1;
  return STEP_ORDER[idx] || null;
}

function resolveStepKey(payload, percent, message) {
  const directStep = stepFromStepNumber(payload && payload.step);
  if (directStep) return directStep;
  const stageStep = stepFromStage(payload && payload.stage);
  if (stageStep) return stageStep;
  return stepFromProgress(percent, message);
}

function stateBadgeLabel(state) {
  switch (String(state || "").trim()) {
    case "waiting":
      return "等待回應";
    case "retrying":
      return "重試中";
    case "throttled":
      return "已降速（限流）";
    case "degraded":
      return "已降級";
    case "cancelling":
      return "取消中";
    case "completed_with_pending":
      return "本輪完成，仍有待補";
    case "partial":
      return "部分完成";
    default:
      return "";
  }
}

function setProgressStateBadge(state) {
  const badge = $("prog-state");
  if (!badge) return;
  const label = stateBadgeLabel(state);
  if (!label) {
    badge.hidden = true;
    badge.textContent = "";
    badge.removeAttribute("data-state");
    return;
  }
  badge.hidden = false;
  badge.textContent = label;
  badge.setAttribute("data-state", String(state));
}

/**
 * 只負責「這一步要顯示什麼字」。
 *
 * 刻意**不**碰 `lastActiveStepKey`——那是計時狀態，由 `trackStepTiming()`
 * 獨佔維護。舊版在這裡順手改它，導致缺 payload 時計時整個壞掉（見
 * `trackStepTiming` 的註解）。
 */
function buildStepMeta(payload, stepKey) {
  if (!payload) return "";
  const done = payload.done != null ? Number(payload.done) : null;
  const total = payload.total != null ? Number(payload.total) : null;
  const unit = (payload.unit || "").trim();
  if (stepKey) lastActiveStepTotal = total;
  // 子階段：讓使用者看得出「補充」這種長步驟裡面現在跑到哪一段
  const sub = String(payload.substage || payload.sub_stage || "").trim();
  const subIdx = Number(payload.substageIndex ?? payload.substage_index);
  const subTotal = Number(payload.substageTotal ?? payload.substage_total);
  const base = formatStepMeta(done, total, unit);
  if (!sub) return base;
  const position =
    Number.isFinite(subIdx) && Number.isFinite(subTotal) && subTotal > 0
      ? `（第 ${subIdx} / ${subTotal} 段）`
      : "";
  return base ? `${sub}${position} · ${base}` : `${sub}${position}`;
}

/** 已經走過的步驟：顯示定格的花費時間，而不是清空隱藏——這是本輪新加的「每步驟計時」。 */
function metaTextForPassedStep(stepKey) {
  return stepMetaTimeText(stepKey);
}

function updateLinearSteps(percent, message, failed, payload) {
  const root = $("linear-steps");
  if (!root) return;
  const items = root.querySelectorAll(".lin-step");
  const currentKey = resolveStepKey(payload, percent, message);
  let idx = currentKey ? STEP_ORDER.indexOf(currentKey) : -1;
  const p = Number(percent) || 0;
  if (!failed && p <= 0) {
    lastStepIdx = -1;
    lastActiveStepKey = null;
    lastActiveStepTotal = null;
  }
  // 單調前進的 clamp 移到「計時」之前套用，不能只影響畫面顯示。
  //
  // 舊版把這段 clamp 放在 trackStepTiming() 之後：畫面靠 clamp 過的 idx 正確
  // 停在「翻譯」，但計時邏輯拿的是 resolveStepKey() 沒被 clamp 過的原始
  // currentKey——翻譯中途（尤其多輪 AI 批次、補充漏翻接續同一輪）文案偶爾會
  // 被誤判回「準備中」之類的「檢查」關鍵字，於是計時邏輯真的把已經定格的
  // 「檢查」計時器重新啟動、一路累加到下一次真正的步驟切換為止——使用者
  // 回報「檢查」花費 22 分鐘就是這樣來的：時間被平白灌進一個早就完成的步驟。
  // 修法：clamp 完 idx 才決定要用哪個 key 記時間，讓「畫面顯示在哪一步」跟
  // 「時間算在哪一步」永遠是同一個答案（clampStepIndexForward 抽成純函式，
  // 有單元測試釘住這個回歸情境）。三個真正開新一輪的入口（開始翻譯／修復／
  // 補充漏翻）都會先把 lastStepIdx 重置成 -1，那才是「這個步驟合理地再跑
  // 一次」，不受這個 clamp 影響。
  idx = clampStepIndexForward(idx, lastStepIdx, p, currentKey, failed);
  const trackedKey = idx >= 0 ? STEP_ORDER[idx] : currentKey;
  if (idx >= 0) trackStepTiming(trackedKey);
  if (failed) {
    items.forEach((el) => {
      el.classList.remove("active", "done", "error");
      const stepKey = el.getAttribute("data-step");
      const si = STEP_ORDER.indexOf(stepKey);
      if (idx >= 0 && si < idx) el.classList.add("done");
      else if (si === idx) el.classList.add("error");
      const meta = el.querySelector("[data-step-meta]");
      if (meta) {
        const text = si === idx ? buildStepMeta(payload, trackedKey) : metaTextForPassedStep(stepKey);
        meta.textContent = text;
        meta.hidden = !text;
      }
    });
    renderStepTotalTime();
    return;
  }
  if (idx > lastStepIdx) lastStepIdx = idx;
  if (currentKey === "done") lastStepIdx = STEP_ORDER.length - 1;
  items.forEach((el) => {
    el.classList.remove("active", "done", "error");
    const stepKey = el.getAttribute("data-step");
    const si = STEP_ORDER.indexOf(stepKey);
    if (idx < 0) {
      const meta = el.querySelector("[data-step-meta]");
      if (meta) meta.hidden = true;
      return;
    }
    if (si < idx) el.classList.add("done");
    else if (si === idx) el.classList.add(currentKey === "done" ? "done" : "active");
    const meta = el.querySelector("[data-step-meta]");
    if (meta) {
      const text = si === idx ? buildStepMeta(payload, trackedKey) : metaTextForPassedStep(stepKey);
      meta.textContent = text;
      meta.hidden = !text;
    }
  });
  if (currentKey === "done") {
    // 「套用」本身也是一個步驟，走到 done 不代表它已經結束（可能還在跑），
    // 只是不再隱藏 meta——舊版在這裡把所有 meta 都藏起來，套用花了多久永遠看不到。
    // 真正定格是在 freezeAllStepTimings()（翻譯整個完成／失敗時），不在這裡。
    items.forEach((el) => {
      el.classList.remove("active");
      el.classList.add("done");
      const stepKey = el.getAttribute("data-step");
      const meta = el.querySelector("[data-step-meta]");
      if (meta) {
        const text = metaTextForPassedStep(stepKey);
        meta.textContent = text;
        meta.hidden = !text;
      }
    });
  }
  renderStepTotalTime();
}

let lastProgressLogKey = "";
/** 忙碌時心跳：避免百分比久不更新像當機 */
let progressBusy = false;
let progressStartedAt = 0;
let lastProgressAt = 0;
let lastRealPercent = 0;
let lastRealMessage = "";
let displayPercent = 0;
let heartbeatTimer = null;

function formatElapsed(ms) {
  const s = Math.floor(Math.max(0, ms) / 1000);
  if (s < 60) return s + " 秒";
  const m = Math.floor(s / 60);
  const r = s % 60;
  return m + " 分 " + r + " 秒";
}

function setProgBarWorking(on) {
  const fill = $("prog-fill");
  const bar = fill && fill.parentElement;
  if (fill) fill.classList.toggle("working", !!on);
  if (bar) bar.classList.toggle("waiting", !!on);
}

function stopProgressHeartbeat() {
  if (heartbeatTimer) {
    clearInterval(heartbeatTimer);
    heartbeatTimer = null;
  }
  setProgBarWorking(false);
}

function startProgressHeartbeat() {
  stopProgressHeartbeat();
  progressStartedAt = Date.now();
  lastProgressAt = Date.now();
  heartbeatTimer = setInterval(() => {
    if (!progressBusy) return;
    const now = Date.now();
    const elapsed = formatElapsed(now - progressStartedAt);
    const stuckMs = now - lastProgressAt;
    const baseMsg = lastRealMessage || "處理中…";
    const msgEl = $("prog-msg");
    if (msgEl) {
      if (stuckMs > 2000) {
        msgEl.textContent = baseMsg + " · " + elapsed;
        setProgBarWorking(true);
        setProgressStateBadge("waiting");
      } else {
        msgEl.textContent = baseMsg + " · 已進行 " + elapsed;
        setProgBarWorking(false);
        setProgressStateBadge(null);
      }
    }
  }, 1000);
}

function consumeProgressPayload(payload) {
  if (!payload || !payload.metrics) return;
  const m = payload.metrics || {};
  let changed = false;
  const setMax = (key, value) => {
    if (value == null || value === "") return;
    const num = Number(value);
    if (!Number.isFinite(num)) return;
    if (coverageMetrics[key] == null || num > Number(coverageMetrics[key] || 0)) {
      coverageMetrics[key] = num;
      changed = true;
    }
  };
  const setDirect = (key, value) => {
    if (value == null || value === "") return;
    const num = Number(value);
    if (!Number.isFinite(num)) return;
    if (coverageMetrics[key] !== num) {
      coverageMetrics[key] = num;
      changed = true;
    }
  };
  setMax("glossary", m.glossary);
  setMax("tm", m.tm);
  setMax("shared", m.shared);
  setMax("ai", m.ai);
  setMax("skipped", m.skipped);
  setMax("qualitySkipped", m.qualitySkipped ?? m.quality_skipped);
  setMax("prior", m.prior);
  setDirect("pending", m.pending);
  setDirect("packPending", m.packPending ?? m.pack_pending);
  setDirect("coveragePercent", m.coveragePercent ?? m.coverage_percent);
  setMax("batchDone", m.batchDone);
  setMax("batchTotal", m.batchTotal);
  setMax("batchRetry", m.batchRetry);
  setMax("batchRetryBatches", m.batchRetryBatches);
  setMax("batchFail", m.batchFail);
  setMax("cacheHitTokens", m.cacheHitTokens);
  setMax("cacheMissTokens", m.cacheMissTokens);
  setMax("completionTokens", m.completionTokens);
  setDirect("cacheHitPercent", m.cacheHitPercent);
  if (payload.detail && /限流|降級|Discord|登入/.test(String(payload.detail))) {
    coverageMetrics.summary = String(payload.detail).trim();
    changed = true;
  } else if (
    coverageMetrics.batchTotal != null &&
    coverageMetrics.batchDone != null &&
    !coverageSettlementLocked
  ) {
    const short = `進行中 · ${formatCount(coverageMetrics.batchDone)}／${formatCount(
      coverageMetrics.batchTotal
    )} 批`;
    if (coverageMetrics.summary !== short) {
      coverageMetrics.summary = short;
      changed = true;
    }
  }
  if (changed) renderCoverageMetrics();
}

function setFontProgress(percent, message, opts) {
  const p = Math.max(0, Math.min(100, Number(percent) || 0));
  const fill = $("font-prog-fill");
  const pctEl = $("font-prog-pct");
  const msgEl = $("font-prog-msg");
  const countEl = $("font-prog-count");
  if (fill) fill.style.width = (p >= 100 ? 100 : p) + "%";
  if (pctEl) pctEl.textContent = Math.floor(p >= 100 ? 100 : p) + "%";
  if (countEl) countEl.textContent = p > 0 ? Math.floor(p) + "% 進行中" : "尚未開始";
  if (msgEl && message) msgEl.textContent = message;
  const activeStep = p >= 100 ? "apply" : p >= 70 ? "apply" : p >= 35 ? "build" : p > 0 ? "settings" : "pick";
  document.querySelectorAll("#font-linear-steps .lin-step").forEach((el) => {
    const key = el.getAttribute("data-step");
    const order = ["pick", "settings", "build", "apply"];
    const idx = order.indexOf(key);
    const activeIdx = order.indexOf(activeStep);
    el.classList.toggle("active", idx === activeIdx && p < 100);
    el.classList.toggle("done", idx >= 0 && idx < activeIdx);
  });
  if (message && !(opts && opts.skipLog)) appendFontLog(Math.floor(p) + "%  " + message);
}

function setProgress(percent, message, opts) {
  const job = window.__busyJobKind || document.body.dataset.appPage || "translate";
  if (job === "font") {
    setFontProgress(percent, message, opts);
    return;
  }
  const p = Math.max(0, Math.min(100, Number(percent) || 0));
  const failed = opts && opts.failed;
  const payload = opts && opts.payload ? opts.payload : null;
  if (!failed && p >= 100) {
    maybePlaySfxSuccess(message);
  }
  if (payload) {
    lastProgressPayload = payload;
  } else if (p <= 3 && /準備|讀取上次|修復：尋找工作階段/.test(String(message || ""))) {
    lastProgressPayload = null;
  }
  if (!failed && p > 0 && p < 100) {
    lastProgressAt = Date.now();
    lastRealPercent = p;
    displayPercent = Math.max(displayPercent, p);
  }
  if (p >= 100 || p === 0) {
    displayPercent = p;
    lastRealPercent = p;
  }
  const showP = progressBusy && p > 0 && p < 100 ? Math.max(p, displayPercent) : p;
  $("prog-fill").style.width = (showP >= 100 ? 100 : showP) + "%";
  $("prog-pct").textContent = Math.floor(showP >= 100 ? 100 : showP) + "%";
  if (message) {
    lastRealMessage = message;
    consumeProgressPayload(payload);
    consumeCoverageMessage(message);
    const elapsed =
      progressBusy && progressStartedAt
        ? " · 已進行 " + formatElapsed(Date.now() - progressStartedAt)
        : "";
    $("prog-msg").textContent = shortenProgressMessage(message) + elapsed;
  }
  setProgressStateBadge(payload ? payload.state : failed ? null : null);
  updateLinearSteps(p, message, failed, payload || (failed ? lastProgressPayload : null));
  setProgBarWorking(false);
  // 日誌去重（AI 等待秒數變化不重寫）。AI 翻譯中的完整文案（批次/重試/token 明細）
  // 已經在上面 consumeProgressPayload／consumeCoverageMessage 完整寫進進階統計面板，
  // 日誌只留精簡版，避免同一件事在畫面上出現兩份（一份囉唆、一份精簡）。
  if (message && !(opts && opts.skipLog)) {
    const key = progressLogDedupeKey(p, message);
    if (key !== lastProgressLogKey) {
      lastProgressLogKey = key;
      appendLog(Math.floor(p) + "%  " + shortenProgressMessage(message));
    }
  }
}

function setBusy(busy, jobKind) {
  const wasBusy = progressBusy;
  progressBusy = !!busy;
  // 一項作業剛結束＝狀態一定變了：本機快取、可分享狀態、備份都要重讀。
  // 這是使用者最期待「畫面自己更新」的時機，不該讓他自己按重新整理。
  if (wasBusy && !busy) {
    try {
      refreshRegion("local-cache-card", "after-run");
    } catch (_) {
      /* 刷新失敗不影響作業結果 */
    }
  }
  const kind = busy ? (jobKind || "translate") : null;
  window.__busyJobKind = kind;
  // 讓後端知道現在能不能直接關閉——翻譯中關閉會弄丟進度與紀錄
  void invoke("set_translation_active_cmd", { active: !!busy }).catch(() => {});
  if (busy && kind) document.body.dataset.busyJob = kind;
  else delete document.body.dataset.busyJob;
  if (busy) {
    // 忙碌時不讓更新視窗持續干擾：延遲顯示給完成後處理。
    try {
      const overlay = document.getElementById("update-overlay");
      if (overlay && !overlay.hidden) {
        if (typeof window.zfUpdateModalSetPending === "function") window.zfUpdateModalSetPending(window.__mcpl_latestUpdateInfo || null, true);
        if (typeof window.zfUpdateModalHide === "function") window.zfUpdateModalHide();
      }
    } catch (_) {
      /* ignore */
    }
  }
  if (busy) {
    displayPercent = Math.max(displayPercent, lastRealPercent);
    startProgressHeartbeat();
  } else {
    stopProgressHeartbeat();
    try {
      const outputDir = selectedOutputDir() || ($("output")?.value || "").trim();
      if (outputDir && kind !== "font") void flushRunLog(resultWorkDir(outputDir));
      const fontOut = ($("font-output")?.value || "").trim();
      if (fontOut && kind === "font") void flushFontLog(fontWorkDir(fontOut));
    } catch (_) {
      /* ignore */
    }
    setProgressStateBadge(null);
  }
  const stop = $("btn-stop");
  const fontStop = $("btn-font-stop");
  if (stop) {
    stop.hidden = !busy || kind !== "translate";
    resetStopButton(stop);
  }
  if (fontStop) fontStop.hidden = !busy || kind !== "font";
  // 仍鎖定：開第二個重任務、改路徑、連線設定等
  // btn-run、btn-inst 不在這裡：它們改用 aria-disabled＋就地原因（狀態卡、D 區 S20）
  const hardLockIds = [
    "btn-repair",
    "btn-output-pick",
    "btn-save-adv",
    "btn-test-api",
    "use-ai",
    "backup-before-apply",
    "api-provider",
    "api-key",
    "base-url",
    "api-model",
    "choose-output-dir",
    "ai-source-custom",
    "ai-source-gpt",
    "ai-source-local",
    "btn-local-llm-setup",
    "btn-local-llm-stop",
    // 取消是單一全域旗標（engine/cancel.rs），這顆若在翻譯中可按，會把翻譯一起停掉。
    "btn-local-llm-cancel",
    "btn-discord-login",
    "btn-discord-join",
    "btn-discord-refresh",
    "btn-discord-logout",
    "btn-gpt-login",
    "btn-gpt-refresh",
    "btn-gpt-cancel",
    "btn-gpt-logout",
    "btn-open-login-url",
    "btn-copy-login-url",
    "btn-cancel-login",
    "btn-font-pick",
    "btn-font-out",
    "btn-font-remove",
    "btn-font-build",
    "btn-reference-pick",
    "btn-reference-file",
    "btn-share-confirm",
    "btn-share-cancel",
    "font-size",
    "font-thickness",
    "font-shift-x",
    "font-shift-y",
    "font-oversample",
    "font-apply-current",
    "btn-glossary",
    "target-version",
    "btn-helper-prepare",
    "btn-helper-rescan",
    "btn-helper-cleanup",
    "helper-ack-ingame",
  ];
  hardLockIds.forEach((id) => {
    const el = $(id);
    if (el) el.disabled = busy;
  });

  // 更新相關按鈕：翻譯中避免誤觸（「稍後」仍可關閉視窗）。「檢查更新」在設定視窗。
  const updateManualBtn = $("btn-update-manual");
  if (updateManualBtn) updateManualBtn.disabled = busy;
  const updateNowBtn = $("btn-update-now");
  if (updateNowBtn) updateNowBtn.disabled = busy;
  const updateLaterBtn = $("btn-update-close");
  if (updateLaterBtn) updateLaterBtn.disabled = false;

  // 忙碌中仍可切分頁瀏覽（開始鈕另由 syncUiState 看 progressBusy）
  ["tab-translate", "tab-font"].forEach((id) => {
    const el = $(id);
    if (el) el.disabled = false;
  });
  ["pack-name", "instance", "output", "reference-pack", "font-pack-name", "font-file", "font-output"].forEach((id) => {
    const el = $(id);
    if (el) {
      el.readOnly = !!busy;
      el.setAttribute("aria-readonly", busy ? "true" : "false");
    }
  });
  if (stop) stop.disabled = false;
  syncOutputField();
  syncStatusRail(document.body.dataset.appPage || "translate");
  syncUiState();
  if (!busy && typeof window.zfUpdateModalMaybeShowPending === "function") window.zfUpdateModalMaybeShowPending();
  if (!busy) packActions.maybeShowUpdateBanner();
  if (!busy && pendingUsageFeedbackNudge && translationState === "complete" && !isBlockingOverlayOpen()) {
    pendingUsageFeedbackNudge = false;
    scheduleUsageFeedbackNudge();
  }
}

/** 翻譯開始：不再收合主介面／最小化；僅靠 setBusy 鎖定操控 */
async function hideUiForTranslateRun() {
  /* no-op：主區保持可見 */
}

function showAppSection(_section) {
  showAppPage(lastWorkbenchPage || "translate");
}

/** 分頁：translate | font（診斷分頁已刪除；設定是獨立視窗 settings.html，不是這裡的分頁） */
function showAppPage(page, opts = {}) {
  const name = page === "font" ? "font" : "translate";
  const previous = document.body.dataset.appPage || "translate";
  const changed = previous !== name;
  const pageTr = $("page-translate");
  const pageFont = $("page-font");
  const tabTr = $("tab-translate");
  const tabFont = $("tab-font");
  if (pageTr) {
    pageTr.hidden = name !== "translate";
    pageTr.classList.toggle("active", name === "translate");
  }
  if (pageFont) {
    pageFont.hidden = name !== "font";
    pageFont.classList.toggle("active", name === "font");
  }
  if (tabTr) {
    tabTr.classList.toggle("active", name === "translate");
    tabTr.setAttribute("aria-selected", name === "translate" ? "true" : "false");
  }
  if (tabFont) {
    tabFont.classList.toggle("active", name === "font");
    tabFont.setAttribute("aria-selected", name === "font" ? "true" : "false");
  }
  syncWorkbenchTabKeys();
  lastWorkbenchPage = name;
  document.body.dataset.appPage = name;
  document.body.dataset.appSection = "workbench";
  const workbench = $("workbench-tabs");
  if (workbench) workbench.hidden = false;
  syncStatusRail(name);
  syncUiState();
  const activePanel = name === "font" ? pageFont : pageTr;
  if (changed && !opts.skipTransition) revealPagePanel(activePanel);
}

function wireTablistKeyboard() {
  document.querySelectorAll('[role="tablist"]').forEach((tablist) => {
    if (tablist.dataset.arrowNavReady === "1") return;
    tablist.dataset.arrowNavReady = "1";
    tablist.addEventListener("keydown", (ev) => {
      if (ev.ctrlKey || ev.metaKey || ev.altKey) return;
      const tabs = Array.from(tablist.querySelectorAll('[role="tab"]')).filter((tab) => !tab.disabled);
      if (!tabs.length) return;
      const current = tabs.indexOf(document.activeElement);
      let next = -1;
      if (ev.key === "ArrowRight" || ev.key === "ArrowDown") next = current < 0 ? 0 : (current + 1) % tabs.length;
      else if (ev.key === "ArrowLeft" || ev.key === "ArrowUp") next = current < 0 ? tabs.length - 1 : (current - 1 + tabs.length) % tabs.length;
      else if (ev.key === "Home") next = 0;
      else if (ev.key === "End") next = tabs.length - 1;
      if (next < 0) return;
      ev.preventDefault();
      tabs[next].focus();
      // 分頁只呼叫 showAppPage（idempotent），用 .click() 代打不會造成重複動作。
      tabs[next].click();
    });
  });
}

function syncStatusRail(page) {
  const name = page === "font" ? "font" : "translate";
  document.querySelectorAll(".rail-panel").forEach((el) => {
    const rail = el.getAttribute("data-rail") || "translate";
    el.hidden = rail !== name;
  });
}

function setTranslationState(state) {
  translationState = ["idle", "ready", "running", "complete", "failed"].includes(state)
    ? state
    : "idle";
  // 一進 running 就把來源標成 run：翻譯／補翻／修復三條路徑都會經過這裡。
  if (translationState === "running") {
    resultSource = "run";
    // 正在翻譯時「本機已有翻譯」卡片沒有意義：進度條已經在跑了，卡片還說
    // 「可接續補翻」是兩個互相矛盾的訊息同時出現在畫面上。
    hideLocalCacheCard();
  }
  document.body.dataset.translationState = translationState;
  document.body.dataset.resultSource = resultSource;
  syncUiState();
  if (translationState === "complete" || translationState === "failed") {
    refreshShareableState();
    freezeAllStepTimings();
  }
  // 只有「這次真的跑過翻譯」才問回饋。單純還原快取不算使用了一次。
  if (translationState === "complete" && resultSource === "run") {
    scheduleUsageFeedbackNudge();
  }
}

/** 右欄在沒跑過翻譯、但本機有舊結果時，要說「上次結果」而不是「尚未開始」。 */
function showCachedResultOnRail(probe) {
  const pct = $("prog-pct");
  const count = $("prog-count");
  const msg = $("prog-msg");
  const fill = $("prog-fill");
  if (fill) fill.style.width = "100%";
  if (pct) pct.textContent = "—";
  if (count) count.textContent = "上次的結果（這次尚未重跑）";
  if (msg) {
    const pending = Number(probe?.pendingCount || probe?.pending_count || 0) || 0;
    msg.textContent = pending > 0
      ? `本機已有翻譯結果，仍有 ${formatCount(pending)} 條待補`
      : "本機已有翻譯結果，可直接打開、再次套用或分享";
  }
  const steps = $("linear-steps");
  if (steps) {
    steps.querySelectorAll(".lin-step").forEach((el) => {
      el.classList.remove("active", "error");
      el.classList.add("done");
      const meta = el.querySelector("[data-step-meta]");
      if (meta) meta.hidden = true;
    });
  }
}

function toggleHidden(id, hidden) {
  const el = $(id);
  if (!el) return;
  el.hidden = !!hidden;
}

/* 「本包選項」只屬於目前整合包：在主工具用 modal 顯示，絕不跨到設定視窗。 */
function isMoreDrawerOpen() {
  const modal = $("pack-options-modal");
  return !!modal && !modal.hidden;
}

function openMoreDrawer() {
  const instanceReady = document.body.dataset.instanceReady === "1";
  const modal = $("pack-options-modal");
  const slot = $("pack-options-modal-slot");
  const host = $("pack-options-host");
  if (!instanceReady || !modal || !slot || !host) {
    appendLog("請先選擇並確認可用的遊戲資料夾，才能開啟本包選項。", "warn");
    return;
  }
  if (host.parentElement !== slot) slot.appendChild(host);
  syncPackOptionsAvailability();
  modal.hidden = false;
  modal.setAttribute("aria-hidden", "false");
  $("btn-pack-options-close")?.focus();
}

function closeMoreDrawer() {
  const modal = $("pack-options-modal");
  if (!modal) return;
  modal.hidden = true;
  modal.setAttribute("aria-hidden", "true");
  $("btn-more-options")?.setAttribute("aria-expanded", "false");
  $("btn-more-options")?.focus();
}


/**
 * 開啟「使用說明與免責」。
 *
 * 內容已經整合進設定頁的「說明」分頁，不再另開浮層——站長要求設定、本包選項、
 * 使用說明三者在同一個地方。這支函式因此變成「跳到設定頁的說明分頁」。
 */
function openGuideReader(_anchor = "") {
  // 設定／說明一律交給輕量 settings.html；不要在主工作台切到舊的內嵌設定頁。
  openAppSettings("help");
}

/** 沒選遊戲資料夾時，「本包選項」整區沒有意義——講明白，不要讓人對空欄位發呆。 */
function syncPackOptionsAvailability() {
  const host = $("pack-options-host");
  const hasInstance = document.body.dataset.instanceReady === "1";
  if (host) host.hidden = !hasInstance;
  if (!hasInstance && isMoreDrawerOpen()) closeMoreDrawer();
}

function openAppSettings(pane = "general") {
  if (pane === "legal" || pane === "guide") pane = "help";
  if (pane === "prefs") pane = "general";
  // 把設定開成第二個作業系統視窗（已開著就聚焦，不會開第二個）
  invoke("open_settings_window", {
    pane,
    theme: document.documentElement.dataset.theme === "light" ? "light" : "dark",
  }).catch((error) => {
    appendLog("無法開啟設定視窗：" + formatInvokeError(error), "warn");
  });
}

function wireShellChrome() {
  initWinbarChrome();
  // 「本包選項」只在已驗證的整合包上以主工具 modal 開啟。
  const moreBtn = $("btn-more-options");
  if (moreBtn) {
    moreBtn.setAttribute("aria-expanded", "false");
    moreBtn.setAttribute("aria-controls", "pack-options-modal");
    moreBtn.onclick = () => {
      if (isMoreDrawerOpen()) closeMoreDrawer();
      else openMoreDrawer();
      moreBtn.setAttribute("aria-expanded", isMoreDrawerOpen() ? "true" : "false");
    };
  }
  $("btn-pack-options-close")?.addEventListener("click", closeMoreDrawer);
  $("pack-options-modal-shade")?.addEventListener("click", closeMoreDrawer);

  const overflowBtn = $("btn-overflow");
  if (overflowBtn) {
    markWired(overflowBtn);
    overflowBtn.onclick = (ev) => {
      ev.stopPropagation();
      openAppSettings("general");
    };
  }

  if ($("btn-consent-accept")) {
    $("btn-consent-accept").onclick = hideConsentOverlay;
  }
  window.addEventListener("keydown", (ev) => {
    if (ev.key !== "Escape") return;
    // 同意頁按 Esc 不作用、不算同意（規格 D-01）；浮層的 Esc 由 ui/modal-scope.js 統一處理
    if (isMoreDrawerOpen()) {
      closeMoreDrawer();
    }
  });
}

function syncUiState() {
  // 設定視窗靠這個知道「翻譯中」與目前遊戲資料夾（刪備份、搬移、刪模型要停用）；狀態沒變不重送
  announceMainState();
  const hasInstance = !!($("instance")?.value || "").trim();
  const hasOutput = !!selectedOutputDir();
  const complete = translationState === "complete";
  const failed = translationState === "failed";
  const locked = progressBusy || shareUploadInFlight;
  const page = document.body.dataset.appPage || "translate";
  const instanceReady = hasInstance && !!instanceValidation.ok && !versionBlocked;
  document.body.dataset.instanceReady = instanceReady ? "1" : "0";
  syncPackMetaUi();

  const hideMore = !(hasInstance && !!instanceValidation.ok);
  const moreBtn = $("btn-more-options");
  if (moreBtn) moreBtn.hidden = hideMore;
  // 本包選項現在是設定視窗的一個分頁：沒選資料夾時只要把那一區換成提示文字，
  // 不要把整個設定視窗關掉——使用者可能正在看別的分頁。
  syncPackOptionsAvailability();
  ["field-output", "pack-version-group", "reference-details"]
    .forEach((id) => toggleHidden(id, !(hasInstance && !!instanceValidation.ok)));
  // 狀態卡（規格 §2）：一句現況＋唯一主要按鈕；AI 列只在狀態要求時出現
  const packState = packActions.renderStatusCard();
  const aiGroup = $("ai-options-group");
  if (aiGroup) {
    const showAi = packState.showAiRow && hasInstance && !!instanceValidation.ok;
    aiGroup.hidden = !showAi;
    aiGroup.setAttribute("aria-hidden", showAi ? "false" : "true");
  }
  const primaryAction = document.querySelector(".primary-action");
  if (primaryAction) primaryAction.hidden = !(hasInstance && !!instanceValidation.ok);
  const runDock = document.querySelector(".run-dock");
  if (runDock) {
    runDock.hidden =
      page === "translate" && !(hasInstance && !!instanceValidation.ok) && !progressBusy;
  }
  // 開始翻譯／停止翻譯在狀態卡（renderStatusCard），這裡不再另外控制
  // 「補充漏翻」與「重新翻譯缺漏」已移除：補翻整併進「開始翻譯」的
  // 「接續補完」選項與同輪自動重試；兩顆按鈕留著只會讓人不知道該按哪個。
  toggleHidden("btn-repair", !failed || locked);
  void refreshConsistencyMergeUi();
  // 只看磁碟有沒有可分享檔案（hasShareableFiles）會讓「分享給其他玩家」在選到一個
  // 本機早有舊結果的資料夾時就提早出現，跟這次根本還沒跑翻譯互相矛盾。注意：
  // probeLocalPackCache 找到快取時也會把 translationState 設成 "complete"（見
  // showCachedResultOnRail 那條路徑），單看 complete 篩不掉這個情境，要一併檢查
  // resultSource === "run"（真的走過這次翻譯／補翻／修復）——舊結果要分享
  // 走「本機已有翻譯」卡片自己的「打包分享」按鈕，這顆只在這次真的翻完才出現。
  const canShare = hasShareableFiles && !locked && complete && resultSource === "run";
  toggleHidden("btn-package", !canShare);
  // 「複製沒翻到的／貼回翻譯」跟分享同時機出現：這次真的跑完翻譯才有意義
  toggleHidden("btn-copy-failed", !canShare);
  toggleHidden("btn-import-translations", !canShare);
  clearShareUrlIfInstanceChanged();
  const fontFileReady = !!($("font-file")?.value || "").trim();
  const fontOutReady = !!($("font-output")?.value || "").trim();
  const translateOutReady = hasOutput;
  const fontBuild = $("btn-font-build");
  if (fontBuild) fontBuild.disabled = locked || !fontFileReady || !fontOutReady;
  const fontFileStatus = $("font-file-validate-status");
  if (fontFileStatus) {
    fontFileStatus.textContent = fontFileReady ? "已選取字體檔。" : "尚未選取字體檔。";
  }
  const fontOutStatus = $("font-output-validate-status");
  if (fontOutStatus) {
    fontOutStatus.textContent = fontOutReady ? "輸出位置已就緒。" : "尚未選擇輸出位置。";
  }
  const fontApplyCurrent = $("font-apply-current");
  if (fontApplyCurrent) {
    fontApplyCurrent.disabled = locked || !hasInstance;
  }
  toggleHidden("btn-open", page !== "translate" || !translateOutReady);
  toggleHidden("btn-open-report", page !== "translate" || !translateOutReady);
  toggleHidden("btn-open-font", page !== "font" || !fontOutReady);
  packActions.syncFolderArea();

  syncTranslationHelperPanel();

  const packageButton = $("btn-package");
  if (packageButton) {
    packageButton.disabled = !canShare;
    packageButton.textContent = lastShareUrl ? "複製分享連結" : "分享給其他玩家";
  }
  const shareHint = $("share-hint");
  if (shareHint) {
    shareHint.hidden = !canShare || shareConfirmationOpen;
    if (canShare && lastShareUrl) {
      shareHint.textContent = "連結已在剪貼簿；再按一次只會複製。";
    } else if (canShare && !shareConfirmationOpen) {
      shareHint.textContent = "完成後可分享帶密碼自解檔（只含一個最新工具資源包）。";
    }
  }
  const confirmPanel = $("share-confirm-panel");
  if (confirmPanel) confirmPanel.hidden = !shareConfirmationOpen || !hasShareableFiles;
  const confirmButton = $("btn-share-confirm");
  const reviewed = !!$("share-confirm-reviewed")?.checked;
  const privateFiles = !!$("share-confirm-private")?.checked;
  if (confirmButton) confirmButton.disabled = !reviewed || !privateFiles || shareUploadInFlight;
  if ($("btn-share-cancel")) $("btn-share-cancel").disabled = shareUploadInFlight;

  syncAiPanel(false);
  syncOutputField();
}

function syncTranslationHelperPanel() {
  const panel = $("translation-helper-panel");
  if (!panel) return;
  const status = translationHelperStatus;
  const hasInstance = !!($("instance")?.value || "").trim();
  const needed = !!status?.needed && hasInstance;
  panel.hidden = !needed;
  if (!needed) return;
  const message = $("translation-helper-message");
  if (message) message.textContent = status.message || "這是選用的任務補充步驟。";
  const command = $("translation-helper-command");
  const commandText = $("translation-helper-command-text");
  const readyInGame = status.supported && ["installed", "existing"].includes(status.state);
  const hasCommand = !!status.command && readyInGame;
  if (command) command.hidden = !hasCommand;
  if (commandText) commandText.textContent = status.command || "";
  const ackRow = $("helper-ack-row");
  const ack = $("helper-ack-ingame");
  if (ackRow) ackRow.hidden = !readyInGame || progressBusy;
  if (ack && progressBusy) ack.disabled = true;
  const prepare = $("btn-helper-prepare");
  if (prepare) {
    prepare.hidden = status.state !== "available" || progressBusy;
    prepare.disabled = progressBusy;
    prepare.textContent = "① 準備輔助模組";
  }
  const rescan = $("btn-helper-rescan");
  if (rescan) {
    const ackOk = !!(ack && ack.checked);
    const canRescan = readyInGame && ackOk && !progressBusy;
    rescan.hidden = !readyInGame || progressBusy;
    rescan.disabled = !canRescan;
    rescan.textContent = "③ 重新翻譯任務文字";
  }
  const cleanup = $("btn-helper-cleanup");
  if (cleanup) {
    cleanup.hidden = !status.installedByTool || progressBusy;
    cleanup.disabled = progressBusy;
  }
  const step1 = $("helper-step-1");
  const step2 = $("helper-step-2");
  const step3 = $("helper-step-3");
  [step1, step2, step3].forEach((el) => {
    if (!el) return;
    el.classList.remove("is-current", "is-done");
  });
  if (status.state === "available") {
    if (step1) step1.classList.add("is-current");
  } else if (readyInGame) {
    if (step1) step1.classList.add("is-done");
    if (ack && ack.checked) {
      if (step2) step2.classList.add("is-done");
      if (step3) step3.classList.add("is-current");
    } else {
      if (step2) step2.classList.add("is-current");
    }
  }
}

async function refreshTranslationHelper() {
  const instancePath = ($("instance")?.value || "").trim();
  if (!instancePath) {
    translationHelperStatus = null;
    syncUiState();
    return;
  }
  try {
    translationHelperStatus = await invoke("inspect_translation_helper_cmd", {
      instancePath,
      outputDir: selectedOutputDir() || null,
    });
  } catch (_) {
    translationHelperStatus = null;
  }
  syncUiState();
}

async function prepareTranslationHelper() {
  const instancePath = ($("instance")?.value || "").trim();
  const outputDir = selectedOutputDir();
  if (!instancePath || !outputDir) return log("請先選擇遊戲資料夾，讓工具知道要把狀態放在哪裡。");
  if (progressBusy) return;
  try {
    const result = await invoke("prepare_translation_helper_cmd", { instancePath, outputDir });
    translationHelperStatus = result;
    appendLog(result.message || "任務補充已準備好。", "info");
    if (result.command) appendLog("進入遊戲後執行：" + result.command, "info");
  } catch (e) {
    appendLog("任務補充已跳過：" + formatInvokeError(e), "warn");
    await refreshTranslationHelper();
  }
  syncUiState();
}

async function rescanAfterTranslationHelper() {
  if (
    !translationHelperStatus ||
    !translationHelperStatus.supported ||
    !["installed", "existing"].includes(translationHelperStatus.state)
  ) return;
  const ack = $("helper-ack-ingame");
  if (!ack || !ack.checked) {
    appendLog("請先勾選「我已啟動遊戲、執行指令並關閉遊戲」再重新翻譯。", "warn");
    return;
  }
  appendLog("開始重新掃描剛剛匯出的任務文字。", "info");
  await onRun();
}

async function cleanupPreparedTranslationHelper() {
  const instancePath = ($("instance")?.value || "").trim();
  const outputDir = selectedOutputDir();
  if (!instancePath || !outputDir) return;
  try {
    const result = await invoke("cleanup_translation_helper_cmd", { instancePath, outputDir });
    if (result.changed) {
      appendLog(result.message || "已清理暫時輔助模組。", "info");
      translationHelperStatus = { ...result, needed: false, supported: false, state: "cleaned" };
    }
  } catch (e) {
    appendLog("翻譯已完成，但輔助模組尚未刪除；請關閉遊戲後再試：" + formatInvokeError(e), "warn");
  }
  syncUiState();
}

async function cleanupTranslationHelperFromPanel() {
  await cleanupPreparedTranslationHelper();
  await refreshTranslationHelper();
}

function scheduleBackupStateRefresh() {
  if (backupProbeTimer) window.clearTimeout(backupProbeTimer);
  backupProbeTimer = window.setTimeout(() => {
    backupProbeTimer = 0;
    refreshBackupState();
  }, 180);
}

async function refreshBackupState() {
  const instancePath = ($("instance")?.value || "").trim();
  const outputDir = selectedOutputDir() || null;
  const token = ++backupProbeToken;
  if (!instancePath) {
    hasApplyBackups = false;
    syncUiState();
    return;
  }
  try {
    const found = await invoke("has_apply_backups_cmd", { instancePath, outputDir });
    if (token === backupProbeToken) hasApplyBackups = !!found;
  } catch (_) {
    if (token === backupProbeToken) hasApplyBackups = false;
  }
  if (token === backupProbeToken) syncUiState();
}

async function pickDir(title, defaultPath) {
  if (!dialog.open) throw new Error("無法開啟資料夾選擇視窗");
  const options = { directory: true, multiple: false, title };
  // 上次選過的位置只拿來當「起始位置」——方便，但不會自動填進輸入框，
  // 使用者仍然要自己確認這次要翻哪一包。
  const start = String(defaultPath || "").trim();
  if (start) options.defaultPath = start;
  const selected = await dialog.open(options);
  return typeof selected === "string" ? selected : null;
}

/** 將已完成的翻譯結果做成限時分享連結。 */
function clearShareUrlIfInstanceChanged() {
  const path = ($("instance")?.value || "").trim();
  if (lastShareUrl && path !== lastShareInstancePath) {
    lastShareUrl = "";
  }
}

async function copyShareUrl(url) {
  const steps =
    "【接收端四步驟】\n" +
    "1. 下載自解 exe，輸入密碼 cloud.zeitfrei.uk（下載頁也會顯示）\n" +
    "2. 執行後選整合包實例根目錄（需含 mods 或 resourcepacks；Prism 多實例勿選錯）\n" +
    "3. 完全關閉遊戲後重開，語言選繁體中文（台灣）\n" +
    "4. 資源包列表只啟用包內那一個「模組包翻譯工具+*」zip\n\n" +
    url;
  try {
    await navigator.clipboard.writeText(steps);
    appendLog("連結與安裝步驟已複製（24 小時有效）");
  } catch (_) {
    appendLog("無法寫入剪貼簿，請再按「複製分享連結」。", "warn");
  }
}

function packageShare() {
  if (lastShareUrl) {
    return copyShareUrl(lastShareUrl);
  }
  if (!hasShareableFiles) {
    return log("請先完成翻譯並產生可安裝檔，再建立分享檔。");
  }
  shareConfirmationOpen = true;
  syncUiState();
  $("share-confirm-panel")?.scrollIntoView({ behavior: "smooth", block: "nearest" });
}

async function confirmShareUpload() {
  if (!$("share-confirm-reviewed")?.checked || !$("share-confirm-private")?.checked) {
    return log("請先勾選兩項確認，再上傳分享檔。");
  }
  if (shareUploadInFlight) return;
  shareUploadInFlight = true;
  syncUiState();
  try {
    await uploadSharePackage();
  } finally {
    shareUploadInFlight = false;
    closeShareConfirmation();
  }
}

function closeShareConfirmation() {
  shareConfirmationOpen = false;
  ["share-confirm-reviewed", "share-confirm-private"].forEach((id) => {
    const el = $(id);
    if (el) el.checked = false;
  });
  syncUiState();
}

async function uploadSharePackage() {
  let outputDir = selectedOutputDir();
  let work = resultWorkDir(outputDir);
  if (localCacheProbe?.shareable && (localCacheProbe.workRoot || localCacheProbe.outputDir)) {
    work = localCacheProbe.workRoot || resultWorkDir(localCacheProbe.outputDir);
    outputDir = localCacheProbe.outputDir || outputDir;
  }
  if (!outputDir || !work) return log("請先完成翻譯（還沒有可打包的翻譯結果）。");
  try {
    const auth = await invoke("discord_auth_status");
    if (!auth || !(auth.loggedIn || auth.logged_in) || !(auth.inGuild || auth.in_guild)) {
      return log("分享前請先登入 Discord 並加入 ZeitFrei 官方伺服器。");
    }
    const ai = await invoke("ai_status");
    void ai;
    const name = ($("pack-name").value || "模組包翻譯分享").trim();
    appendLog("正在整理可安裝檔案並上傳…");
    const result = await invoke("upload_share_package_cmd", { workRoot: work, name });
    const url = String(result.url || result || "").trim();
    if (!url) {
      return log("分享失敗：服務沒有回傳連結。");
    }
    lastShareUrl = url;
    lastShareInstancePath = ($("instance")?.value || "").trim();
    await copyShareUrl(url);
  } catch (e) {
    log("分享失敗：\n" + formatInvokeError(e));
  }
}

function syncAiPanel(refreshStatus = true) {
  const panel = $("ai-panel");
  const enabled = !!$("use-ai")?.checked;
  if (panel) {
    panel.hidden = !enabled;
    panel.setAttribute("aria-hidden", enabled ? "false" : "true");
  }
  // #ai-options-group 顯示由 syncUiState 的 instanceReady 閘門控制
  if (enabled && refreshStatus) refreshAiStatus();
}

function normalizeAiMode(mode) {
  const m = String(mode || "").trim().toLowerCase();
  if (m === "gpt" || m === "custom") return m;
  return "local";
}

function aiModeFromUi() {
  if ($("ai-source-gpt")?.checked) return "gpt";
  if ($("ai-source-custom")?.checked) return "custom";
  return "local";
}

/**
 * 來源選擇是**唯一**的控制項，隱藏的 `#use-ai` 只是跟著它走的狀態欄位。
 *
 * 「不使用 AI」不是沒有翻譯——共享庫、術語表、翻譯記憶照樣全跑，
 * 而且那三層不需要登入也不需要下載模型。多數整合包這樣就能翻掉大部分。
 */
function syncUseAiFromSource() {
  const none = $("ai-source-none")?.checked;
  const useAi = $("use-ai");
  if (useAi) useAi.checked = !none;
  return !none;
}

function setAiModeRadios(mode) {
  const normalized = normalizeAiMode(mode);
  // 使用者上次選了「不使用 AI」時，不要被 normalizeAiMode 拉回 local
  const noneSelected = $("ai-source-none")?.checked && $("use-ai") && !$("use-ai").checked;
  if (!noneSelected) {
    if ($("ai-source-custom")) $("ai-source-custom").checked = normalized === "custom";
    if ($("ai-source-gpt")) $("ai-source-gpt").checked = normalized === "gpt";
    if ($("ai-source-local")) $("ai-source-local").checked = normalized === "local";
  }
  return normalized;
}

function isAiAuthBusy() {
  const discordOpen = $("discord-login-fallback") && !$("discord-login-fallback").hidden;
  const gptOpen = $("gpt-login-overlay") && !$("gpt-login-overlay").hidden;
  const localOpen = $("local-llm-overlay") && !$("local-llm-overlay").hidden;
  return !!(discordOpen || gptOpen || localOpen || gptLoginInFlight);
}

function showAiConfigPane(mode) {
  const normalized = normalizeAiMode(mode);
  if (isAiAuthBusy()) {
    return normalized;
  }
  if ($("managed-auth-panel")) $("managed-auth-panel").hidden = false;
  if ($("gpt-auth-panel")) $("gpt-auth-panel").hidden = normalized !== "gpt";
  if ($("local-llm-panel")) $("local-llm-panel").hidden = normalized !== "local";
  if ($("adv-details")) $("adv-details").hidden = normalized !== "custom";
  if ($("btn-test-api")) $("btn-test-api").hidden = normalized !== "custom";
  if ($("api-test-status") && normalized !== "custom") $("api-test-status").textContent = "";
  return normalized;
}

function syncAiModeUi(mode) {
  const normalized = setAiModeRadios(mode);
  currentAiMode = normalized;
  showAiConfigPane(normalized);
  return normalized;
}

function aiModeLabel(mode) {
  if (mode === "custom") return "自訂 API";
  if (mode === "gpt") return "ChatGPT";
  if (mode === "local") return "本地模型";
  return "自訂 API";
}

function showAiModeSwitching(mode) {
  const statusEl = $("key-status");
  const statusRow = statusEl?.closest(".ai-status");
  if (statusEl) statusEl.textContent = `AI：已切換至${aiModeLabel(mode)}，開始翻譯前確認`;
  if (statusRow) statusRow.dataset.state = "checking";
}

function applyDiscordStatusFromAi(s) {
  const loggedIn = !!(s && (s.loggedIn || s.logged_in));
  const inGuild = !!(s && (s.inGuild || s.in_guild));
  const serviceAvailable = s && (s.serviceAvailable ?? s.service_available) !== false;
  const displayName = String(
    (s && (s.discordDisplayName || s.discord_display_name || s.displayName || s.display_name)) || ""
  ).trim();
  const discordMessage = String((s && (s.discordMessage || s.discord_message)) || s?.message || "").trim();
  const title = $("discord-auth-title");
  const authNote = $("discord-auth-note");
  if (title) {
    title.textContent =
      loggedIn && inGuild
        ? `Discord 已驗證${displayName ? `：${displayName}` : ""}`
        : !loggedIn
          ? "Discord 尚未登入"
          : !serviceAvailable
            ? "Discord 登入服務連線失敗"
            : !inGuild
              ? "尚未加入官方伺服器"
              : "Discord 尚未驗證";
  }
  if (authNote) {
    authNote.textContent = !serviceAvailable
      ? discordMessage || "請檢查網路後按「重新檢查」。"
      : discordMessage || "翻譯前請登入 Discord 並加入官方伺服器（維護、收集建議、調整工具）。";
  }
  if ($("btn-discord-login")) $("btn-discord-login").hidden = loggedIn;
  if ($("btn-discord-logout")) $("btn-discord-logout").hidden = !loggedIn;
  if ($("btn-discord-join")) $("btn-discord-join").hidden = inGuild;
  // 已驗證就收成一行：政策上所有 AI 來源都要會籍，但「已經好了」不需要一直佔四顆按鈕的版面。
  // 沒好的時候維持展開，因為那正是使用者需要動手的時候。
  const panel = $("managed-auth-panel");
  if (panel) panel.dataset.settled = loggedIn && inGuild && serviceAvailable ? "1" : "0";
  refreshIssueReportUi();
}

async function refreshAiStatus() {
  if (refreshAiStatusInFlight) return refreshAiStatusInFlight;
  refreshAiStatusInFlight = (async () => {
    try {
      const statusEl = $("key-status");
      const noteEl = $("ai-source-note");
      const statusRow = statusEl?.closest(".ai-status");
      if (!statusEl) return;
      if (statusRow) statusRow.dataset.state = "checking";
      statusEl.textContent = "AI：正在確認";
      try {
        const s = await invoke("ai_status");
        latestAiStatus = s || null;
        const ready = s && s.ready !== false;
        const mode = String((s && (s.aiMode || s.ai_mode)) || aiModeFromUi());
        const usingOwnKey = !!(s && (s.usingOwnKey || s.using_own_key));
        const discordReady = !!(s && (s.discordReady || s.discord_ready));
        setAiModeRadios(mode);
        currentAiMode = normalizeAiMode(mode);
        if (!isAiAuthBusy()) showAiConfigPane(mode);
        applyDiscordStatusFromAi(s);
        if (mode === "gpt") {
          const gptStatus = await refreshGptStatus();
          const gptReady = ready && gptStatusIsUsable(gptStatus);
          const gptState = String(gptStatus?.state || "").toLowerCase();
          statusEl.textContent = gptReady
            ? "AI：ChatGPT 已登入，開始時會測試翻譯"
            : !discordReady
              ? "AI：請先完成 Discord 驗證"
              : gptState === "reauth_required"
                ? "AI：請重新登入 ChatGPT"
                : "AI：正在等待 ChatGPT 可用";
          if (statusRow) statusRow.dataset.state = gptReady ? "gpt" : "error";
          if (noteEl) noteEl.textContent = String(gptStatus?.message || GPT_COPY.noteGpt);
          return s;
        }
        if (mode === "local") {
          const localStatus = await localLlmStatus();
          const localReady = ready && !!(localStatus && localStatus.ready);
          statusEl.textContent = localReady
            ? "AI：本地模型已就緒，開始時會測試翻譯"
            : !discordReady
              ? "AI：請先完成 Discord 驗證"
              : localStatus && localStatus.installed
                ? "AI：本地模型已安裝，請啟動"
                : "AI：請先安裝本地模型";
          if (statusRow) statusRow.dataset.state = localReady ? "own" : "error";
          // ai-source-note 是「這個來源是什麼」的一句話說明，跟 local-llm-panel 內的
          // 安裝步驟說明是兩件事——舊版兩邊塞同一句「兩步：…」，畫面上逐字重複。
          if (noteEl) noteEl.textContent = GPT_COPY.noteLocalShort;
          if ($("local-llm-title")) {
            // 「服務尚未就緒」是內部狀態，不是使用者需要處理的事——檔案在、也同意過，
            // 工具自己把服務叫起來就好。使用者只需要知道「能不能用」。
            $("local-llm-title").textContent = localReady
              ? "本地模型可以使用"
              : localStatus && localStatus.installed
                ? "本地模型已就緒，第一次使用會花幾秒啟動"
                : "尚未安裝本地模型";
          }
          if ($("local-llm-note")) {
            $("local-llm-note").textContent = GPT_COPY.noteLocal;
          }
          // 就緒後才給「停用」出口：llama-server 會一直佔著記憶體／VRAM。
          // 刪除本地模型檔案只在設定→資料與備份（B5a-2）。
          const stopBtn = $("btn-local-llm-stop");
          if (stopBtn) stopBtn.hidden = !localReady;
          syncSetupButtonLabel(localReady);
          return s;
        }
        statusEl.textContent = ready
          ? "AI：自訂 API 已設定，開始時會測試翻譯"
          : !discordReady
            ? "AI：請先完成 Discord 驗證"
            : usingOwnKey
              ? "AI：請確認金鑰與 Discord"
              : "AI：請先設定自訂 API";
        if (statusRow) statusRow.dataset.state = ready ? "own" : "error";
        if (noteEl) noteEl.textContent = GPT_COPY.noteCustom;
        return s;
      } catch (e) {
        latestAiStatus = null;
        const detail = formatInvokeError(e);
        statusEl.textContent = "AI：狀態確認失敗";
        if (statusRow) statusRow.dataset.state = "error";
        if (noteEl) {
          noteEl.textContent =
            "無法讀取 AI 狀態：" + detail + "（本機簡繁轉換仍可用；需要 AI 時請檢查網路後重試）";
        }
        return null;
      }
    } finally {
      // 開發人員資格由後端在 ai_status 之後判定；顯示與開關在設定視窗（settings.html）。
      refreshAiStatusInFlight = null;
    }
  })();
  return refreshAiStatusInFlight;
}

async function refreshApiSettings() {
  try {
    const s = await invoke("get_api_settings");
    initApiKeyMask();
    setApiKeyMask(String(s.keyMasked || s.key_masked || ""));
    syncAiModeUi(String(s.aiMode || s.ai_mode || "local"));
    syncCustomProviderUi(String(s.provider || "deepseek"));
    const bu = (s.baseUrl || s.base_url || "").trim();
    const model = (s.model || "").trim();
    if ($("base-url")) $("base-url").value = bu;
    if ($("api-model")) $("api-model").value = model;
  } catch (e) {
    /* AI 狀態由 refreshAiStatus 顯示；設定讀取失敗不阻擋本機翻譯。 */
  }
}

function renderApiKeyMask() {
  const input = $("api-key");
  if (!input) return;
  input.value = apiKeyEditing
    ? "#".repeat(Math.min(apiKeyDraft.length, 128))
    : apiKeySavedMask;
}

function setApiKeyMask(mask) {
  apiKeyDraft = "";
  apiKeySavedMask = mask ? "########" : "";
  apiKeyEditing = false;
  renderApiKeyMask();
}

function replaceApiKeySelection(text) {
  const input = $("api-key");
  if (!input) return;
  const start = Math.max(0, Math.min(apiKeyDraft.length, input.selectionStart ?? apiKeyDraft.length));
  const end = Math.max(start, Math.min(apiKeyDraft.length, input.selectionEnd ?? start));
  apiKeyDraft = apiKeyDraft.slice(0, start) + text + apiKeyDraft.slice(end);
  renderApiKeyMask();
  const cursor = start + text.length;
  input.focus();
  input.setSelectionRange(cursor, cursor);
}

function initApiKeyMask() {
  const input = $("api-key");
  if (!input || input.dataset.maskReady === "true") return;
  input.dataset.maskReady = "true";
  input.addEventListener("focus", () => {
    apiKeyEditing = true;
    renderApiKeyMask();
    input.setSelectionRange(apiKeyDraft.length, apiKeyDraft.length);
  });
  input.addEventListener("blur", () => {
    if (!apiKeyDraft) {
      apiKeyEditing = false;
      renderApiKeyMask();
    }
  });
  input.addEventListener("beforeinput", (event) => {
    if (!apiKeyEditing) return;
    const type = event.inputType || "";
    if (type === "insertText" || type === "insertCompositionText" || type === "insertFromDrop") {
      event.preventDefault();
      replaceApiKeySelection(event.data || "");
    } else if (type === "insertFromPaste" && event.data != null) {
      event.preventDefault();
      replaceApiKeySelection(event.data);
    } else if (type === "deleteContentBackward" || type === "deleteContentForward" || type === "deleteByCut") {
      event.preventDefault();
      const inputEl = $("api-key");
      const start = inputEl?.selectionStart ?? apiKeyDraft.length;
      const end = inputEl?.selectionEnd ?? start;
      if (start !== end) {
        replaceApiKeySelection("");
      } else if (type === "deleteContentBackward" && start > 0) {
        inputEl.setSelectionRange(start - 1, start);
        replaceApiKeySelection("");
      } else if (type === "deleteContentForward" && start < apiKeyDraft.length) {
        inputEl.setSelectionRange(start, start + 1);
        replaceApiKeySelection("");
      }
    }
  });
  input.addEventListener("paste", (event) => {
    event.preventDefault();
    replaceApiKeySelection(event.clipboardData?.getData("text") || "");
  });
  input.addEventListener("drop", (event) => event.preventDefault());
}

function syncCustomProviderUi(provider) {
  const supported = ["deepseek", "glm", "openai", "qwen", "other"];
  const normalized = supported.includes(provider) ? provider : "deepseek";
  const select = $("api-provider");
  if (select) select.value = normalized;
  const isOther = normalized === "other";
  const fields = $("custom-endpoint-fields");
  if (fields) fields.hidden = !isOther;
  const base = $("base-url");
  const model = $("api-model");
  if (base) base.disabled = !isOther;
  if (model) model.disabled = !isOther;
  const note = $("api-provider-note");
  if (note) {
    if (isOther) {
      note.textContent = "請再填寫服務網址與模型名稱；一般使用者不需要改這些設定。";
    } else if (normalized === "glm") {
      note.textContent = "只要填 API 金鑰，工具會自動使用智譜 GLM 的官方設定。";
    } else if (normalized === "openai") {
      note.textContent = "只要填 API 金鑰，工具會自動使用 OpenAI 的官方設定。";
    } else if (normalized === "qwen") {
      note.textContent = "只要填 API 金鑰，工具會自動使用通義千問的官方設定。";
    } else {
      note.innerHTML =
        '推薦：便宜划算（官方 deepseek-v4-flash、非思考模式）。只要填 API 金鑰；金鑰申請 <a href="https://platform.deepseek.com" class="inline-ext-link" data-url="https://platform.deepseek.com">platform.deepseek.com</a>';
      note.querySelector("a.inline-ext-link")?.addEventListener("click", (e) => {
        e.preventDefault();
        const url = e.currentTarget.getAttribute("data-url");
        openExternalUrl(url);
      });
    }
  }
}

async function changeAiMode(mode) {
  const revision = ++aiModeRevision;
  const normalized = syncAiModeUi(mode);
  showAiModeSwitching(normalized);
  try {
    await invoke("set_ai_mode_cmd", { aiMode: normalized });
  } catch (e) {
    appendError("無法切換 AI 來源：" + formatInvokeError(e));
    if (revision === aiModeRevision) {
      const statusEl = $("key-status");
      const statusRow = statusEl?.closest(".ai-status");
      if (statusEl) statusEl.textContent = "AI：切換失敗，請重新選擇";
      if (statusRow) statusRow.dataset.state = "error";
    }
  }
  if (revision !== aiModeRevision) return latestAiStatus;
  // 切換事件不發起遠端驗證；真正開始翻譯時 ensureAiReadyForAction 會取得最新狀態。
  return latestAiStatus;
}

function queueAiModeChange(mode) {
  const normalized = syncAiModeUi(mode);
  // 立即更新畫面；設定檔寫入依序排入背景工作，避免快速切換時反轉最後選擇。
  aiModeWriteChain = aiModeWriteChain
    .catch(() => null)
    .then(() => changeAiMode(normalized));
  aiModeChangePromise = aiModeWriteChain;
  return aiModeChangePromise;
}

/**
 * 選了本地模型的人，第一次要先明確同意才會用到雲端補量（P0-05）。
 *
 * 「本地模型」這個選擇本身就表達了不想把文字送上網、也不想付費。舊版這個開關
 * 沒設定過時預設是開的，於是本地翻不好時會靜默改打雲端 API，用掉使用者自己的額度。
 * 後端已改成預設關閉；這裡負責問一次，讓想用的人有辦法打開。
 * 已經選過（不論開或關）的人不會再被問。
 */
async function ensureCloudTopUpConsent() {
  let view;
  try {
    view = await invoke("cloud_topup_choice_cmd");
  } catch (_) {
    return; // 問不到就當作沒同意，後端預設關閉，不影響翻譯進行
  }
  if (!view || !view.needsConsent) return;

  const yes = await confirmDialog({ ...CLOUD_TOPUP_CONSENT });
  try {
    setSetting(LOCAL_CLOUD_TOPUP_KEY, yes ? "1" : "0");
  } catch (_) {
    /* 存不起來就下次再問，不擋翻譯 */
  }
  appendLog(
    yes
      ? "已同意：本地翻不好時改用線上 AI 補完（會用到你的 API 額度）。"
      : "已選擇只用本地模型：文字不會送出，也不會產生費用。翻不好的句子會列進待補清單。",
    "info"
  );
}

async function ensureAiReadyForAction() {
  await aiModeChangePromise;
  let status = await refreshAiStatus();
  if (status && status.ready !== false) {
    const mode = String((status.aiMode || status.ai_mode) || aiModeFromUi());
    if (mode === "gpt") return gptStatusIsUsable(await refreshGptStatus());
    if (mode === "local") {
      await ensureCloudTopUpConsent();
      return ensureLocalLlmReady({ refreshAiStatus, appendLog, silent: true });
    }
    return true;
  }
  const mode = String((status && (status.aiMode || status.ai_mode)) || aiModeFromUi());
  if (mode === "local") {
    await ensureCloudTopUpConsent();
    return ensureLocalLlmReady({ refreshAiStatus, appendLog });
  }
  const message = String((status && status.message) || "目前無法確認 AI 狀態。");
  appendLog(message, "warn");
  if (!(status && (status.discordReady || status.discord_ready))) {
    $("managed-auth-panel")?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  } else if (mode === "custom") {
    $("api-key")?.focus();
  } else if (mode === "gpt") {
    $("gpt-auth-panel")?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }
  return false;
}

/**
 * 自動送出失敗時，不要只丟一句「送不出去」就結束。
 *
 * 使用者剛打完一段詳細說明，被告知失敗卻沒有下一步，等於白打一次。這裡把他
 * 剛填的內容整理好複製到剪貼簿，再問要不要直接開 Discord——貼上去就好。
 */
/** 金鑰是否落地。開關在設定視窗；主視窗只在啟動時把目前選擇告訴後端。 */
const REMEMBER_KEY_STORAGE = "modpack-i18n-remember-api-key-v1";
/** 本地翻不好時是否改用線上 AI 補完。後端讀同一個鍵（translate.localCloudTopUp）。 */
const LOCAL_CLOUD_TOPUP_KEY = "modpack-i18n-local-cloud-topup-v1";

async function wirePrivacySettings() {
  // 既有已存金鑰的使用者升級後不能突然不能用：偵測到設定檔裡已經有金鑰就
  // 預設「記住」，全新使用者則預設不記住（使用者要求金鑰不落地）。
  const saved = getSetting(REMEMBER_KEY_STORAGE, null);
  let initial = saved === "1";
  if (saved === null) {
    try {
      const view = await invoke("get_api_settings");
      initial = !!(view && (view.hasKey ?? view.has_key));
    } catch (_) {
      initial = false;
    }
    // 記下推定值，設定視窗才顯示得出目前的選擇
    setSetting(REMEMBER_KEY_STORAGE, initial ? "1" : "0");
  }
  await invoke("set_remember_api_key_cmd", { remember: initial }).catch(() => {});
}

/**
 * 翻譯結束（成功或失敗都算）後把本地模型收掉。
 *
 * llama-server 會一直佔著記憶體與顯示卡，翻完不關等於整台電腦被綁住。
 * 「翻譯後保持常駐」設定已移除：本地模型一律翻完就關。
 *
 * B4：以輪次編號避免關閉競態——翻完立刻按「接續補完」時，上一輪遲到的關閉
 * 不得關掉新一輪要用的模型（見 core/local-model-round.js，後端也會再檢查一次）。
 */
const localModelRounds = createLocalModelRounds({
  invoke,
  isLocalMode: () => aiModeFromUi() === "local",
  log: (msg) => appendLog(msg),
  onStopped: () => void refreshAiStatus(),
  markIdle: () => invoke("set_translation_active_cmd", { active: false }).catch(() => {}),
});

async function releaseLocalModelAfterRun(round) {
  await localModelRounds.release(round);
}

/**
 * 翻譯套用後檢查資源包清單是否健康，壞掉就主動提供修復。
 *
 * 這是使用者那次閃退換來的：整合包自己的 ~150 個資源包全部沒被啟用，
 * 字體 builder 找不到 prominent:textures/gui/realms.png，
 * 「Default font failed to load」→ 模型沒烘焙 → 標題畫面空指標。
 * 光看遊戲的錯誤訊息完全看不出跟翻譯有關，所以工具要自己抓出來。
 */
async function checkResourcePackHealth(instancePath) {
  if (!instancePath) return;
  let report;
  try {
    report = await invoke("verify_resource_packs_cmd", { instancePath });
  } catch (_) {
    return;
  }
  const disabled = Array.isArray(report?.presentButDisabled) ? report.presentButDisabled : [];
  // 修復範圍已縮到「工具動過的項目」，所以有一個就值得問，不必等到超過三個
  if (!report?.listEmpty && disabled.length === 0) return;

  appendLog(report.summary || "資源包清單可能不完整。", "warn");
  const shown = disabled.slice(0, 6).map((n) => "・" + n).join("\n");
  const more = disabled.length > 6 ? "\n…共 " + disabled.length + " 個" : "";
  // 說明要回答四件事：為什麼要修、怎麼會這樣、修了什麼、不修會怎樣。
  // 少了任何一項，使用者只能憑感覺按下去——那不是知情的選擇。
  const backupNote = report?.backupUsed
    ? "工具比對的是套用前的備份「" + report.backupUsed + "」。"
    : "這台電腦上找不到套用前的備份，所以只會加回工具自己的翻譯包。";
  const fix = await confirmDialog({
    title: report.listEmpty ? "資源包清單是空的" : "資源包清單少了東西",
    body:
      (report.summary || "") +
      "\n\n【為什麼要修】\n" +
      "遊戲設定檔裡的資源包清單少了項目。翻譯包不在清單裡，遊戲就讀不到翻譯，" +
      "你會看到全部都還是英文。\n" +
      "\n【怎麼會這樣】\n" +
      "多半是遊戲或啟動器在套用之後重新寫過這個檔案，把清單蓋掉了；也可能是手動改過。" +
      backupNote +
      "\n\n【會加回哪些】\n" + shown + more +
      "\n翻譯包會排在最後（優先權最高，才蓋得過其他語言的資源包）。" +
      "\n**整合包原本就關著的資源包不會被啟用**——那是整合包作者刻意關掉的，工具不動它。" +
      "\n\n【不修會怎樣】\n" +
      "翻譯不會生效。先前實測還遇過資源包遺失導致字體載入失敗、進而開不了遊戲。" +
      "\n\n修改前會先另存一份遊戲設定檔，改壞了可以還原。",
    confirmLabel: "修復資源包清單",
    cancelLabel: "先不要",
    danger: !!report.listEmpty,
  });
  if (!fix) return;
  try {
    const fixed = await invoke("repair_resource_packs_cmd", { instancePath });
    appendLog((fixed?.summary || "已修復資源包清單。") + "請重新啟動遊戲確認。");
    showAppToast("已修復資源包清單，請重開遊戲", 3000);
  } catch (e) {
    appendLog("修復失敗：" + formatInvokeError(e), "warn");
  }
}

/**
 * 翻譯進行中被要求關閉工具時的處理。
 *
 * 使用者實測過「翻到一半關掉、重開續翻」的後果：前一小時的紀錄被覆寫、
 * 「不備份直接覆蓋」的選擇也不見了。後端現在會攔下關閉事件並發這個訊號，
 * 由這裡問使用者，並在他確定要離開時**先把日誌與進度落檔**再退出。
 */
async function handleCloseWhileBusy() {
  const jobName =
    window.__busyJobKind === "repair" ? "修復" :
    window.__busyJobKind === "supplement" ? "補充漏翻" : "翻譯";
  const leave = await confirmDialog({
    title: jobName + "還在進行中",
    body:
      "現在關閉會中斷" + jobName + "。已經完成的部分會保留，下次可以接續，\n" +
      "但這一輪還沒寫出的內容會遺失。\n\n" +
      "如果只是想把視窗收起來，可以按「繼續執行」，工具會在背景把它跑完。",
    confirmLabel: "仍要關閉",
    cancelLabel: "繼續執行",
    danger: true,
  });
  if (!leave) return;
  appendLog("使用者選擇中斷並關閉工具，正在保存進度…", "warn");
  try {
    await invoke("cancel_task").catch(() => {});
  } catch (_) { /* 取消失敗不影響落檔 */ }
  try {
    const outputDir = selectedOutputDir();
    if (outputDir) await flushRunLog(resultWorkDir(outputDir));
  } catch (_) { /* 落檔失敗也要讓使用者關得掉 */ }
  await invoke("set_translation_active_cmd", { active: false }).catch(() => {});
  await invoke("quit_app").catch(() => {});
}

/**
 * 把沒翻到的項目整張表複製到剪貼簿。
 *
 * 使用者反映：想拿去線上 AI 翻，但目前只能一個一個開檔案複製「有點慘」。
 * 表格格式就是 `命名空間,鍵,原文,譯文,原因`——譯文欄留空給他填，
 * 填完直接用「貼回翻譯」貼回來就好。
 */
async function onCopyFailedItems() {
  const outputDir = selectedOutputDir();
  if (!outputDir) return appendLog("還沒有翻譯結果可以匯出。", "warn");
  try {
    const csv = await invoke("failed_items_csv_cmd", { outputDir });
    const lines = String(csv || "").split("\n").filter(Boolean).length - 1;
    if (lines <= 0) {
      return appendLog("這一包沒有待補項目，不需要匯出。");
    }
    await navigator.clipboard.writeText(csv);
    appendLog(
      `已複製 ${formatCount(lines)} 條沒翻到的項目到剪貼簿。貼到線上 AI 請它翻「譯文」那一欄，` +
        "翻完把整張表（或「鍵<Tab>譯文」兩欄）複製起來，回來按「貼回翻譯」。" +
        "工具會逐條檢查 %s、§ 這類格式符號，對不上的會退回不寫進遊戲。"
    );
    showAppToast(`已複製沒翻到的 ${formatCount(lines)} 條`, 3000);
  } catch (e) {
    appendLog("匯出失敗：" + formatInvokeError(e), "warn");
  }
}

/** 把線上翻好的內容貼回來併入翻譯結果。 */
async function onImportTranslations() {
  const outputDir = selectedOutputDir();
  if (!outputDir) return appendLog("還沒有翻譯結果可以匯入。", "warn");
  let text = "";
  try {
    text = await navigator.clipboard.readText();
  } catch (_) {
    return appendLog("讀不到剪貼簿內容。請先複製翻好的表格再按一次。", "warn");
  }
  if (!String(text || "").trim()) {
    return appendLog("剪貼簿是空的。請先複製翻好的表格。", "warn");
  }
  const go = await confirmDialog({
    title: "要把剪貼簿的翻譯併入嗎？",
    body:
      "會逐條檢查格式符號（%s、§ 等），對不上的原樣退回不寫進遊戲。\n\n" +
      "併入後會重建翻譯資源包，你需要重新套用或重開遊戲才看得到。",
    confirmLabel: "併入",
    cancelLabel: "取消",
  });
  if (!go) return;
  try {
    const report = await invoke("import_translations_cmd", { outputDir, text });
    // 通知型對話框改成紀錄＋toast（規格 §1.2）；紀錄寫完整清單，toast 照實（沒併入就不說已併入）
    const view = describeImportReport(report);
    appendLog(view.log || "匯入完成。", view.level);
    showAppToast(view.toast, 3000);
  } catch (e) {
    appendLog("匯入失敗：" + formatInvokeError(e), "warn");
  }
}

async function openExternalUrl(url) {
  if (!url) return;
  try {
    await invoke("open_url", { url });
  } catch (e) {
    appendLog("無法開啟連結：" + formatInvokeError(e), "warn");
    throw e;
  }
}

let _appToastTimer = null;

function showAppToast(message, ms = 2200) {
  let el = document.getElementById("app-toast");
  if (!el) {
    el = document.createElement("div");
    el.id = "app-toast";
    el.className = "app-toast";
    el.setAttribute("role", "status");
    // 對話框開著時背景會 inert，toast 要照樣讀得到（ui/modal-scope.js）
    el.dataset.modalExempt = "1";
    document.body.appendChild(el);
  }
  el.textContent = String(message || "");
  el.classList.add("show");
  if (_appToastTimer) clearTimeout(_appToastTimer);
  _appToastTimer = setTimeout(() => {
    el.classList.remove("show");
  }, ms);
}

function isMissingCommandError(error) {
  const detail = String(formatInvokeError(error || "") || "").toLowerCase();
  return (
    detail.includes("unknown command") ||
    detail.includes("not found") ||
    detail.includes("unknown variant") ||
    detail.includes("does not exist")
  );
}

async function invokeFirstAvailable(commandNames, payload) {
  let lastError = null;
  for (let i = 0; i < commandNames.length; i += 1) {
    const name = commandNames[i];
    try {
      return payload === undefined ? await invoke(name) : await invoke(name, payload);
    } catch (error) {
      lastError = error;
      if (!isMissingCommandError(error) || i === commandNames.length - 1) {
        throw error;
      }
    }
  }
  throw lastError || new Error("找不到可用 command。");
}

function gptDisplayName(status) {
  return String((status && (status.email || status.name || status.accountId || status.account_id)) || "").trim();
}

function renderGptStatus(status) {
  const state = String(status?.state || "").trim().toLowerCase();
  const loggedIn = !!(status && (status.loggedIn || status.logged_in));
  const usable = status?.usable === true || state === "ready";
  const title = $("gpt-auth-title");
  const note = $("gpt-auth-note");
  const logoutBtn = $("btn-gpt-logout");
  const cancelBtn = $("btn-gpt-cancel");
  const loginBtn = $("btn-gpt-login");
  const statusName = gptDisplayName(status);
  if (title) {
    title.textContent = usable
      ? `ChatGPT 已登入：${statusName || "ChatGPT 帳號"}（開始翻譯時會測試）`
      : loggedIn
        ? state === "reauth_required"
          ? `需重新登入：${statusName || "ChatGPT 帳號"}`
          : `已保存：${statusName || "ChatGPT 帳號"}`
        : GPT_COPY.statusLoggedOut;
  }
  if (note) {
    if (gptLoginInFlight) {
      note.textContent = GPT_COPY.statusPending;
    } else if (status && status.message) {
      note.textContent = String(status.message);
    } else {
      note.textContent = loggedIn ? GPT_COPY.noteGpt : GPT_COPY.statusLoggedOut;
    }
  }
  if (logoutBtn) logoutBtn.hidden = !loggedIn;
  if (cancelBtn) cancelBtn.hidden = !gptLoginInFlight;
  if (loginBtn) loginBtn.hidden = usable;
  const panel = $("gpt-auth-panel");
  if (panel) panel.dataset.authState = state || (loggedIn ? "saved" : "logged_out");
  return usable;
}

async function refreshGptStatus(force = false) {
  const now = Date.now();
  if (!force && gptStatusCache && now - gptStatusCheckedAt < 4000) {
    renderGptStatus(gptStatusCache);
    return gptStatusCache;
  }
  if (gptStatusInFlight) return gptStatusInFlight;
  gptStatusInFlight = (async () => {
    let status = null;
    try {
      status = await invokeFirstAvailable(["gpt_auth_status_cmd", "gpt_auth_status"], undefined);
    } catch (error) {
      status = {
        loggedIn: false,
        logged_in: false,
        usable: false,
        state: "check_failed",
        message: GPT_COPY.loginFailed + " " + formatInvokeError(error),
      };
    }
    gptStatusCache = status;
    gptStatusCheckedAt = Date.now();
    renderGptStatus(status);
    return status;
  })().finally(() => {
    gptStatusInFlight = null;
  });
  return gptStatusInFlight;
}

function gptStatusIsUsable(status) {
  return !!(status && (status.usable === true || String(status.state || "").toLowerCase() === "ready"));
}

async function beginGptLogin() {
  const loginBtn = $("btn-gpt-login");
  if (loginBtn) loginBtn.disabled = true;
  gptLoginInFlight = true;
  await refreshGptStatus(true);
  try {
    const result = await invokeFirstAvailable(["gpt_login"], undefined);
    if (result && result.ok) {
      appendLog("ChatGPT 登入完成。", "info");
      showAppToast("ChatGPT 已登入");
      markGptLoginOverlayDone();
    } else {
      const reason = String((result && result.error) || "登入未完成");
      if (reason === "cancelled") {
        appendLog("已取消 ChatGPT 登入。", "warn");
      } else if (reason === "timeout") {
        appendLog("ChatGPT 登入逾時，請重新登入。", "warn");
      } else {
        appendLog(GPT_COPY.loginFailed + " " + reason, "warn");
      }
    }
  } catch (error) {
    appendError("ChatGPT 登入失敗：" + formatInvokeError(error));
  } finally {
    gptLoginInFlight = false;
    if (loginBtn) loginBtn.disabled = false;
    gptStatusCache = null;
    await refreshAiStatus();
  }
}

async function cancelGptLoginFlow() {
  try {
    await invokeFirstAvailable(["cancel_gpt_login_cmd", "cancel_gpt_login"], undefined);
    appendLog("已要求取消 ChatGPT 登入。", "warn");
  } catch (error) {
    appendError("無法取消 ChatGPT 登入：" + formatInvokeError(error));
  }
}

async function logoutGpt() {
  const ok = await confirmDialog({
    title: "登出 ChatGPT？",
    body: "登出後要再用 ChatGPT 翻譯，得重新在瀏覽器完成一次登入。本機的翻譯結果不受影響。",
    confirmLabel: "登出",
    cancelLabel: "先不要",
  });
  if (!ok) return;
  try {
    await invokeFirstAvailable(["gpt_logout_cmd", "gpt_logout"], undefined);
    appendLog("已登出 ChatGPT。", "info");
  } catch (error) {
    appendError("ChatGPT 登出失敗：" + formatInvokeError(error));
  } finally {
    gptStatusCache = null;
    await refreshAiStatus();
  }
}

let gptLoginUrl = "";
let gptDeviceCode = "";

function showGptLoginOverlay(payload) {
  const overlay = $("gpt-login-overlay");
  if (!overlay) return;
  gptDeviceCode = String((payload && (payload.userCode || payload.user_code)) || gptDeviceCode || "").trim();
  gptLoginUrl = String((payload && (payload.url || payload.verifyUrl)) || gptLoginUrl || "").trim();
  if ($("gpt-device-code")) $("gpt-device-code").textContent = gptDeviceCode || "————";
  if ($("gpt-login-overlay-note")) {
    $("gpt-login-overlay-note").textContent = "請在瀏覽器輸入此授權碼，或已開啟的頁面確認。";
  }
  if ($("gpt-login-overlay-status")) $("gpt-login-overlay-status").textContent = "";
  if ($("gpt-login-overlay-title")) $("gpt-login-overlay-title").textContent = "裝置授權碼";
  overlay.hidden = false;
  overlay.setAttribute("aria-hidden", "false");
}

function closeGptLoginOverlay() {
  const overlay = $("gpt-login-overlay");
  if (!overlay) return;
  overlay.hidden = true;
  overlay.setAttribute("aria-hidden", "true");
}

function markGptLoginOverlayDone() {
  if ($("gpt-login-overlay-title")) $("gpt-login-overlay-title").textContent = "已登入";
  if ($("gpt-login-overlay-note")) {
    $("gpt-login-overlay-note").textContent = "ChatGPT 登入完成。可按關閉。";
  }
  if ($("gpt-login-overlay-status")) $("gpt-login-overlay-status").textContent = "登入成功";
  const overlay = $("gpt-login-overlay");
  if (overlay) {
    overlay.hidden = false;
    overlay.setAttribute("aria-hidden", "false");
  }
}


function pathLeaf(path) {
  const text = String(path || "").trim();
  if (!text) return "";
  const parts = text.split(/[\\/]+/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : text;
}

const PACK_META_STORAGE_PREFIX = "mcpl.packMeta.";

function packMetaStorageKey(instancePath) {
  const path = String(instancePath || "").trim().toLowerCase();
  if (!path) return "";
  let hash = 0;
  for (let i = 0; i < path.length; i += 1) {
    hash = (Math.imul(31, hash) + path.charCodeAt(i)) | 0;
  }
  return PACK_META_STORAGE_PREFIX + Math.abs(hash).toString(36);
}

// 整合包資訊要跨 session 記住：舊版存 sessionStorage，關掉工具就忘，同一個包每次都要重填。
function loadPackMeta(instancePath) {
  const key = packMetaStorageKey(instancePath);
  if (!key) return {};
  for (const store of [localStorage, sessionStorage]) {
    try {
      const raw = store.getItem(key);
      if (!raw) continue;
      const parsed = JSON.parse(raw);
      if (parsed && typeof parsed === "object") return parsed;
    } catch (_) {
      /* 換下一個來源 */
    }
  }
  return {};
}

function savePackMeta(instancePath, data) {
  const key = packMetaStorageKey(instancePath);
  if (!key) return;
  try {
    localStorage.setItem(key, JSON.stringify(data || {}));
  } catch (_) {
    /* ignore quota */
  }
}

function packMetaFromForm() {
  return {
    displayName: ($("pack-display-name")?.value || "").trim(),
    packRef: ($("pack-ref")?.value || "").trim(),
    packVersion: ($("pack-version-meta")?.value || "").trim(),
    skipped: false,
  };
}

function formatPackMetaSummary(meta) {
  const parts = [];
  if (meta.displayName) parts.push(`名稱：${meta.displayName}`);
  if (meta.packRef) parts.push(`連結／包名：${meta.packRef}`);
  if (meta.packVersion) parts.push(`版本：${meta.packVersion}`);
  return parts.join(" · ") || "已儲存整合包資訊";
}

function syncPackMetaUi() {
  const card = $("pack-meta-card");
  if (!card) return;
  const instanceReady = document.body.dataset.instanceReady === "1";
  const instancePath = ($("instance")?.value || "").trim();
  if (!instanceReady || !instancePath) {
    card.hidden = true;
    return;
  }
  const meta = loadPackMeta(instancePath);
  const form = $("pack-meta-form");
  const summary = $("pack-meta-summary");
  const toggle = $("btn-pack-meta-toggle");
  const editing = form?.dataset.editing === "1";
  const hasSaved = !!(meta.displayName || meta.packRef || meta.packVersion);
  card.hidden = false;
  // 按過「略過」就收成一行，但仍留「填寫資訊」可以回頭。舊版寫了 skipped 卻沒人讀，
  // 那顆按鈕等於按了沒反應；直接整張藏掉又會變成無法反悔的死路。
  const skipped = !!meta.skipped && !hasSaved && !editing;
  card.dataset.skipped = skipped ? "1" : "0";
  if (hasSaved && summary && !editing) {
    if (form) form.hidden = true;
    summary.hidden = false;
    const text = $("pack-meta-summary-text");
    if (text) text.textContent = formatPackMetaSummary(meta);
    if (toggle) {
      toggle.textContent = "修改";
      toggle.setAttribute("aria-expanded", "false");
    }
  } else if (editing) {
    if (form) form.hidden = false;
    if (summary) summary.hidden = true;
    if (toggle) {
      toggle.textContent = "收合";
      toggle.setAttribute("aria-expanded", "true");
    }
    if ($("pack-display-name")) $("pack-display-name").value = meta.displayName || "";
    if ($("pack-ref")) $("pack-ref").value = meta.packRef || "";
    if ($("pack-version-meta")) $("pack-version-meta").value = meta.packVersion || "";
  } else {
    if (form) form.hidden = true;
    if (summary) summary.hidden = false;
    const text = $("pack-meta-summary-text");
    if (text) {
      text.textContent = skipped ? "已略過；需要時可再填寫。" : "尚未填寫；不填也能翻譯。";
    }
    if (toggle) {
      toggle.textContent = "填寫資訊";
      toggle.setAttribute("aria-expanded", "false");
    }
    if ($("pack-display-name")) $("pack-display-name").value = meta.displayName || "";
    if ($("pack-ref")) $("pack-ref").value = meta.packRef || "";
    if ($("pack-version-meta")) $("pack-version-meta").value = meta.packVersion || "";
  }
}

function savePackMetaFromForm() {
  const instancePath = ($("instance")?.value || "").trim();
  if (!instancePath) return;
  const data = packMetaFromForm();
  savePackMeta(instancePath, data);
  appendLog("已記住這個整合包的資訊，下次開工具不用再填。");
  const form = $("pack-meta-form");
  if (form) delete form.dataset.editing;
  syncPackMetaUi();
}

function skipPackMetaCard() {
  const instancePath = ($("instance")?.value || "").trim();
  if (!instancePath) return;
  savePackMeta(instancePath, { ...loadPackMeta(instancePath), skipped: true });
  const form = $("pack-meta-form");
  if (form) delete form.dataset.editing;
  syncPackMetaUi();
}

function editPackMetaCard() {
  const form = $("pack-meta-form");
  if (form) form.dataset.editing = "1";
  // 使用者主動要填了，就不再算「已略過」。
  const path = ($("instance")?.value || "").trim();
  if (path) {
    const meta = loadPackMeta(path);
    if (meta.skipped) savePackMeta(path, { ...meta, skipped: false });
  }
  const summary = $("pack-meta-summary");
  if (summary) summary.hidden = true;
  if (form) form.hidden = false;
  const instancePath = ($("instance")?.value || "").trim();
  const meta = loadPackMeta(instancePath);
  if ($("pack-display-name")) $("pack-display-name").value = meta.displayName || "";
  if ($("pack-ref")) $("pack-ref").value = meta.packRef || "";
  if ($("pack-version-meta")) $("pack-version-meta").value = meta.packVersion || "";
  const toggle = $("btn-pack-meta-toggle");
  if (toggle) {
    toggle.textContent = "收合";
    toggle.setAttribute("aria-expanded", "true");
  }
}

function togglePackMetaForm() {
  const form = $("pack-meta-form");
  const toggle = $("btn-pack-meta-toggle");
  if (!form) return;
  const expanded = form.dataset.editing === "1" || toggle?.getAttribute("aria-expanded") === "true";
  if (!expanded) {
    editPackMetaCard();
  } else {
    delete form.dataset.editing;
    syncPackMetaUi();
  }
}

function wirePackMetaCard() {
  const pairs = [
    ["btn-pack-meta-toggle", togglePackMetaForm],
    ["btn-pack-meta-save", savePackMetaFromForm],
    ["btn-pack-meta-skip", skipPackMetaCard],
  ];
  pairs.forEach(([id, handler]) => {
    const el = $(id);
    if (!el || el.dataset.wired) return;
    el.dataset.wired = "1";
    el.addEventListener("click", handler);
  });
}

function wireCriticalUiDelegation() {
  if (document.documentElement.dataset.criticalUiDelegated === "1") return;
  document.documentElement.dataset.criticalUiDelegated = "1";

  const run = (id, fn) => {
    try {
      const out = fn();
      if (out && typeof out.then === "function") out.catch((e) => console.warn("[delegate]", id, e));
    } catch (e) {
      console.warn("[delegate]", id, e);
    }
  };

  document.addEventListener(
    "click",
    (ev) => {
      const btn = ev.target?.closest?.("button");
      if (!btn?.id) return;
      if (btn.dataset.wired === "1") return;
      if (btn.disabled) return;
      switch (btn.id) {
        case "btn-pack-meta-toggle":
          run(btn.id, () => togglePackMetaForm());
          break;
        case "btn-pack-meta-save":
          run(btn.id, () => savePackMetaFromForm());
          break;
        case "btn-pack-meta-skip":
          run(btn.id, () => skipPackMetaCard());
          break;
        case "tab-translate":
          run(btn.id, () => showAppPage("translate"));
          break;
        case "tab-font":
          run(btn.id, () => showAppPage("font"));
          break;
        case "btn-inst":
          run(btn.id, () => onPickInstance());
          break;
        case "btn-run":
          run(btn.id, () => onRun());
          break;
        case "btn-stop":
          run(btn.id, () => onStop());
          break;
        case "btn-quit":
          run(btn.id, () => onQuitApp());
          break;
        case "btn-overflow":
          run(btn.id, () => openAppSettings("general"));
          break;
        case "btn-repair":
          run(btn.id, () => onRepair());
          break;
        case "btn-package":
          run(btn.id, () => packageShare());
          break;
        default:
          break;
      }
    },
    true
  );
}

function markWired(el) {
  if (el) el.dataset.wired = "1";
}

async function onQuitApp() {
  try {
    await invoke("quit_app");
  } catch (e) {
    window.close();
  }
}

/**
 * 採用一個遊戲資料夾：驗證、歸零、算結果位置、探本機快取。
 *
 * 從「瀏覽…」與「接續補完」兩個入口共用同一段流程——兩邊要走一模一樣的路，
 * 不然接續進來的狀態會跟自己選資料夾進來的不一致。
 */
async function adoptInstancePath(p, { silentProbe = false } = {}) {
  // 換整合包＝一切從頭：步驟燈號、計時、統計都要歸零，
  // 否則上一包的狀態會留在畫面上，看起來像這一包已經翻過。
  resetStepPanelForNewInstance();
  hideLocalCacheCard();
  packActions.clearRemoval();
  $("instance").value = p;
  writeLastInstancePath(p);
  const ok = await validateSelectedInstance(p);
  setTranslationState(ok ? "ready" : "idle");
  if (ok) {
    // 選資料夾成功一次＝熟手：S01 的附加說明退場（規格 §4.2 pick-folder）
    disclosure.retire("pickFolder");
    resumeOnboarding();
  }
  await detectVersionForInstance(p, false);
  await refreshPackTranslationName(p);
  await refreshReferencePack();
  $("output").value = "";
  $("output").dataset.autoPath = "";
  $("output").dataset.customPath = "";
  if (!customOutputEnabled()) {
    try {
      const base = await resolveOutputDirForInstance(p);
      if (base) {
        setAutoOutputDir(base);
        appendLog(
          "這個模組整合包的結果位置：\n" +
            base +
            "\n翻譯完成會套用到遊戲資料夾。不同模組整合包請勿共用同一個結果資料夾。"
        );
      }
    } catch (_) {
      /* 略 */
    }
  }
  try {
    syncUiState();
  } catch (_) {
    /* ignore */
  }
  await refreshTranslationHelper();
  // 選完資料夾當下就檢查寫得進去沒有——不要等翻完三小時才在套用階段失敗
  void checkWriteAccessFor(p);
  return probeLocalPackCache(p, { silent: silentProbe });
}

async function onPickInstance() {
  // 翻譯中 D 區停用（規格 S20）：原因已常駐在欄位下方，這裡只是不動作
  if (progressBusy || isAriaDisabled($("btn-inst"))) {
    packActions.syncFolderArea();
    return;
  }
  try {
    const p = await pickDir("選擇遊戲資料夾", readLastInstancePath());
    if (p) {
      hideResumeCard();
      await adoptInstancePath(p);
    }
  } catch (e) {
    log(String(e));
  }
}

/** 主工作台關鍵動作：必須在 syncUiState 之前接線，且每鈕獨立 try。 */
function wireWorkbenchActions() {
  if (document.documentElement.dataset.workbenchActionsWired === "1") return;
  document.documentElement.dataset.workbenchActionsWired = "1";

  const bind = (id, handler) => {
    try {
      const el = $(id);
      if (!el) return;
      markWired(el);
      el.onclick = (ev) => {
        try {
          const out = handler(ev);
          if (out && typeof out.then === "function") out.catch((e) => console.warn("[wire]", id, e));
        } catch (e) {
          console.warn("[wire]", id, e);
        }
      };
    } catch (e) {
      console.warn("[wire-bind]", id, e);
    }
  };

  bind("tab-translate", () => {
    showAppPage("translate");
  });
  bind("tab-font", () => showAppPage("font"));
  bind("btn-inst", () => onPickInstance());
  bind("btn-quit", () => onQuitApp());
  bind("btn-run", () => onRun());
  bind("btn-stop", () => onStop());
  bind("btn-repair", () => onRepair());
  bind("btn-package", () => packageShare());
}

function forceClearBlockingOverlays({ keepConsent = false } = {}) {
  try {
    forceRevealUi();
  } catch (_) {
    /* ignore */
  }
  const ids = [
    "update-overlay",
    "feedback-overlay",
    "issue-overlay",
    "gpt-login-overlay",
    "local-llm-overlay",
  ];
  if (!keepConsent) ids.push("consent-overlay");
  for (const id of ids) {
    try {
      const el = $(id);
      if (!el) continue;
      el.hidden = true;
      el.setAttribute("aria-hidden", "true");
    } catch (_) {
      /* ignore */
    }
  }
  try {
    const root = $("onboard-root");
    if (root) {
      root.hidden = true;
      root.classList.remove("is-active");
      root.setAttribute("aria-hidden", "true");
    }
  } catch (_) {
    /* ignore */
  }
}

function getActivePackMeta() {
  return loadPackMeta(($("instance")?.value || "").trim());
}

function initReloadGuard() {
  window.addEventListener(
    "keydown",
    (ev) => {
      const key = ev.key;
      const mod = ev.ctrlKey || ev.metaKey;
      const isReloadKey = key === "F5" || (mod && (key === "r" || key === "R"));
      if (!isReloadKey) return;
      ev.preventDefault();
      if (progressBusy || shareUploadInFlight) {
        showAppToast("翻譯進行中，無法重新載入。");
      }
    },
    { capture: true }
  );
}

async function beginDiscordLogin() {
  const loginButton = $("btn-discord-login");
  const fallback = $("discord-login-fallback");
  if (loginButton) loginButton.disabled = true;
  if (fallback) fallback.hidden = false;
  if ($("discord-auth-title")) $("discord-auth-title").textContent = "等待 Discord 登入";
  if ($("discord-auth-note")) $("discord-auth-note").textContent = "請在瀏覽器完成授權，再回到工具。";
  if ($("discord-login-fallback-note")) {
    $("discord-login-fallback-note").textContent = "請在瀏覽器完成授權；完成後可按關閉。";
  }
  try {
    const result = await invoke("discord_login");
    if (result && result.ok) {
      if ($("discord-login-fallback-note")) {
        $("discord-login-fallback-note").textContent = "Discord 登入完成。可按關閉。";
      }
      appendLog("Discord 登入完成，正在確認官方伺服器資格。");
    } else {
      const reason = String((result && result.error) || "登入未完成");
      const message = reason === "cancelled"
        ? "已取消 Discord 登入。"
        : reason === "timeout"
          ? "Discord 登入逾時，請重新登入。"
          : "Discord 登入未完成：" + reason;
      appendLog(message, "warn");
    }
  } catch (e) {
    appendError("Discord 登入失敗：" + formatInvokeError(e));
  } finally {
    if (loginButton) loginButton.disabled = false;
    await refreshAiStatus();
  }
}

/*
 * Turnstile 已停用（worker/wrangler.toml TURNSTILE_ENFORCED="0"），HTML 也早就沒有
 * btn-turnstile-* 這三顆按鈕。原本的 beginTurnstileVerification 與其接線是純死碼，
 * 留著只會讓人以為「下載被 Turnstile 擋住」。要復活時從 git 歷史取回。
 */

async function detectVersionForInstance(instancePath, silent) {
  const select = $("target-version");
  const status = $("version-status");
  if (!select || !instancePath) return null;
  try {
    const detected = await invoke("detect_mc_version", { instancePath });
    if (detected && !isSupportedMinecraftVersion(detected)) {
      clearVersionBlock();
      setVersionBlock(unsupportedVersionMessage(detected));
      // 不把非法版本塞進下拉、不自動選上
      if (!select.value || select.dataset.autoDetected === "true") {
        select.value = "";
        select.dataset.autoDetected = "true";
      }
      if (status) status.textContent = versionBlockReason;
      syncUiState();
      return null;
    }
    clearVersionBlock();
    if (detected && !Array.from(select.options).some((option) => option.value === detected)) {
      // 僅允許把已支援版本加入（例如 1.21.x 細部）
      if (isSupportedMinecraftVersion(detected)) {
        select.add(new Option(detected, detected));
      }
    }
    if (detected) {
      if (!select.value || select.dataset.autoDetected === "true") {
        select.value = detected;
        select.dataset.autoDetected = "true";
        if (status) status.textContent = "已自動偵測：Minecraft " + detected;
      } else if (status && !silent) {
        status.textContent = "已手動指定：Minecraft " + select.value;
      }
    } else if (status && !silent) {
      status.textContent = "找不到版本，請從下拉選單指定 1.13 以上";
    }
    syncUiState();
    return detected || null;
  } catch (e) {
    if (status && !silent) status.textContent = "版本偵測失敗，請從下拉選單手動指定 1.13 以上";
    return null;
  }
}

async function refreshInstanceTarget(instancePath) {
  try {
    const target = await invoke("check_install_target", { instancePath });
    if (!target || target.ok === false) return false;
    return true;
  } catch (_) {
    return false;
  }
}

function setInstanceValidateStatus(ok, reason, state) {
  const el = $("instance-validate-status");
  if (!el) return;
  // 沒通過的原因只在狀態卡說（一件事只在一處說）；這裡只留通過後的補充
  el.hidden = !ok;
  el.textContent = reason || "";
  el.dataset.state = state || (ok ? "ok" : reason ? "error" : "idle");
}

async function validateSelectedInstance(path) {
  const instancePath = String(path || "").trim();
  if (!instancePath) {
    instanceValidation = { ok: false, reason: "尚未選擇遊戲資料夾。" };
    clearVersionBlock();
    setInstanceValidateStatus(false, instanceValidation.reason, "idle");
    syncUiState();
    return false;
  }
  try {
    const result = await invoke("validate_instance_cmd", { instancePath });
    const ok = !!(result && result.ok);
    const reason = String((result && result.reason) || "").trim() || (ok ? "遊戲資料夾可用。" : "遊戲資料夾檢查沒過。");
    const hints = Array.isArray(result?.hints) ? result.hints.filter(Boolean) : [];
    instanceValidation = { ok, reason, hints };
    const detail = hints.length ? `${reason} ${hints[0]}` : reason;
    setInstanceValidateStatus(ok, detail, ok ? "ok" : "error");
    syncUiState();
    return ok;
  } catch (e) {
    instanceValidation = { ok: false, reason: formatInvokeError(e) };
    setInstanceValidateStatus(false, instanceValidation.reason, "error");
    syncUiState();
    return false;
  }
}

/**
 * 手動貼上／輸入遊戲資料夾路徑（跟「瀏覽…」挑資料夾是兩條不同的輸入路徑）。
 *
 * 只做驗證還不夠：舊版這裡只呼叫 validateSelectedInstance，於是（1）路徑沒被記住，
 * 關掉工具再打開又要重新輸入一次；（2）「本機已有翻譯」卡片仍停在上一個資料夾的
 * 探測結果，跟畫面上新輸入的路徑對不上。
 *
 * 不完整比照 onPickInstance()：那邊會整段重置 output 欄位、重新偵測版本／整合包名稱，
 * 這裡若原封不動搬過來，使用者打字打到一半（每次停頓 400ms 就觸發一次）會被反覆
 * 清空自訂輸出路徑，體感是「打字打一半設定被吃掉」。這裡只做驗證通過後最小必要的
 * 兩件事：記住路徑、重新對齊快取探測。
 */
async function onInstanceTypedPath(path) {
  const ok = await validateSelectedInstance(path);
  if (!ok) return;
  writeLastInstancePath(path);
  if (!customOutputEnabled()) {
    try {
      const base = await resolveOutputDirForInstance(path);
      if (base) setAutoOutputDir(base);
    } catch (_) {
      /* 略：輸出路徑之後仍可從「本包選項」手動調整 */
    }
  }
  await probeLocalPackCache(path, { silent: true }).catch(() => null);
}

async function refreshPackTranslationName(instancePath) {
  try {
    const info = await invoke("detect_pack_translation_name", { instancePath });
    const name = info && (info.packName || info.pack_name);
    const el = $("pack-name");
    if (el && name && el.dataset.auto !== "0") {
      el.value = name;
      el.dataset.auto = "1";
    }
    if ($("pack-version-status")) {
      const version = info && info.version ? info.version : "R1";
      const source = info && info.source ? info.source : "未找到版本檔，使用複查編號";
      $("pack-version-status").textContent = `資源包版本：${version}（${source}）。可翻譯前自訂名稱；留空則系統產生。`;
    }
  } catch (_) {
    if ($("pack-version-status")) {
      $("pack-version-status").textContent =
        "模組整合包版本尚未偵測，完成翻譯時會使用 R1。可翻譯前自訂名稱；留空則系統產生。";
    }
  }
}

/** 送後端的資源包名：仍為自動建議／空白／占位 → 空字串（由系統命名） */
function packNameForTranslate() {
  const el = $("pack-name");
  if (!el) return "";
  if (el.dataset.auto !== "0") return "";
  const raw = (el.value || "").trim();
  if (!raw || raw === "選擇實例後自動命名" || raw === "留空則自動命名") {
    return "";
  }
  return raw;
}

async function refreshReferencePack() {
  const status = $("reference-status");
  const input = $("reference-pack");
  if (input && (input.value || "").trim()) {
    if (status) status.textContent = "已指定參考翻譯；翻譯時會優先填缺。";
    return input.value;
  }
  if (status) {
    status.textContent = "尚未指定參考翻譯；可手動選本機的繁體中文翻譯或社群漢化資料夾、zip，或略過。";
  }
  return "";
}

function normalizeApiKeyDraft(raw) {
  const key = String(raw || "").trim();
  // 畫面遮罩或誤貼 # 不得當成真金鑰送出
  if (!key || /^#+$/.test(key)) return "";
  return key;
}

async function onSaveAdv() {
  try {
    const keyToSave = normalizeApiKeyDraft(apiKeyDraft);
    await invoke("save_api_settings_cmd", {
      apiKey: keyToSave,
      baseUrl: ($("base-url").value || "").trim(),
      provider: ($("api-provider").value || "deepseek").trim(),
      model: ($("api-model").value || "").trim(),
    });
    $("api-key").value = ""; // 輸入框清空，畫面上不留金鑰
    apiKeyDraft = "";
    await refreshApiSettings();
    await refreshAiStatus();
    const settings = await invoke("get_api_settings").catch(() => null);
    const hasKey = !!(settings && (settings.hasKey || settings.has_key));
    log(
      hasKey
        ? "設定已儲存。已存金鑰會用於翻譯與測試；輸入框的 ######## 只是遮罩。"
        : "設定已儲存。"
    );
    if (hasKey) {
      await testCustomApiKey({ quietLog: false });
    }
  } catch (e) {
    log("儲存失敗：\n" + String(e));
  }
}

async function testCustomApiKey(opts) {
  const quietLog = !!(opts && opts.quietLog);
  const statusEl = $("api-test-status");
  const btn = $("btn-test-api");
  if (btn) btn.disabled = true;
  if (statusEl) statusEl.textContent = "正在用本機已存金鑰測試連線…";
  try {
    const msg = await invoke("test_custom_api_key_cmd");
    const ok = String(msg || "金鑰有效，可連線到你的 API。");
    if (statusEl) statusEl.textContent = ok;
    if (!quietLog) log(ok);
    return true;
  } catch (e) {
    const err = formatInvokeError(e);
    if (statusEl) statusEl.textContent = "測試失敗：" + err;
    if (!quietLog) appendError("金鑰測試失敗：" + err);
    return false;
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function onTestApiKey() {
  const draft = normalizeApiKeyDraft(apiKeyDraft);
  if (draft) {
    await onSaveAdv();
    return;
  }
  await testCustomApiKey({ quietLog: false });
}

/**
 * 套用前的遊戲執行中檢查。
 *
 * 套用會直接寫進遊戲實例資料夾；Minecraft 開著時檔案被鎖，結果是半套用——玩家看到殘缺
 * 翻譯或閃退，然後把帳算在翻譯頭上。舊版只在失敗「之後」才說「請先關閉 Minecraft」。
 *
 * 失效方向刻意設成放行：後端偵測不出來（非 Windows、權限不足、查詢失敗）時 known=false，
 * 這裡直接回 true。這個檢查只能擋「確定在跑」，不能變成新的卡關來源。
 */
async function ensureGameClosed(instancePath, actionLabel) {
  let verdict = null;
  try {
    verdict = await invoke("is_game_running_cmd", { instancePath });
  } catch (_) {
    return true; // 查不到就放行
  }
  if (!verdict || !verdict.running) return true;
  const retry = await confirmDialog({
    title: "遊戲好像還開著",
    body:
      `偵測到這個整合包的 Minecraft 正在執行。現在${actionLabel}，檔案會被鎖住，` +
      "可能只套用一半，遊戲裡會出現殘缺翻譯甚至閃退。\n\n請先完全關閉遊戲與啟動器，再按「我已關閉，重新檢查」。",
    affected: [instancePath],
    danger: true,
    confirmLabel: "我已關閉，重新檢查",
    cancelLabel: "先不要繼續",
  });
  if (!retry) {
    appendLog(`已取消${actionLabel}：請先關閉 Minecraft 再試一次。`, "warn");
    return false;
  }
  return ensureGameClosed(instancePath, actionLabel);
}

/**
 * 開始翻譯：同一件事正在跑時，重複點擊一律忽略。
 *
 * 守衛放在函式定義處而不是接線處，因為這些動作有多個呼叫端
 * （直接接線、文件委派保底、快取卡的捷徑按鈕），只守其中一條會漏。
 */
async function onRun() {
  // 狀態卡的主要按鈕停用時用 aria-disabled（仍可聚焦、讀得到原因），點了不動作
  if (isAriaDisabled($("btn-run"))) return;
  packActions.clearRemoval();
  return runExclusive("run", onRunInner, {
    onBusy: () => appendLog("「開始翻譯」已經在執行中，請稍候。", "warn"),
  });
}

async function onRunInner() {
  const instancePath = ($("instance").value || "").trim();
  let outputDir = selectedOutputDir();
  if (!instancePath) return log("請先選擇「遊戲資料夾」。");
  if (!(await validateSelectedInstance(instancePath))) {
    return log(instanceValidation.reason || "實例檢查未通過，無法開始翻譯。");
  }
  if (!outputDir || isLegacySharedWorkPath(outputDir)) {
    outputDir = (await resolveOutputDirForInstance(instancePath)) || "";
    if (outputDir) {
      setAutoOutputDir(outputDir);
      appendLog("這個模組整合包的結果位置：\n" + outputDir);
    }
  }
  if (!outputDir) return log("翻譯結果位置還沒準備好，請重新選擇遊戲資料夾。");

  // 覆蓋提醒不能只看 localCacheProbe：它只在「選資料夾」或「啟動還原」時才填，
  // 剛跑完一次翻譯、或探測失敗時是 null，於是最該提醒的情況反而不提醒。
  // 這裡在按下開始翻譯的當下補探一次，並把 hasShareableFiles 也納入判斷。
  if (!localCacheProbe && (hasShareableFiles || outputDir)) {
    await probeLocalPackCache(instancePath, { silent: true }).catch(() => null);
  }
  const hasExistingResult =
    (localCacheProbe && (localCacheProbe.status === "ready" || localCacheProbe.shareable)) ||
    hasShareableFiles;
  if (hasExistingResult) {
    // 舊版只有「仍要重新翻譯／取消」兩個選項，使用者既看不出「重新翻譯」會發生
    // 什麼事，也沒有「這次另外存一份」的路。改成講清楚每個選項的後果。
    const existingPath =
      localCacheProbe?.workRoot || localCacheProbe?.outputDir || resultWorkDir(outputDir);
    const choice = await choiceDialog({
      title: "這個整合包已經翻譯過了，這次想怎麼做？",
      body: "既有的翻譯結果在：\n" + (existingPath || "（位置未知）"),
      options: [
        {
          value: "supplement",
          label: "接續補完（建議）",
          detail: "沿用既有結果，只補之前沒翻到的句子。最快，也不會動到已經翻好的內容。",
        },
        {
          value: "overwrite",
          label: "覆蓋重翻",
          detail: "刪掉舊結果、整包重新翻一次。譯文品質不滿意想重來時選這個。",
        },
        {
          value: "newcopy",
          label: "另存一份新的",
          detail: "舊的完整保留，這次的結果存到新資料夾。想比較兩次結果或換了 AI 來源時選這個。",
        },
      ],
      cancelLabel: "取消",
    });
    if (!choice) {
      return appendLog("已取消；可使用上方「本機已有翻譯」直接打開、套用或分享。", "warn");
    }
    if (choice === "supplement") {
      appendLog("改用「接續補完」：沿用既有結果，只補沒翻到的句子。");
      return onSupplementInner();
    }
    if (choice === "newcopy") {
      const nextDir = await invoke("next_result_dir_cmd", { outputDir }).catch(() => "");
      if (nextDir) {
        outputDir = nextDir;
        setAutoOutputDir(nextDir);
        appendLog("這次的結果會另存到新位置：\n" + nextDir);
      } else {
        appendLog("無法建立新的結果資料夾，改為覆蓋既有結果。", "warn");
      }
    }
  }

  // 遊戲開著也照常翻譯：只有最後「裝進遊戲」那一步需要關遊戲，
  // 後端會回「已翻完、還沒裝進遊戲」，到時再按「套用到遊戲」即可。
  applyPending.hideCard();

  writeLastInstancePath(instancePath);
  let useAi = !!$("use-ai").checked;
  if (useAi && !(await ensureAiReadyForAction())) {
    // 使用者明明開了 AI，卻被問「要不要不用 AI 跑一次」是多餘的岔路——
    // 這裡只講「為什麼現在不能用」與「怎麼解決」，把不用 AI 降級成次要選項。
    const aiMode = aiModeFromUi();
    const why =
      aiMode === "local"
        ? "本地模型現在啟動不起來。常見原因是模型資料夾被移動或刪除，或這台電腦的記憶體不足。"
        : aiMode === "gpt"
          ? "GPT 登入尚未完成或已過期。"
          : "自訂 API 金鑰尚未通過驗證。";
    const fallback = await confirmDialog({
      title: "AI 現在無法使用",
      body:
        why +
        "\n\n上方的 AI 區塊可以重新設定。如果你想先看看翻譯效果，也可以不使用 AI 跑一次——" +
        "術語表、翻譯記憶與簡繁轉換都不需要 AI，之後再按「補充漏翻」把剩下的補上。",
      confirmLabel: "先不使用 AI 跑一次",
      cancelLabel: "回去設定 AI",
    });
    if (!fallback) return;
    useAi = false;
    appendLog("這一輪不使用 AI：只用術語表、翻譯記憶與簡繁轉換。之後可用「補充漏翻」再補。", "warn");
  }
  let targetVersion = ($("target-version")?.value || "").trim();
  if (targetVersion && !isSupportedMinecraftVersion(targetVersion)) {
    setVersionBlock(unsupportedVersionMessage(targetVersion));
    syncUiState();
    return log(versionBlockReason);
  }
  if (!targetVersion) {
    targetVersion = (await detectVersionForInstance(instancePath, true)) || "";
  }
  if (versionBlocked) {
    return log(versionBlockReason || unsupportedVersionMessage("未知"));
  }
  if (!targetVersion) {
    return log("無法確認 Minecraft 版本。請從版本選單指定 1.13 以上（或年份版 26.x）後再翻譯。");
  }
  if (!isSupportedMinecraftVersion(targetVersion)) {
    return log(unsupportedVersionMessage(targetVersion));
  }

  // 要不要留下「翻譯結果」改由設定「翻完刪除翻譯結果」決定（唯一位置在設定→資料與備份，
  // 第一次勾要同列確認；B5a-2）。開始前不再跳三選一。只管結果資料夾，備份照 backupChoice（G1.8）。
  const skipResultFolder = deleteResultsAfterApplyEnabled();

  setBusy(true, "translate");
  lastStepIdx = -1;
  setTranslationState("running");
  lastProgressLogKey = "";
  currentRunStamp = newRunStamp();
  clearLog("開始翻譯");
  resetCoverageMetrics("翻譯統計蒐集中");
  // 只有「開始翻譯」（全新一輪）才清空步驟計時；修復／補充漏翻是接續同一輪，時間要繼續累加。
  resetStepTimings();
  if ($("btn-package")) $("btn-package").disabled = true;
  // 「不保留」只管翻譯結果資料夾；備份一律照設定（後端讀 translate.backupChoice）
  const backupNote = {
    always: "套用到遊戲前會先備份遊戲原本的檔案。",
    never: "依你的設定不備份；覆蓋遊戲原本的檔案前會再問你。",
  }[currentBackupChoice()] || "第一次套用到遊戲前會問你要不要備份。";
  appendLog(
    (skipResultFolder
      ? "翻譯完成後會套用到遊戲，套用後不保留這次的翻譯結果。"
      : "翻譯完成後會套用到遊戲，翻譯結果也會留著。") + backupNote
  );
  setProgress(1, "準備中…");
  void hideUiForTranslateRun();
  await paintBeforeInvoke();

  let applyFollowUp = null;
  const modelRound = await localModelRounds.begin();
  try {
    const result = await invoke("one_click_translate", {
      instancePath,
      outputDir,
      packName: packNameForTranslate(),
      useAi,
      keepResults: !skipResultFolder,
      referencePack: (($('reference-pack')?.value || "").trim() || null),
      targetVersion: targetVersion || null,
      coverageTier: "max",
    });
    const pendingCount = Number(result.pendingCount ?? result.pending_count ?? 0) || 0;
    const coveragePercent = Number(result.coveragePercent ?? result.coverage_percent);
    const completedWithPending = result.completedWithPending ?? result.completed_with_pending ?? pendingCount > 0;
    // 「本來就不該翻、已原樣保留」的數量：跟待補分開顯示，避免使用者誤會成漏翻
    const staysUnchangedCount =
      Number(result.staysUnchanged ?? result.stays_unchanged ?? 0) || 0;
    if (staysUnchangedCount > 0) {
      coverageMetrics.staysUnchanged = Math.max(
        coverageMetrics.staysUnchanged || 0,
        staysUnchangedCount
      );
    }
    setProgress(
      100,
      completedWithPending
        ? `本輪流程完成，仍有 ${formatCount(pendingCount)} 條待補`
        : "翻譯流程完成",
      {
        payload: {
          state: completedWithPending ? "completed_with_pending" : "completed",
          detail: Number.isFinite(coveragePercent) ? `可玩文字覆蓋率 ${coveragePercent}%` : "",
          metrics: {
            packPending: pendingCount,
            ...(Number.isFinite(coveragePercent) ? { coveragePercent } : {}),
          },
        },
      }
    );
    let msg = result.playerSummary || result.player_summary || "翻譯完成，請看日誌。";
    const notYetApplied = isApplyPending(result);
    if (notYetApplied) {
      applyFollowUp = { result, ctx: { instancePath, outputDir, packName: packNameForTranslate() || null } };
    }
    // 結論先行：完成訊息本身偏技術，先給一句人話，讓使用者知道「現在就能玩」，
    // 不用讀完整份報告才敢開遊戲。少數原文保留是正常的，一併先講清楚。
    const headline = notYetApplied
      ? "翻譯已完成，但還沒裝進遊戲（原因與下一步見下方）。"
      : staysUnchangedCount > 0
      ? "可以直接開遊戲了，主要遊戲文字都已是繁體中文。\n少數專有名詞、單位符號與附魔等級維持原文是正常的，翻了反而會出錯。"
      : "可以直接開遊戲了，主要遊戲文字都已是繁體中文。";
    msg = headline + "\n\n" + msg;
    if (result.minemenuMsg || result.minemenu_msg) {
      msg += "\n\n" + (result.minemenuMsg || result.minemenu_msg);
    }
    const siblingWarning = siblingInstanceWarning(result);
    if (siblingWarning) {
      msg += "\n\n【請確認】" + siblingWarning;
      appendLog(siblingWarning, "warn");
    }
    // 套用完檢查資源包清單有沒有被弄壞。使用者實測遇過清單被清空，導致字體
    // 找不到材質 → 資源重載失敗 → 模型沒烘焙 → 標題畫面直接閃退。
    await checkResourcePackHealth(instancePath);
    consumeCoverageMessage(msg);
    setLogFinal(msg);
    setTranslationState("complete");
    if (notYetApplied) {
      // 還沒裝進遊戲：翻譯結果是之後「套用到遊戲」的來源，不能照「不保留」刪掉
      appendLog("翻譯已完成，還沒裝進遊戲；翻譯結果先保留，等你按「套用到遊戲」。", "warn");
    } else if (skipResultFolder) {
      appendLog("已依你的選擇不保留翻譯結果，正在清理暫存資料夾…");
      try {
        await invoke("delete_result_folder_cmd", { outputDir });
        appendLog("翻譯已完成並裝進遊戲；依你的選擇沒有保留翻譯結果（備份照你的設定處理）。");
      } catch (cleanupErr) {
        appendLog(
          "翻譯已完成並直接套用，但清理翻譯結果資料夾時發生問題：" +
            (cleanupErr?.message || cleanupErr),
          "warn"
        );
      }
    } else {
      appendLog("翻譯已完成並直接套用。想分享給其他玩家時，再按「分享給其他玩家」。");
    }
    await cleanupPreparedTranslationHelper();
  } catch (e) {
    if (isCancellation(e)) {
      setTranslationState("idle");
      appendLog("已停止。先前完成的部分仍保留；有效譯文會盡量上傳共享庫（已上傳過的不會重複灌庫）。", "warn");
    } else if (isDiscordGateError(e)) {
      setTranslationState("idle");
      setProgress(Math.max(lastRealPercent, Math.floor(displayPercent) || 0), "請先完成 Discord 驗證");
      handleDiscordGateError(e);
      return;
    } else {
      setTranslationState("failed");
    }
    handleRunFailure(e, "翻譯失敗");
    if (!isCancellation(e)) {
      appendLog("可把上方錯誤訊息留下來方便排查。");
    }
  } finally {
    setBusy(false);
    refreshBackupState();
    void releaseLocalModelAfterRun(modelRound);
  }
  if (applyFollowUp) await applyPending.handle(applyFollowUp.result, applyFollowUp.ctx);
}

/** 舊版共用 work／work\\翻譯結果 → 應改走 per-instance */
function isLegacySharedWorkPath(path) {
  const n = String(path || "").replace(/[\\/]+$/, "").toLowerCase();
  if (!n) return false;
  const marker = "modpack-i18n-tool\\work";
  const marker2 = "modpack-i18n-tool/work";
  const idx = Math.max(n.lastIndexOf(marker), n.lastIndexOf(marker2));
  if (idx < 0) return false;
  const rest = n.slice(idx).replace(/\//g, "\\");
  return (
    rest === "modpack-i18n-tool\\work" ||
    rest === "modpack-i18n-tool\\work\\翻譯結果" ||
    /modpack-i18n-tool\\work\\翻譯結果$/i.test(rest)
  );
}

/**
 * 補翻模式。
 *
 * 「重新翻譯缺漏」勾選框已移除——使用者無從判斷該不該勾，而它的語意
 * （連品質暫緩的也重送）現在由「開始翻譯」時的「覆蓋重翻」選項涵蓋。
 * 這裡固定回 null＝一般補翻，不強制重送。
 */
function supplementTranslationMode() {
  return null;
}

/** 修復：重建 zip／對齊工作階段；可選一併 AI 補缺。不修世界閃退。 */
/**
 * 修復工作階段：同一件事正在跑時，重複點擊一律忽略。
 *
 * 守衛放在函式定義處而不是接線處，因為這些動作有多個呼叫端
 * （直接接線、文件委派保底、快取卡的捷徑按鈕），只守其中一條會漏。
 */
async function onRepair() {
  return runExclusive("repair", onRepairInner, {
    onBusy: () => appendLog("「修復工作階段」已經在執行中，請稍候。", "warn"),
  });
}

async function onRepairInner() {
  const outputDir = selectedOutputDir();
  if (!outputDir) {
    return log("請先選好與上次相同的「翻譯結果」位置。");
  }
  try {
    const st = await invoke("session_status", { outputDir });
    if (!(st.ok || st.OK)) {
      return log(
        "找不到上次的翻譯紀錄。\n請確認結果位置與上次相同，或先按一次「開始翻譯」。"
      );
    }
  } catch (e) {
    /* 繼續交給後端 */
  }

  const useAi = !!$("use-ai").checked;
  if (useAi && !(await ensureAiReadyForAction())) return;

  setBusy(true, "translate");
  lastStepIdx = -1;
  setTranslationState("running");
  lastProgressLogKey = "";
  clearLog("開始修復");
  appendLog("這不能修好「進世界閃退」。");
  setProgress(2, "準備修復…");
  void hideUiForTranslateRun();
  await paintBeforeInvoke();

  let applyFollowUp = null;
  const modelRound = await localModelRounds.begin();
  try {
    const result = await invoke("repair_translation_pack", {
      outputDir,
      useAi,
      translationMode: supplementTranslationMode(),
    });
    setProgress(100, "修復完成！");
    {
      let msg = result.playerSummary || result.player_summary || "翻譯完成，請看日誌。";
      const siblingWarning = siblingInstanceWarning(result);
      if (siblingWarning) {
        msg += "\n\n【請確認】" + siblingWarning;
        appendLog(siblingWarning, "warn");
      }
      setLogFinal(msg);
    }
    setTranslationState("complete");
    if (isApplyPending(result)) applyFollowUp = { result, ctx: applyContextFromUi(outputDir) };
  } catch (e) {
    if (isCancellation(e)) {
      setTranslationState("idle");
      appendLog("已停止。先前完成的部分仍保留；有效譯文會盡量上傳共享庫（已上傳過的不會重複灌庫）。", "warn");
    } else if (isDiscordGateError(e)) {
      setTranslationState("idle");
      setProgress(Math.max(lastRealPercent, Math.floor(displayPercent) || 0), "請先完成 Discord 驗證");
      handleDiscordGateError(e);
      return;
    } else {
      setTranslationState("failed");
    }
    handleRunFailure(e, "修復失敗");
  } finally {
    setBusy(false);
    refreshBackupState();
    void releaseLocalModelAfterRun(modelRound);
  }
  if (applyFollowUp) await applyPending.handle(applyFollowUp.result, applyFollowUp.ctx);
}

/** 只補缺漏：不重掃 mods，讀工作階段 + AI */
/**
 * 補充漏翻：同一件事正在跑時，重複點擊一律忽略。
 *
 * 守衛放在函式定義處而不是接線處，因為這些動作有多個呼叫端
 * （直接接線、文件委派保底、快取卡的捷徑按鈕），只守其中一條會漏。
 */
async function onSupplement() {
  return runExclusive("supplement", onSupplementInner, {
    onBusy: () => appendLog("「補充漏翻」已經在執行中，請稍候。", "warn"),
  });
}

async function onSupplementInner() {
  const outputDir = selectedOutputDir();
  if (!outputDir) {
    return log("請選與上次相同的「翻譯結果」位置。");
  }
  const useAi = !!$("use-ai")?.checked;
  if (useAi && !(await ensureAiReadyForAction())) return;
  try {
    const st = await invoke("session_status", { outputDir });
    if (!(st.ok || st.OK)) {
      return log(
        "找不到上次的翻譯紀錄。\n請確認結果位置與上次相同，或先按「開始翻譯」。"
      );
    }
  } catch (e) {
    let has = false;
    try {
      has = await invoke("has_session", { outputDir });
    } catch (_) {
      has = false;
    }
    if (!has) {
      return log("找不到上次的翻譯紀錄。請確認結果位置，或先完整翻一次。");
    }
  }

  setBusy(true, "translate");
  lastStepIdx = -1;
  setTranslationState("running");
  lastProgressLogKey = "";
  clearLog("開始再補一些");
  // 補充漏翻是接續同一個整合包的翻譯效果，不是另開一輪新翻譯——進階統計要接著累加，
  // 不能讓「翻譯」階段辛苦累出來的數字被「補充」階段的新引擎歸零蓋掉。
  resetCoverageMetrics("補翻統計蒐集中", { carryForward: true });
  setProgress(3, "準備中…");
  void hideUiForTranslateRun();
  await paintBeforeInvoke();

  let applyFollowUp = null;
  const modelRound = await localModelRounds.begin();
  try {
    const result = await invoke("supplement_translate", {
      outputDir,
      useAi,
      translationMode: supplementTranslationMode(),
    });
    const pendingCount = Number(result.pendingCount ?? result.pending_count ?? 0) || 0;
    const completedWithPending = result.completedWithPending ?? result.completed_with_pending ?? pendingCount > 0;
    setProgress(
      100,
      completedWithPending
        ? `補譯完成，仍有 ${formatCount(pendingCount)} 條待補；品質暫緩項目不會重送`
        : "補譯完成！",
      {
        payload: {
          state: completedWithPending ? "completed_with_pending" : "completed",
          metrics: { packPending: pendingCount },
        },
      }
    );
    let msg = result.playerSummary || result.player_summary || "翻譯完成，請看日誌。";
    const siblingWarning = siblingInstanceWarning(result);
    if (siblingWarning) {
      msg += "\n\n【請確認】" + siblingWarning;
      appendLog(siblingWarning, "warn");
    }
    consumeCoverageMessage(msg);
    setLogFinal(msg);
    setTranslationState("complete");
    if (isApplyPending(result)) {
      applyFollowUp = { result, ctx: applyContextFromUi(outputDir) };
    } else {
      appendLog("複查完成，結果已重新套用到遊戲。", "info");
    }
    await cleanupPreparedTranslationHelper();
  } catch (e) {
    if (isCancellation(e)) {
      setTranslationState("idle");
      appendLog("已停止。先前完成的部分仍保留；有效譯文會盡量上傳共享庫（已上傳過的不會重複灌庫）。", "warn");
    } else if (isDiscordGateError(e)) {
      setTranslationState("idle");
      setProgress(Math.max(lastRealPercent, Math.floor(displayPercent) || 0), "請先完成 Discord 驗證");
      handleDiscordGateError(e);
      return;
    } else {
      setTranslationState("failed");
    }
    handleRunFailure(e, "再補一些失敗");
  } finally {
    setBusy(false);
    refreshBackupState();
    void releaseLocalModelAfterRun(modelRound);
  }
  if (applyFollowUp) await applyPending.handle(applyFollowUp.result, applyFollowUp.ctx);
}

/** 停止：後端在下一個檢查點乾淨收尾，已完成的檔案保留 */
async function onStop() {
  if (stopRequestInFlight) {
    showAppToast("停止要求已送出，正在等待目前步驟收尾。", 1800);
    return;
  }
  stopRequestInFlight = true;
  const btn = $("btn-stop");
  if (btn) {
    btn.disabled = true;
    btn.textContent = STOP_LABELS.sending;
  }
  setProgressStateBadge("cancelling");
  appendLog(
    "已要求停止，等目前這一步做完就會收尾；有效譯文會上傳共享庫，已上傳過的不會重複灌庫。",
    "warn"
  );
  try {
    await invoke("cancel_task");
    if (btn) {
      btn.textContent = STOP_LABELS.sent;
      window.setTimeout(() => {
        if (!progressBusy || !$("btn-stop")) return;
        $("btn-stop").disabled = false;
        // 審查 2：第一次停止後後端正在寫出並套用已翻部分；再按會放棄寫出與套用
        $("btn-stop").textContent = STOP_LABELS.afterFirstStop;
        $("btn-stop").title = STOP_LABELS.afterFirstStopTitle;
      }, 900);
    }
  } catch (e) {
    appendError("無法送出停止要求：" + formatInvokeError(e));
    resetStopButton(btn);
  } finally {
    stopRequestInFlight = false;
  }
}

/** 打開自訂術語表，讓玩家改成自己喜歡的譯名 */
async function onOpenGlossary() {
  try {
    const path = await invoke("open_glossary");
    appendLog("已開啟自訂譯名檔：\n" + path);
    appendLog("格式：{\"英文原文\": \"你要的中文\"}；存檔後重新翻譯即生效。");
  } catch (e) {
    appendError("無法開啟自訂譯名檔：" + formatInvokeError(e));
  }
}

async function refreshConsistencyMergeUi() {
  const btn = $("btn-merge-consistency");
  const hint = $("consistency-merge-hint");
  const outputDir = selectedOutputDir();
  if (!btn) return;
  if (!outputDir || translationState !== "complete") {
    btn.hidden = true;
    if (hint) hint.hidden = true;
    return;
  }
  const work = resultWorkDir(outputDir);
  try {
    const status = await invoke("consistency_suggestions_status_cmd", { workRoot: work });
    const exists = !!(status && (status.exists || status.exists === true));
    const count = Number(status?.count || 0);
    btn.hidden = !exists;
    if (hint) {
      hint.hidden = !exists;
      if (exists) {
        hint.textContent = `結果資料夾有用詞不一致建議（約 ${count} 條）。按「併入用詞建議」寫進術語表；預設不覆蓋你已有的譯名。`;
      }
    }
  } catch (_) {
    btn.hidden = true;
    if (hint) hint.hidden = true;
  }
}

async function onMergeConsistencySuggestions() {
  const outputDir = selectedOutputDir();
  if (!outputDir) {
    showAppToast("請先完成翻譯並有結果資料夾。", 2600);
    return;
  }
  const work = resultWorkDir(outputDir);
  try {
    const result = await invoke("merge_consistency_suggestions_cmd", {
      workRoot: work,
      work_root: work,
      overwrite: false,
    });
    const message = String(result?.message || "已併入建議譯名。");
    appendLog(message);
    if (result?.glossaryPath) appendLog("術語表：" + result.glossaryPath);
    showAppToast(message, 3600);
    await refreshConsistencyMergeUi();
  } catch (e) {
    const msg = formatInvokeError(e);
    appendError("併入用詞建議失敗：" + msg);
    showAppToast(msg, 4200);
  }
}

async function loadUiPrefs() {
  try {
    const p = await invoke("get_ui_prefs");
    const min =
      p.minimizeOnClose != null
        ? !!p.minimizeOnClose
        : p.minimize_on_close != null
          ? !!p.minimize_on_close
          : true;
    void min; // 關閉行為的開關在設定視窗；這裡只需要版本號
    applyAppVersion(String(p.appVersion || p.app_version || "").trim());
  } catch (e) {
    /* 預設已勾選 */
  }
}

/**
 * 版本號唯一真相源＝Cargo.toml，由 get_ui_prefs 帶回來。
 *
 * 舊版在 HTML 硬編碼四處，再用一段 regex 在開啟說明時事後修補——改版必漏一處。
 */
function applyAppVersion(version) {
  if (!version) return;
  document.querySelectorAll("[data-app-version]").forEach((el) => {
    el.textContent = version;
  });
}

const COVERAGE_ACK_STORAGE_KEY = "modpack-i18n-coverage-ack-hard";

function wireCoverageTiers() {
  /* 0.2.2：完整度三選已移除；固定 max，此函式保留空殼以免舊呼叫炸掉。 */
}

const FONT_PRESETS = {
  clear: { size: 11, weight: 400, shiftX: 0, shiftY: 0.5, oversample: 5 },
  compact: { size: 9.5, weight: 400, shiftX: 0, shiftY: 0.3, oversample: 4 },
  large: { size: 14, weight: 450, shiftX: 0, shiftY: 0.6, oversample: 4 },
};

function readFontPrefs() {
  try {
    const raw = localStorage.getItem(FONT_PREFS_STORAGE_KEY);
    if (!raw) return null;
    return JSON.parse(raw);
  } catch (_) {
    return null;
  }
}

function writeFontPrefs() {
  try {
    localStorage.setItem(
      FONT_PREFS_STORAGE_KEY,
      JSON.stringify({
        size: Number($("font-size")?.value || 11),
        weight: Number($("font-thickness")?.value || 400),
        shiftX: Number($("font-shift-x")?.value || 0),
        shiftY: Number($("font-shift-y")?.value || 0.5),
        oversample: Number($("font-oversample")?.value || 4),
        packName: ($("font-pack-name")?.value || "").trim(),
      })
    );
  } catch (_) {
    /* ignore */
  }
}

function applyFontPrefs(prefs) {
  if (!prefs) return;
  const map = [
    ["font-size", "size", "font-size-value"],
    ["font-thickness", "weight", "font-thickness-value"],
    ["font-shift-x", "shiftX", "font-shift-x-value"],
    ["font-shift-y", "shiftY", "font-shift-y-value"],
    ["font-oversample", "oversample", "font-oversample-value"],
  ];
  map.forEach(([id, key, outId]) => {
    if (prefs[key] == null || !$(id)) return;
    $(id).value = String(prefs[key]);
    if ($(outId)) $(outId).textContent = String(prefs[key]);
  });
  if (prefs.packName && $("font-pack-name")) $("font-pack-name").value = prefs.packName;
}

function applyFontPreviewStyles() {
  const sample = $("font-preview-sample");
  if (!sample) return;
  const size = Number($("font-size")?.value || 11);
  const shiftX = Number($("font-shift-x")?.value || 0);
  const shiftY = Number($("font-shift-y")?.value || 0.5);
  sample.style.fontSize = `${Math.max(14, size * 1.6)}px`;
  sample.style.transform = `translate(${shiftX}px, ${shiftY}px)`;
}

async function updateFontPreview(path) {
  const panel = $("font-preview");
  const sample = $("font-preview-sample");
  const missing = $("font-preview-missing");
  if (!panel || !sample || !path) return;
  // 先把上一個字體的臉拿掉再載新的。不先清的話，新字體讀取失敗時預覽會繼續用
  // 前一個字體的字形顯示，旁邊卻標著新檔名——使用者會以為新字體沒問題。
  if (fontPreviewFace) {
    try {
      document.fonts.delete(fontPreviewFace);
    } catch (_) {
      /* ignore */
    }
    fontPreviewFace = null;
  }
  sample.style.fontFamily = "";
  if (missing) missing.hidden = true;
  try {
    panel.hidden = false;
    applyFontPreviewStyles();
    const b64 = await invoke("read_font_file_base64", { fontPath: path });
    const ext = String(path).toLowerCase().endsWith(".otf") ? "opentype" : "truetype";
    fontPreviewFace = new FontFace("mcpl-font-preview", `url(data:font/${ext};base64,${b64})`);
    await fontPreviewFace.load();
    document.fonts.add(fontPreviewFace);
    sample.style.fontFamily = "mcpl-font-preview, sans-serif";
    if (missing) {
      const lacksGlyph = typeof document.fonts.check === "function"
        ? !document.fonts.check('16px "mcpl-font-preview"', "繁")
        : false;
      missing.hidden = !lacksGlyph;
    }
  } catch (e) {
    if (missing) missing.hidden = false;
    appendFontLog("字體預覽失敗：" + String(e), "warn");
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  // 0.2.4：先露出 UI、再綁全部按鈕；任何 await／錯誤都不可擋住接線
  revealInitialContent();
  wireHelpTips();
  // 分頁方向鍵（規格 §6）：左右切換「翻譯」「字體工具」，只有目前分頁在 Tab 順序裡
  syncWorkbenchTabKeys = wireTabKeys($("workbench-tabs"));
  // 設定改存實體檔案（跟著工具走），不再只依賴 WebView2 的 localStorage 快取。
  // 必須在 initTheme／loadBackupPreference 之前完成，否則那些函式會讀到還沒
  // 從檔案同步回來的舊值。失敗會靜默降級成純 localStorage，不擋啟動。
  await loadSettings().catch(() => null);
  // 後端在「翻譯中被要求關閉」時發這個訊號，由前端問使用者並先落檔
  try {
    listen("close-requested-while-busy", () => {
      void handleCloseWhileBusy();
    });
  } catch (_) {
    /* 監聽註冊失敗不影響其他功能 */
  }
  const startupRevealFallback = window.setTimeout(() => {
    forceRevealUi();
    revealInitialContent();
  }, 2000);

  let apiSettingsTask = Promise.resolve();
  try {
    initTheme();
    // 主視窗用快捷鍵或自動縮放改了大小：寫進設定檔，並告訴設定視窗更新顯示
    onScalePersisted((percent, auto) => {
      setSetting("mcpl-webview-scale", String(percent));
      setSetting("mcpl-webview-autoscale", auto ? "1" : "0");
      void Promise.resolve(
        emit(SETTINGS_UPDATED_EVENT, {
          path: "appearance.uiScale",
          value: String(percent),
          autoScale: auto,
          source: "main",
        })
      ).catch(() => {});
    });
    initWebviewScale();
    loadBackupPreference();
    syncAiPanel(false);
    apiSettingsTask = refreshApiSettings().catch((e) => {
      try {
        appendLog("啟動時讀取 API 設定失敗：" + String(e), "warn");
      } catch (_) {
        /* ignore */
      }
      return null;
    });
    apiSettingsTask.then(() => refreshAiStatus().catch(() => null));
    loadUiPrefs().catch(() => null);
    refreshBackupState().catch(() => null);
    setProgress(0, "尚未開始");
    resetCoverageMetrics("尚未開始");
    resetStepTimings();
    showAppPage("translate", { skipTransition: true });
    setTranslationState("idle");
  } catch (e) {
    forceRevealUi();
    try {
      console.error("[boot]", e);
      const detail = e && e.stack ? String(e.stack).slice(0, 200) : String(e);
      appendLog("啟動初始化失敗（介面仍可操作）：" + detail, "warn");
    } catch (_) {
      /* ignore */
    }
  }

  // —— 關鍵 UI 接線：委派保底 + 工作台提前接線；syncUiState 不可擋掉後續 ——
  try {
    wireCoverageTiers();
  } catch (e) {
    console.warn("[boot] wireCoverageTiers", e);
  }
  try {
    wireShellChrome();
  } catch (e) {
    console.warn("[boot] wireShellChrome", e);
  }
  try {
    wireTablistKeyboard();
  } catch (e) {
    console.warn("[boot] wireTablistKeyboard", e);
  }
  try {
    initSfxControls();
  } catch (e) {
    console.warn("[boot] initSfxControls", e);
  }
  try {
    initReloadGuard();
  } catch (e) {
    console.warn("[boot] initReloadGuard", e);
  }
  try {
    wireLocalCacheCard();
  } catch (e) {
    console.warn("[boot] wireLocalCacheCard", e);
  }
  try {
    applyPending.wire();
  } catch (e) {
    console.warn("[boot] applyPending.wire", e);
  }
  try {
    wireResumeCard();
  } catch (e) {
    console.warn("[boot] wireResumeCard", e);
  }
  try {
    wireWriteAccessCard();
  } catch (e) {
    console.warn("[boot] wireWriteAccessCard", e);
  }
  try {
    wireCriticalUiDelegation();
  } catch (e) {
    console.warn("[boot] wireCriticalUiDelegation", e);
  }
  try {
    void wirePrivacySettings();
  } catch (e) {
    console.warn("[boot] wirePrivacySettings", e);
  }
  try {
    wirePackMetaCard();
  } catch (e) {
    console.warn("[boot] wirePackMetaCard", e);
  }
  try {
    wireWorkbenchActions();
  } catch (e) {
    console.warn("[boot] wireWorkbenchActions", e);
  }
  try {
    packActions.wireStatusCardActions();
  } catch (e) {
    console.warn("[boot] wireStatusCardActions", e);
  }

  // —— 以下全部為同步接線（不可插入 await）；syncUiState 失敗不得中斷 ——
  try {
    syncUiState();
  } catch (e) {
    console.warn("[boot] syncUiState", e);
  }
  ["instance", "output", "font-output"].forEach((id) => {
    const input = $(id);
    if (!input) return;
    input.addEventListener("input", () => {
      hasApplyBackups = false;
      if (id === "instance" && !input.value.trim()) {
        instanceValidation = { ok: false, reason: "尚未選擇遊戲資料夾。" };
        setInstanceValidateStatus(false, instanceValidation.reason, "idle");
        setTranslationState("idle");
        // 欄位清空了，上一個資料夾的「本機已有翻譯」卡片不該還留著——不清的話，
        // 使用者清空重填的空窗期會看到一張跟目前輸入完全對不上的卡片。
        hideLocalCacheCard();
      } else {
        if (id === "output" && customOutputEnabled()) input.dataset.customPath = input.value.trim();
        if (id === "instance") {
          window.clearTimeout(input._validateTimer);
          input._validateTimer = window.setTimeout(() => {
            onInstanceTypedPath(input.value.trim());
          }, 400);
        }
        syncUiState();
        scheduleBackupStateRefresh();
      }
      scheduleBackupStateRefresh();
    });
  });
  if ($("choose-output-dir")) {
    $("choose-output-dir").addEventListener("change", () => {
      const input = $("output");
      if (!input) return;
      const autoPath = (input.dataset.autoPath || "").trim();
      if (customOutputEnabled()) {
        if ((input.value || "").trim() === autoPath) {
          input.value = (input.dataset.customPath || "").trim();
        }
      } else {
        const typed = (input.value || "").trim();
        if (typed && typed !== autoPath) input.dataset.customPath = typed;
        if (autoPath) input.value = autoPath;
      }
      syncUiState();
      scheduleBackupStateRefresh();
    });
  }
  ["font-size", "font-thickness", "font-shift-x", "font-shift-y", "font-oversample"].forEach((id) => {
    const input = $(id);
    const output = $(id + "-value");
    if (!input || !output) return;
    const syncValue = () => {
      output.textContent = input.value;
    };
    input.addEventListener("input", syncValue);
    syncValue();
  });
  // tabs／btn-inst／run／stop／quit／guide 已由 wireWorkbenchActions 提前接線

  if ($("use-ai")) $("use-ai").onchange = () => syncUiState();
  if ($("api-provider")) {
    $("api-provider").onchange = () => syncCustomProviderUi($("api-provider").value);
  }
  document.querySelectorAll('input[name="ai-source"]').forEach((radio) => {
    radio.addEventListener("change", () => {
      if (!radio.checked) return;
      const nextMode = String(radio.value || "local");
      // 「不使用 AI」只是關掉 AI 那一層，共享庫／術語表／翻譯記憶照跑，
      // 也不必為它去切換後端的 AI 模式（那會白白觸發登入檢查）。
      syncUseAiFromSource();
      if (nextMode === "none") {
        syncUiState();
        return;
      }
      currentAiMode = normalizeAiMode(nextMode);
      aiModeChangePromise = queueAiModeChange(currentAiMode);
    });
  });
  if ($("btn-gpt-login")) $("btn-gpt-login").onclick = beginGptLogin;
  if ($("btn-gpt-refresh")) $("btn-gpt-refresh").onclick = () => refreshGptStatus().catch(() => null);
  if ($("btn-gpt-cancel")) $("btn-gpt-cancel").onclick = cancelGptLoginFlow;
  if ($("btn-gpt-logout")) $("btn-gpt-logout").onclick = logoutGpt;
  wireLocalLlm({
    refreshAiStatus,
    appendLog,
    // 下載完就該能翻譯：關掉 overlay、回到工作台、把「開始翻譯」帶到眼前。
    onReadyToTranslate: () => {
      showAppPage("translate");
      const run = $("btn-run");
      if (run && !run.hidden) {
        run.scrollIntoView({ behavior: "smooth", block: "center" });
        run.focus?.();
        appendLog("本地模型已就緒，可以按「開始翻譯」。");
      } else {
        appendLog("本地模型已就緒。選好遊戲資料夾後就能開始翻譯。");
      }
    },
  });
  if ($("btn-gpt-overlay-close")) $("btn-gpt-overlay-close").onclick = closeGptLoginOverlay;
  if ($("btn-cancel-gpt-overlay")) {
    $("btn-cancel-gpt-overlay").onclick = async () => {
      await cancelGptLoginFlow();
      closeGptLoginOverlay();
    };
  }
  if ($("btn-copy-gpt-code")) {
    $("btn-copy-gpt-code").onclick = async () => {
      const value = gptDeviceCode || $("gpt-device-code")?.textContent || "";
      if (!value || value === "————") return;
      try {
        await navigator.clipboard.writeText(value);
        if ($("gpt-login-overlay-status")) $("gpt-login-overlay-status").textContent = "已複製授權碼";
      } catch (_) {
        showAppToast("無法複製授權碼");
      }
    };
  }
  if ($("btn-open-gpt-login")) {
    $("btn-open-gpt-login").onclick = () => openExternalUrl(gptLoginUrl);
  }
  if ($("btn-close-discord-fallback")) {
    $("btn-close-discord-fallback").onclick = () => {
      if ($("discord-login-fallback")) $("discord-login-fallback").hidden = true;
      showAiConfigPane(aiModeFromUi());
    };
  }

  if ($("btn-discord-login")) $("btn-discord-login").onclick = beginDiscordLogin;
  if ($("btn-discord-refresh")) $("btn-discord-refresh").onclick = refreshAiStatus;
  if ($("btn-discord-join")) {
    $("btn-discord-join").onclick = () => {
      const invite = (latestAiStatus && (latestAiStatus.inviteUrl || latestAiStatus.invite_url)) || "https://discord.gg/zeitfrei";
      return openExternalUrl(invite);
    };
  }
  if ($("btn-feedback-close")) {
    $("btn-feedback-close").onclick = () => hideFeedbackOverlay();
  }
  if ($("btn-feedback-back")) {
    $("btn-feedback-back").onclick = () => retreatFeedbackStep();
  }
  if ($("btn-feedback-next")) {
    $("btn-feedback-next").onclick = () => advanceFeedbackStep();
  }
  if ($("btn-feedback-submit")) {
    $("btn-feedback-submit").onclick = () => submitUsageFeedbackFromOverlay();
  }
  if ($("btn-issue-report")) $("btn-issue-report").onclick = () => showIssueOverlay();
  if ($("btn-issue-close")) $("btn-issue-close").onclick = () => hideIssueOverlay();
  if ($("btn-issue-submit")) $("btn-issue-submit").onclick = () => submitIssueReportFromOverlay();
  if ($("btn-issue-login")) $("btn-issue-login").onclick = () => beginDiscordLogin();
  if ($("btn-issue-join")) {
    $("btn-issue-join").onclick = () => {
      const invite = (latestAiStatus && (latestAiStatus.inviteUrl || latestAiStatus.invite_url)) || "https://discord.gg/zeitfrei";
      return openExternalUrl(invite);
    };
  }
  ["issue-summary", "issue-cause", "issue-detail"].forEach((id) => {
    const el = $(id);
    if (!el) return;
    el.addEventListener("input", () => refreshIssueReportUi());
    el.addEventListener("change", () => refreshIssueReportUi());
  });
  document.querySelectorAll('input[name="feedback-pain"], input[name="feedback-wish"], input[name="feedback-rating"]').forEach((el) => {
    el.addEventListener("change", () => updateFeedbackNoteVisibility());
  });
  if ($("btn-discord-logout")) {
    $("btn-discord-logout").onclick = async () => {
      try {
        await invoke("discord_logout");
        appendLog("已登出 Discord。");
      } catch (e) {
        appendError("Discord 登出失敗：" + formatInvokeError(e));
      }
      await refreshAiStatus();
      };
  }
  if ($("btn-open-login-url")) {
    $("btn-open-login-url").onclick = () => openExternalUrl(discordLoginUrl || $("discord-login-url")?.value || "");
  }
  if ($("btn-copy-login-url")) {
    $("btn-copy-login-url").onclick = async () => {
      const value = discordLoginUrl || $("discord-login-url")?.value || "";
      if (!value || !/^https:\/\//i.test(value)) return;
      try {
        await navigator.clipboard.writeText(value);
        appendLog("已複製 Discord 登入網址。");
      } catch (_) {
        const input = $("discord-login-url");
        input?.select();
        document.execCommand("copy");
      }
    };
  }
  if ($("btn-cancel-login")) {
    $("btn-cancel-login").onclick = async () => {
      await invoke("cancel_discord_login_cmd");
      if ($("discord-login-fallback")) $("discord-login-fallback").hidden = true;
    };
  }
  if ($("target-version")) {
    $("target-version").onchange = () => {
      const value = $("target-version").value;
      $("target-version").dataset.autoDetected = "false";
      if (value && !isSupportedMinecraftVersion(value)) {
        setVersionBlock(unsupportedVersionMessage(value));
        $("target-version").value = "";
        if ($("version-status")) $("version-status").textContent = versionBlockReason;
        syncUiState();
        return;
      }
      clearVersionBlock();
      if ($("version-status")) {
        $("version-status").textContent = value
          ? "已手動指定：Minecraft " + value
          : "將從遊戲資料夾自動偵測（須為 1.13 以上）";
      }
      syncUiState();
    };
  }
  if ($("backup-before-apply")) {
    $("backup-before-apply").onchange = saveBackupPreference;
  }
  // btn-quit／btn-run／btn-stop／supplement／repair 已由 wireWorkbenchActions 接線
  if ($("btn-save-adv")) $("btn-save-adv").onclick = onSaveAdv;
  if ($("btn-test-api")) $("btn-test-api").onclick = onTestApiKey;
  if ($("btn-glossary")) $("btn-glossary").onclick = onOpenGlossary;
  if ($("btn-merge-consistency")) $("btn-merge-consistency").onclick = onMergeConsistencySuggestions;
  if ($("btn-helper-prepare")) $("btn-helper-prepare").onclick = prepareTranslationHelper;
  if ($("btn-helper-rescan")) $("btn-helper-rescan").onclick = rescanAfterTranslationHelper;
  if ($("btn-helper-cleanup")) $("btn-helper-cleanup").onclick = cleanupTranslationHelperFromPanel;
  if ($("helper-ack-ingame")) {
    $("helper-ack-ingame").onchange = () => syncTranslationHelperPanel();
  }
  function openGuideOverlay() {
    openGuideReader();
  }
  function closeGuideOverlay() {
    /* 使用說明已移到獨立設定視窗，主視窗沒有說明浮層要關 */
  }
  if ($("onboard-skip")) {
    $("onboard-skip").onclick = () => stopOnboarding(true, { skipped: true });
  }
  if ($("onboard-prev")) {
    $("onboard-prev").onclick = () => {
      previousOnboardingStep();
    };
  }
  if ($("onboard-next")) {
    $("onboard-next").onclick = () => {
      nextOnboardingStep();
    };
  }
  if ($("onboard-shade")) {
    $("onboard-shade").onclick = () => stopOnboarding(true, { skipped: true });
  }
  // Escape：引導與同意頁的 Esc 由 ui/modal-scope.js 處理（引導＝跳過＋toast；同意頁＝不作用）。
  // 介面縮放快捷鍵由 ui-scale.js 處理
  window.addEventListener("keydown", (ev) => {
    if (ev.key === "Escape") closeGuideOverlay();
  });
  packActions.wireOverlayFocus();
  // 推廣連結（若啟動早期已接線則略過）
  document.querySelectorAll(".promo-card[data-url], #btn-ai-support[data-url]").forEach((el) => {
    if (el.dataset.wired === "1") return;
    el.dataset.wired = "1";
    el.addEventListener("click", async () => {
      const url = el.getAttribute("data-url");
      if (!url) return;
      try {
        await openExternalUrl(url);
      } catch (e) {
        appendLog("無法開啟連結：" + String(e), "warn");
      }
    });
  });
  if ($("btn-font-pick")) {
    $("btn-font-pick").onclick = async () => {
      try {
        if (!dialog.open) throw new Error("無法開啟檔案選擇");
        const f = await dialog.open({
          multiple: false,
          filters: [{ name: "字體", extensions: ["ttf", "otf"] }],
          title: "選擇你喜歡的字體檔（TTF／OTF）",
        });
        if (typeof f === "string") {
          $("font-file").value = f;
          updateFontPreview(f);
          syncUiState();
        }
      } catch (e) {
        appendFontLog(String(e), "error");
      }
    };
  }
  if ($("btn-font-remove")) {
    // 只拿掉字體工具裝進遊戲的字體包；翻譯由「移除翻譯」另外處理
    $("btn-font-remove").onclick = async () => {
      const instancePath = ($("instance")?.value || "").trim();
      if (!instancePath) return appendFontLog("請先在翻譯頁選好遊戲資料夾。");
      if (progressBusy) return appendFontLog("其他工作進行中，請稍候。");
      const ok = await confirmDialog({
        title: "移除字體包？",
        body: "會拿掉工具裝進這個模組整合包的字體包，並從資源包清單移除。翻譯不受影響。",
        affected: [instancePath],
        // D-06：不可逆、危險，預設焦點在「取消」
        danger: true,
        confirmLabel: "移除字體包",
        cancelLabel: "取消",
      });
      if (!ok) return;
      try {
        const summary = await invoke("remove_font_pack_cmd", { instancePath });
        appendFontLog(String(summary || "已移除字體包。"));
      } catch (e) {
        appendFontLog("移除字體包失敗：" + formatInvokeError(e), "error");
      }
    };
  }
  if ($("btn-font-out")) {
    $("btn-font-out").onclick = async () => {
      try {
        const p = await pickDir("選擇字體資源包輸出位置");
        if (p) {
          $("font-output").value = p;
          syncUiState();
        }
      } catch (e) {
        appendFontLog(String(e), "error");
      }
    };
  }
  if ($("btn-font-build")) {
    $("btn-font-build").onclick = () =>
      runExclusive("font-build", buildFontPackOnce, {
        onBusy: () => appendFontLog("字體資源包正在建立中，請稍候。", "warn"),
      });
    const buildFontPackOnce = async () => {
      const fontPath = ($("font-file").value || "").trim();
      const outputDir = ($("font-output")?.value || "").trim();
      if (!fontPath) return appendFontLog("請先選擇字體檔。");
      if (!outputDir) return appendFontLog("請先選擇字體包的輸出位置。");
      if (progressBusy) return appendFontLog("其他工作進行中，請稍候。");
      setBusy(true, "font");
      clearFontLog("開始建立字體資源包");
      setProgress(10, "正在建立字體資源包…");
      try {
        const targetVersion = ($("target-version")?.value || "").trim() || null;
        const r = await invoke("create_font_pack", {
          fontPath,
          outputDir,
          packName: ($("font-pack-name").value || "我的遊戲字體").trim(),
          packDesc: "自訂遊戲字體",
          fontOptions: {
            size: Number($("font-size")?.value || 11),
            weight: Number($("font-thickness")?.value || 400),
            shiftX: Number($("font-shift-x")?.value || 0),
            shiftY: Number($("font-shift-y")?.value || 0.5),
            oversample: Number($("font-oversample")?.value || 4),
          },
          targetVersion,
        });
        let finalMessage = r.playerSummary || r.player_summary || "字體資源包已建立。";
        const packPath = r.packPath || r.pack_path || "";
        const shouldApplyFont = !!$("font-apply-current")?.checked;
        const instancePath = ($("instance")?.value || "").trim();
        if (shouldApplyFont) {
          if (!instancePath) {
            appendFontLog("未選整合包，字體包已建立但未套用。", "warn");
          } else if (packPath) {
            setProgress(70, "正在套用字體包到目前實例…");
            const applied = await invoke("apply_font_pack_to_current_instance", {
              instancePath,
              fontPackPath: packPath,
            });
            finalMessage += "\n\n" + (applied.playerSummary || applied.player_summary || "已嘗試套用字體包。");
          }
        }
        setProgress(100, shouldApplyFont && instancePath ? "字體包完成並已套用" : "字體包完成");
        appendFontLog("────────");
        appendFontLog(finalMessage);
        toggleHidden("btn-open-font", false);
      } catch (e) {
        setProgress(0, "失敗", { failed: true });
        appendFontLog("建立字體包失敗：" + String(e), "error");
      } finally {
        setBusy(false);
        syncUiState();
      }
    };
  }

  if ($("btn-reference-pick")) {
    $("btn-reference-pick").onclick = async () => {
      try {
        const selected = await pickDir("選取參考翻譯資料夾");
        if (selected) {
          $("reference-pack").value = selected;
          if ($("reference-status")) $("reference-status").textContent = "已指定參考翻譯，開始翻譯時會優先套用。";
          syncUiState();
        }
      } catch (e) {
        log(String(e));
      }
    };
  }
  if ($("btn-reference-file")) {
    $("btn-reference-file").onclick = async () => {
      try {
        if (!dialog.open) throw new Error("無法開啟檔案選擇");
        const selected = await dialog.open({
          multiple: false,
          filters: [{ name: "參考翻譯包", extensions: ["zip", "jar"] }],
          title: "選取參考翻譯 zip",
        });
        if (typeof selected === "string") {
          $("reference-pack").value = selected;
          if ($("reference-status")) {
            $("reference-status").textContent = "已指定參考翻譯 zip；工具只填缺並轉台灣用語，不會上傳參考包。";
          }
          syncUiState();
        }
      } catch (e) {
        log(String(e));
      }
    };
  }
  if ($("btn-cfpa-download")) {
    $("btn-cfpa-download").onclick = async () => {
      const version = ($("target-version")?.value || "").trim();
      if (!version) {
        appendLog("請先選好整合包並確認 Minecraft 版本，再下載 CFPA。", "warn");
        return;
      }
      const btn = $("btn-cfpa-download");
      if (btn) btn.disabled = true;
      try {
        appendLog(`正在嘗試下載 CFPA（${version}）…`);
        const result = await invoke("download_cfpa_reference_pack", { mcVersion: version, destDir: null });
        const path = result?.path || "";
        if (path && $("reference-pack")) {
          $("reference-pack").value = path;
          if ($("reference-status")) {
            $("reference-status").textContent =
              result?.attribution || "已下載 CFPA 參考包；只填缺並轉台灣用語。";
          }
          if ($("reference-ack-license")) $("reference-ack-license").checked = true;
          appendLog("CFPA 下載完成：" + path);
        }
      } catch (e) {
        appendLog("CFPA 下載略過：" + formatInvokeError(e), "warn");
        if ($("reference-status")) {
          $("reference-status").textContent = "社群簡中翻譯下載失敗，可改選本機 zip／資料夾。";
        }
      } finally {
        if (btn) btn.disabled = false;
        syncUiState();
      }
    };
  }

  document.querySelectorAll(".font-preset").forEach((btn) => {
    btn.addEventListener("click", () => {
      const preset = FONT_PRESETS[btn.getAttribute("data-preset") || ""];
      if (!preset) return;
      applyFontPrefs(preset);
      writeFontPrefs();
      updateFontPreview(($("font-file")?.value || "").trim());
      // 這裡以前寫 font-rail-msg——那個 id 不存在，於是按預設按鈕完全沒有回饋。
      const rail = $("font-prog-msg");
      if (rail) rail.textContent = `已套用預設「${btn.textContent}」。可再微調後建立。`;
    });
  });
  applyFontPrefs(readFontPrefs());
  ["font-size", "font-thickness", "font-shift-x", "font-shift-y", "font-oversample", "font-pack-name"].forEach((id) => {
    const el = $(id);
    if (!el) return;
    el.addEventListener("change", writeFontPrefs);
  });

  if ($("pack-name")) {
    $("pack-name").addEventListener("input", () => {
      $("pack-name").dataset.auto = "0";
    });
  }

  // btn-inst 已由 wireWorkbenchActions / onPickInstance 接線
  if ($("btn-output-pick")) $("btn-output-pick").onclick = async () => {
    try {
      const p = await pickDir("選擇翻譯結果要放的資料夾");
      if (p) {
        $("output").value = p;
        $("output").dataset.customPath = p;
        syncUiState();
        scheduleBackupStateRefresh();
        await refreshTranslationHelper();
      }
    } catch (e) {
      log(String(e));
    }
  };
  // btn-suggest-rp 已不存在於 HTML；建議路徑改由 resolveOutputDirForInstance 自動決定。
  // btn-package 已由 wireWorkbenchActions 接線
  if ($("btn-share-confirm")) $("btn-share-confirm").onclick = confirmShareUpload;
  if ($("btn-share-cancel")) $("btn-share-cancel").onclick = closeShareConfirmation;
  ["share-confirm-reviewed", "share-confirm-private"].forEach((id) => {
    $(id)?.addEventListener("change", syncUiState);
  });
  async function openResultFolder(fromFont) {
    const outputInput = fromFont ? $("font-output") : $("output");
    const outputDir = (outputInput?.value || "").trim();
    if (!outputDir) return fromFont ? appendFontLog("還沒選字體輸出位置。") : log("還沒選結果位置。");
    try {
      const work = fromFont ? fontWorkDir(outputDir) : resultWorkDir(outputDir);
      await invoke("open_path", { path: work });
    } catch (e) {
      if (fromFont) appendFontLog(String(e), "error");
      else log(String(e));
    }
  }
  if ($("btn-open")) $("btn-open").onclick = () => openResultFolder(false);
  if ($("btn-open-font")) $("btn-open-font").onclick = () => openResultFolder(true);

  if ($("btn-clear-log")) {
    $("btn-clear-log").onclick = () => {
      clearLog("日誌已清除");
      const el = $("log");
      if (el) el.classList.add("log-empty");
    };
  }
  if ($("btn-clear-font-log")) {
    $("btn-clear-font-log").onclick = () => clearFontLog("字體日誌已清除");
  }
  ["font-size", "font-thickness", "font-shift-x", "font-shift-y"].forEach((id) => {
    $(id)?.addEventListener("input", () => {
      const out = $(id + "-value");
      const el = $(id);
      if (out && el) out.textContent = String(el.value);
      applyFontPreviewStyles();
    });
  });

  if ($("btn-open-report")) {
    $("btn-open-report").onclick = async () => {
      const outputDir = selectedOutputDir() || ($("output")?.value || "").trim();
      if (!outputDir) return log("還沒選結果位置。");
      const work = resultWorkDir(outputDir);
      try {
        await flushRunLog(work);
      } catch (e) {
        appendLog("寫入執行日誌失敗：" + String(e), "warn");
      }
      const candidates = [
        work + "\\" + RUN_LOG_FILE,
        work + "\\翻譯錯誤日誌.txt",
        work + "\\覆蓋範圍說明.txt",
        work + "\\" + DEV_TRACE_FILE,
      ];
      let opened = false;
      for (const path of candidates) {
        try {
          await invoke("open_path", { path });
          opened = true;
          break;
        } catch (_) {
          /* 試下一個報告檔 */
        }
      }
      if (opened) return;
      try {
        await invoke("open_path", { path: work });
        appendLog("尚未找到報告檔，已改開翻譯結果資料夾。", "warn");
      } catch (e) {
        log(String(e));
      }
    };
  }

  // 全部按鈕已接線；再掛後端事件（可 await，失敗不影響已綁定的 UI）
  window.clearTimeout(startupRevealFallback);
  forceRevealUi();
  revealInitialContent();
  // 先卸掉可能卡住命中的蓋層，再依規則顯示 consent；onboard 僅在 consent 已關後啟動。
  forceClearBlockingOverlays({ keepConsent: true });
  {
    showConsentOverlay();
    // 啟動時的更新檢查可能比設定檔載入更早回來而被延後；同意頁狀態確定後再判斷一次 N-01
    packActions.maybeShowUpdateBanner();
    window.setTimeout(() => {
      try {
        if ($("consent-overlay") && !$("consent-overlay").hidden) return;
        forceClearBlockingOverlays({ keepConsent: true });
        startOnboarding({ force: false });
      } catch (_) {
        /* ignore */
      }
    }, 350);
  }

  try {
    await listen("translate-progress", (ev) => {
      const p = (ev && ev.payload) || {};
      const message = p.message || "處理中…";
      queueProgressPayload(p);
      // 進度事件不自動刷紅字；真正錯誤走 translate-log error
    });
  } catch (e) {
    /* 無 event 時仍可跑完後顯示 */
  }
  try {
    await listen("translate-log", (ev) => {
      const p = (ev && ev.payload) || {};
      const level = (p.level || p.Level || "info").toLowerCase();
      const message = p.message || p.Message || "";
      if (!message) return;
      consumeCoverageMessage(message);
      if (level === "error") appendError(message);
      else if (level === "warn") appendLog(message, "warn");
      else appendLog(message, "info");
    });
  } catch (e) {
    /* 略 */
  }
  try {
    await listen("discord-login-url", (ev) => {
      const payload = (ev && ev.payload) || {};
      discordLoginUrl = String(payload.url || "").trim();
      if ($("discord-login-url")) $("discord-login-url").value = discordLoginUrl || "登入網址尚未就緒";
      if ($("discord-login-fallback")) $("discord-login-fallback").hidden = false;
    });
  } catch (e) {
    /* 瀏覽器仍可能由後端直接開啟，不阻擋登入。 */
  }
  try {
    await listen("gpt-login-code", (ev) => {
      const payload = (ev && ev.payload) || {};
      showGptLoginOverlay(payload);
    });
  } catch (e) {
    /* GPT 登入仍可能只靠瀏覽器頁面；overlay 為加值顯示。 */
  }

  try {
    await restoreLastInstanceOnStartup();
  } catch (e) {
    try {
      appendLog("啟動時還原上次整合包略過：" + formatInvokeError(e), "warn");
    } catch (_) {
      /* ignore */
    }
  }

  // 每個區塊自己保持最新，不必使用者按重新整理。
  //
  // 工具的狀態有一半來自外部（使用者去檔案總管刪了結果資料夾、換了 mods、
  // 登出 Discord…），工具偵測不到這些變動。舊版只有「本機快取」一項會在
  // 切回視窗時重探，其餘全靠使用者自己發現不對勁。
  //
  // 硬規則見 core/refresh-bus.js：值沒變不碰 DOM、使用者正在用的區塊不動、
  // 不做整頁 reload。
  try {
    configureRefreshBus({ busy: () => progressBusy || translationState === "running" });

    registerRegion({
      id: "local-cache-card",
      scope: "translate",
      when: ["focus", "interval"],
      minIntervalMs: 30000,
      refresh: async () => {
        const instancePath = ($("instance")?.value || "").trim();
        if (!instancePath) return "none";
        const probe = await probeLocalPackCache(instancePath, { silent: true }).catch(() => null);
        // 指紋：狀態與數字都沒變就不必重畫
        return probe ? `${probe.status}:${probe.pendingCount ?? probe.pending_count ?? 0}` : "none";
      },
    });

    registerRegion({
      id: "ai-options-group",
      scope: "translate",
      when: ["focus"],
      minIntervalMs: 15000,
      refresh: async () => {
        await refreshAiStatus();
        return String(latestAiStatus?.ready ?? "");
      },
    });

    startRefreshBus();
  } catch (e) {
    console.warn("[boot] refresh-bus", e);
  }

});

// 兩個視窗之間的同步**另外掛一個 DOMContentLoaded**，不擠進上面那條開機鏈。
//
// 為什麼分開：上面那條鏈很長，中途任何一個 await 拋錯，後面就全部不會跑。
// 第一版把設定視窗的初始化排在那條鏈的最後，結果開機一出錯就整片白畫面。
// 分成獨立的監聽器之後，兩邊互不影響——一邊炸了另一邊照樣跑完。
window.addEventListener("DOMContentLoaded", () => {
  wireSettingsBridge().catch((e) => {
    console.warn("[boot] settings-bridge", e);
  });
});

// ══ 主視窗 ↔ 設定視窗：設定同步與設定視窗請主視窗做的事 ═════════════════
// 設定視窗（settings.html）不載入 app.js：它自己寫設定檔，再用事件通知主視窗立刻套用。
// （本包選項早已不在設定視窗，舊的本包選項跨視窗同步已刪，B5a-2。）
async function wireSettingsBridge() {
  // settings.html 是獨立、輕量的頁面：它自己寫設定檔，再通知主工具立刻套用。
  await listen(SETTINGS_UPDATED_EVENT, (ev) => {
    // 自己送出去的（主視窗縮放同步給設定視窗）不再處理一次
    if (ev?.payload?.source === "main") return;
    routeSettingsUpdate(ev?.payload || {}, {
      store: (path, value) => applyExternalSetting(path, value),
      theme: (value) => applyTheme(value),
      uiScale: (value) => void setWebviewScalePercent(value, { persist: true, fromAuto: false }),
      uiAutoScale: (on) => void setWebviewAutoScale(on, { persist: true }),
      sfx: (prefs) => applySfxPrefs(prefs),
      outputStorage: () => void onOutputStorageChangedExternally(),
      rememberApiKey: (on) => void invoke("set_remember_api_key_cmd", { remember: on }).catch(() => {}),
    });
  });
  await listen(SETTINGS_ACTION_EVENT, (ev) => {
    routeSettingsAction(ev?.payload || {}, {
      // 重看引導與說明：重播引導，並把所有說明重設為第一次（規格 §1.3、§4.1）
      "replay-onboarding": () => {
        disclosure.resetAll();
        showAppPage("translate");
        syncUiState();
        startOnboarding({ force: true });
      },
      "show-update": () => {
        if (typeof window.zfCheckUpdate === "function") void window.zfCheckUpdate();
      },
    });
  });
  // 設定視窗做完事（清除金鑰、刪除本地模型、刪除備份）→ 主畫面立即更新；設定視窗要主視窗狀態 → 回報
  await wireSettingsNotices({
    listen,
    emit,
    readMainState: () => ({
      busy: progressBusy,
      instancePath: ($("instance")?.value || "").trim(),
      outputDir: selectedOutputDir() || "",
      packName: packNameForTranslate() || "",
    }),
    onApiKeyCleared: async () => {
      await refreshApiSettings();
      await refreshAiStatus();
    },
    onLocalModelDeleted: async () => {
      forgetLocalLlmAfterExternalDelete();
      await refreshAiStatus();
    },
    onBackupsDeleted: async (summary) => {
      if (summary) appendLog(summary, "info");
      await refreshBackupState();
    },
    // 搬移工具資料期間「開始翻譯」停用並就地說明（審查 F10）
    onDataMigratingChanged: () => syncUiState(),
  });
}

/** 設定「翻完刪除翻譯結果」（唯一位置在設定→資料與備份；第一次勾要同列確認）。 */
function deleteResultsAfterApplyEnabled() {
  return readDeleteResultsSetting(getSetting);
}

/**
 * 設定視窗改了「結果存放位置」：值已寫進設定檔與 localStorage，
 * 主視窗只要替目前選的遊戲資料夾重新算一次結果位置。
 */
async function onOutputStorageChangedExternally() {
  const instancePath = ($("instance")?.value || "").trim();
  if (instancePath && !customOutputEnabled()) {
    const base = await resolveOutputDirForInstance(instancePath);
    if (base) setAutoOutputDir(base);
    await probeLocalPackCache(instancePath, { silent: true });
  }
  appendLog(outputStorageHint(readOutputStorageMode()));
}

// 更新檢查只由主視窗做（設定視窗不載入 app.js）。
packActions.setUpdateChecker(wireUpdateChecker({
  appendLog,
  isBusy: () => progressBusy,
  // 自動檢查有新版 → 橫幅 N-01（不直接跳視窗、不與同意頁疊；測試版不會走到這裡，G0.4）
  onUpdateAvailable: (info) => packActions.maybeShowUpdateBanner(info),
}));
