const TAURI = window.__TAURI__ || {};
const invoke =
  (TAURI.core && TAURI.core.invoke) ||
  (() => Promise.reject(new Error("程式尚未就緒，請用免安裝版開啟。")));
const dialog = TAURI.dialog || {};
const listen =
  (TAURI.event && TAURI.event.listen) ||
  (async () => () => {});

const $ = (id) => document.getElementById(id);

const THEME_STORAGE_KEY = "modpack-i18n-theme";
const UI_SCALE_STORAGE_KEY = "modpack-i18n-ui-scale";
const UI_AUTOSCALE_STORAGE_KEY = "modpack-i18n-ui-autoscale";
const ONBOARDING_STORAGE_KEY = "modpack-i18n-onboarding-seen-v1.0.0";
const BACKUP_STORAGE_KEY = "modpack-i18n-backup-before-apply";
const FONT_PREFS_STORAGE_KEY = "modpack-i18n-font-prefs";
const UI_SCALE_MIN = 0.9;
const UI_SCALE_AUTO_MIN = 1.0;
const UI_SCALE_MAX = 1.5;
const UI_SCALE_STEP = 0.05;
const GP_REWARD_STORAGE_KEY = "modpack-i18n-gp-reward-v1";
const USAGE_FEEDBACK_CLIENT_ID_KEY = "modpack-i18n-usage-feedback-client-id-v1";
const USAGE_FEEDBACK_LAST_SUBMIT_AT_KEY = "modpack-i18n-usage-feedback-last-submit-at-v1";
const USAGE_FEEDBACK_LAST_NUDGE_AT_KEY = "modpack-i18n-usage-feedback-last-nudge-at-v1";
const SFX_VOLUME_STORAGE_KEY = "modpack-i18n-sfx-volume-v1";
const SFX_MUTED_STORAGE_KEY = "modpack-i18n-sfx-muted-v1";
let uiScale = 1;
let uiAutoScale = true;
let lastAutoAvailWidth = 0;
let latestAiStatus = null;
let discordLoginUrl = "";
let turnstileUrl = "";
let aiModeChangePromise = Promise.resolve();
let translationState = "idle";
let managedAiPaused = false;
let managedAiCurrentError = null;
let gpRewardInFlight = false;
let gpRewardTimerId = 0;
let pendingUsageFeedbackNudge = false;
let usageFeedbackDelayTimerId = 0;
let feedbackStep = 1;
let shareConfirmationOpen = false;
let shareUploadInFlight = false;
let lastShareUrl = "";
let lastShareInstancePath = "";
let hasShareableFiles = false;
let shareableProbeToken = 0;
let sfxVolume = 0.55;
let sfxMuted = false;
let sfxAudioCtx = null;
let sfxLastPlayAt = 0;
let sfxLastErrorAt = 0;
let sfxLastSuccessAt = 0;
let apiKeyDraft = "";
let apiKeySavedMask = "";
let apiKeyEditing = false;
let hasApplyBackups = false;
let lastDiagnosisResult = null;
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
  summary: "尚未開始",
};
/** 主譯結算後鎖定命中明細，忽略隊列／後期免費命中覆寫 */
let coverageSettlementLocked = false;
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

function prefersReducedMotion() {
  return !!window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches;
}

/** 與後端 is_supported_minecraft_version 對齊：≥1.13 或年份版 ≥26 */
function parseMcVersionParts(version) {
  const cleaned = String(version || "")
    .trim()
    .match(/^[\d.]+/);
  if (!cleaned) return null;
  const parts = cleaned[0]
    .split(".")
    .filter(Boolean)
    .map((p) => Number(p))
    .filter((n) => Number.isFinite(n));
  return parts.length >= 2 ? parts : null;
}

function isSupportedMinecraftVersion(version) {
  const parts = parseMcVersionParts(version);
  if (!parts) return false;
  if (parts[0] >= 26) return true;
  if (parts[0] !== 1) return false;
  if (parts[1] > 13) return true;
  if (parts[1] < 13) return false;
  return true; // 1.13 / 1.13.x
}

function unsupportedVersionMessage(version) {
  return (
    "本工具僅支援 Minecraft 1.13 以上（含年份版 26.x），偵測到 " +
    version +
    "，無法翻譯。"
  );
}

function clearVersionBlock() {
  versionBlocked = false;
  versionBlockReason = "";
}

function setVersionBlock(reason) {
  versionBlocked = true;
  versionBlockReason = reason || "Minecraft 版本過舊，無法翻譯。";
}

/** 強制卸除啟動骨架／is-loading，避免永遠蓋住可點元件。 */
function forceRevealUi() {
  try {
    const body = document.body;
    if (!body) return;
    body.classList.remove("is-loading");
    body.querySelectorAll(".is-loading").forEach((el) => el.classList.remove("is-loading"));
  } catch (_) {
    /* ignore */
  }
}

function revealInitialContent() {
  if (!document.body || startupContentRevealed) {
    forceRevealUi();
    return;
  }
  startupContentRevealed = true;
  forceRevealUi();
  if (prefersReducedMotion()) return;
  document.body.classList.add("content-fade-in");
  window.setTimeout(() => {
    document.body.classList.remove("content-fade-in");
  }, CONTENT_FADE_MS + 90);
}

function revealPagePanel(panel) {
  if (!panel) return;
  const token = ++pageTransitionToken;
  document.querySelectorAll(".page-panel.is-loading").forEach((el) => {
    el.classList.remove("is-loading");
  });
  panel.classList.remove("is-loading");
  panel.classList.remove("content-fade-in");
  if (prefersReducedMotion()) return;
  panel.classList.add("content-fade-in");
  window.setTimeout(() => {
    if (token === pageTransitionToken) panel.classList.remove("content-fade-in");
  }, CONTENT_FADE_MS + 90);
}

// 腳本一載入就掛 fallback：即使 DOMContentLoaded 中途拋錯，2s 內也必須露出 UI
(function bootRevealGuard() {
  const run = () => forceRevealUi();
  if (document.body) run();
  document.addEventListener("DOMContentLoaded", run);
  window.setTimeout(run, 2000);
})();

function resetCoverageMetrics(summary = "等待翻譯開始") {
  coverageSkippedSeen = new Set();
  coverageSettlementLocked = false;
  coverageMetrics = {
    glossary: 0,
    tm: 0,
    shared: 0,
    ai: 0,
    pending: null,
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
    summary,
  };
  renderCoverageMetrics();
}

function formatCount(n) {
  const num = Number(n) || 0;
  try {
    return num.toLocaleString("zh-TW");
  } catch (_) {
    return String(num);
  }
}

function renderCoverageMetrics() {
  const setText = (id, value) => {
    const el = $(id);
    if (el) el.textContent = String(value);
  };
  setText("metric-glossary", coverageMetrics.glossary);
  setText("metric-tm", coverageMetrics.tm);
  setText("metric-shared", coverageMetrics.shared);
  setText("metric-ai", coverageMetrics.ai);
  setText("metric-skipped", coverageMetrics.skipped);
  setText("metric-pending", coverageMetrics.pending == null ? "—" : coverageMetrics.pending);
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
  setText(
    "metric-token-hit",
    coverageMetrics.cacheHitTokens == null ? "—" : formatCount(coverageMetrics.cacheHitTokens)
  );
  setText(
    "metric-token-miss",
    coverageMetrics.cacheMissTokens == null ? "—" : formatCount(coverageMetrics.cacheMissTokens)
  );
  setText(
    "metric-token-out",
    coverageMetrics.completionTokens == null ? "—" : formatCount(coverageMetrics.completionTokens)
  );
  const translated =
    (Number(coverageMetrics.glossary) || 0) +
    (Number(coverageMetrics.tm) || 0) +
    (Number(coverageMetrics.shared) || 0) +
    (Number(coverageMetrics.ai) || 0);
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
    if (coverageMetrics.pending != null) {
      const total = translated + (Number(coverageMetrics.pending) || 0);
      countEl.textContent = `已翻譯 ${formatCount(translated)} / ${formatCount(total)} 條目`;
    } else if (translated > 0) {
      countEl.textContent = `已翻譯 ${formatCount(translated)} 條目`;
    } else {
      countEl.textContent = coverageMetrics.summary || "尚未開始";
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
  if (pendingHit) {
    coverageMetrics.pending = Number(pendingHit[1]) || 0;
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

function clampUiScale(value) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return 1;
  return Math.min(UI_SCALE_MAX, Math.max(UI_SCALE_MIN, Math.round(parsed * 100) / 100));
}

function clampAutoUiScale(value) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return 1;
  return Math.min(UI_SCALE_MAX, Math.max(UI_SCALE_AUTO_MIN, Math.round(parsed * 100) / 100));
}

function computeAutoScale() {
  const w = (window.screen && screen.availWidth) || 1920;
  let pct = 100;
  if (w <= 1920) pct = 100;
  else if (w <= 2304) pct = 110;
  else if (w <= 2560) pct = 120;
  else if (w <= 3200) pct = 130;
  else pct = 140;
  return clampAutoUiScale(pct / 100);
}

function isUiAutoScaleOn() {
  return uiAutoScale !== false;
}

function applyUiScale(value, save = true, opts = {}) {
  const fromAuto = !!opts.fromAuto;
  uiScale = clampUiScale(value);
  document.documentElement.style.setProperty("--ui-scale", String(uiScale));
  const label = $("scale-label");
  const button = $("btn-scale");
  const percent = Math.round(uiScale * 100);
  if (label) {
    label.textContent = fromAuto || isUiAutoScaleOn() ? `介面 ${percent}%（自動）` : `介面 ${percent}%`;
  }
  if (button) {
    button.title = isUiAutoScaleOn()
      ? "已開啟自動依螢幕調整；關閉後可用 Ctrl＋↑／↓或滾輪手動調整"
      : `Ctrl＋↑／↓或 Ctrl＋滾輪調整介面大小；點擊重設為 100%（目前 ${percent}%）`;
    button.disabled = isUiAutoScaleOn();
  }
  if (save && !fromAuto) {
    try {
      localStorage.setItem(UI_SCALE_STORAGE_KEY, String(uiScale));
    } catch (_) {
      /* 瀏覽器儲存不可用時仍保留本次縮放 */
    }
  }
}

function applyAutoScale() {
  const w = (window.screen && screen.availWidth) || 1920;
  lastAutoAvailWidth = w;
  applyUiScale(computeAutoScale(), false, { fromAuto: true });
}

function setUiAutoScale(on, persist = true) {
  uiAutoScale = !!on;
  const box = $("ui-autoscale");
  if (box) box.checked = uiAutoScale;
  if (persist) {
    try {
      localStorage.setItem(UI_AUTOSCALE_STORAGE_KEY, uiAutoScale ? "1" : "0");
    } catch (_) {
      /* ignore */
    }
  }
  if (uiAutoScale) applyAutoScale();
  else {
    let saved = 1;
    try {
      saved = clampUiScale(localStorage.getItem(UI_SCALE_STORAGE_KEY) || 1);
    } catch (_) {
      /* default */
    }
    applyUiScale(saved, false);
  }
}

function initUiScale() {
  let auto = true;
  try {
    const raw = localStorage.getItem(UI_AUTOSCALE_STORAGE_KEY);
    if (raw === "0") auto = false;
    if (raw === "1") auto = true;
  } catch (_) {
    /* default on */
  }
  uiAutoScale = auto;
  const box = $("ui-autoscale");
  if (box) box.checked = auto;
  if (auto) applyAutoScale();
  else {
    let saved = 1;
    try {
      saved = clampUiScale(localStorage.getItem(UI_SCALE_STORAGE_KEY) || 1);
    } catch (_) {
      /* 使用預設比例 */
    }
    applyUiScale(saved, false);
  }
  window.setInterval(() => {
    if (!isUiAutoScaleOn()) return;
    const w = (window.screen && screen.availWidth) || 1920;
    if (w !== lastAutoAvailWidth) applyAutoScale();
  }, 1500);
}

function adjustUiScale(delta) {
  if (isUiAutoScaleOn()) {
    zoomAutoHint();
    return;
  }
  applyUiScale(uiScale + delta);
}

let _mouseXY = { x: 24, y: 72 };
let _zoomHintTimer = null;

window.addEventListener(
  "pointermove",
  (ev) => {
    _mouseXY.x = ev.clientX;
    _mouseXY.y = ev.clientY;
  },
  { passive: true, capture: true }
);

function placeNearCursor(el, x, y, gap = 14) {
  if (!el) return;
  el.style.visibility = "hidden";
  el.style.left = "0px";
  el.style.top = "0px";
  const rect = el.getBoundingClientRect();
  const vw = window.innerWidth || 800;
  const vh = window.innerHeight || 600;
  let left = x + gap;
  let top = y + gap;
  if (left + rect.width > vw - 8) left = Math.max(8, x - rect.width - gap);
  if (top + rect.height > vh - 8) top = Math.max(8, y - rect.height - gap);
  el.style.left = `${Math.round(left)}px`;
  el.style.top = `${Math.round(top)}px`;
  el.style.visibility = "";
}

function zoomAutoHint() {
  let el = document.getElementById("zoom-hint");
  if (!el) {
    el = document.createElement("div");
    el.id = "zoom-hint";
    el.className = "zoom-hint";
    el.setAttribute("role", "status");
    document.body.appendChild(el);
  }
  el.textContent = "介面縮放已鎖定（自動縮放開啟中；可到 ⋯ 設定關閉後再手動調整）";
  el.classList.add("show");
  placeNearCursor(el, _mouseXY.x, _mouseXY.y, 14);
  if (_zoomHintTimer) clearTimeout(_zoomHintTimer);
  _zoomHintTimer = setTimeout(() => {
    el.classList.remove("show");
  }, 1400);
}

const ONBOARD_STEPS = [
  {
    selector: ".service-tabs",
    title: "三個服務分頁",
    body: "頂部分頁可切換「翻譯」「字體」「診斷」。一般先留在翻譯頁；出問題再開診斷。",
  },
  {
    selector: ".path-block",
    title: "先選遊戲資料夾",
    body: "選取 Minecraft 實例資料夾並通過檢查後，才會顯示 AI、開始翻譯與右側步驟／日誌。",
  },
  {
    selector: "#ai-options-group",
    fallback: "#path-gate-hint",
    title: "AI 輔助（可選）",
    body: "免費代管需 Discord；額度用盡或自訂失敗時，推薦改用自訂 API 的 DeepSeek（便宜划算），到 platform.deepseek.com 申請金鑰。也可關閉 AI 只做本機整理。",
  },
  {
    selector: "#btn-run",
    fallback: "#path-gate-hint",
    title: "開始翻譯",
    body: "通過資料夾檢查後按「開始翻譯」。完成會盡量自動套用；不會宣稱 100%。忙碌時「停止」與「更多選項」同排。",
  },
  {
    selector: "#tab-diagnose",
    fallback: ".service-tabs",
    title: "錯誤分析",
    body: "可貼 crash／log，或選整合包資料夾做記錄＋mods 交叉驗證。每次只給一個主因與下一步。",
  },
  {
    selector: "#tab-font",
    fallback: ".service-tabs",
    title: "字體資源包",
    body: "把 .ttf／.otf 打成資源包，可選套用到目前實例。與翻譯無關，中文變□多半是字體問題。",
  },
  {
    selector: "#btn-overflow",
    title: "⋯ 設定",
    body: "右上角 ⋯＝設定：外觀、自動縮放、檢查更新、完整使用說明，以及重播本引導都在這裡。",
  },
];

let onboardIndex = 0;
let onboardActive = false;

function hasSeenOnboarding() {
  try {
    return localStorage.getItem(ONBOARDING_STORAGE_KEY) === "1";
  } catch (_) {
    return false;
  }
}

function markOnboardingSeen() {
  try {
    localStorage.setItem(ONBOARDING_STORAGE_KEY, "1");
  } catch (_) {
    /* ignore */
  }
}

function resolveOnboardTarget(step) {
  if (!step) return null;
  let el = document.querySelector(step.selector);
  if (el) {
    const style = window.getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    const visible =
      style.display !== "none" &&
      style.visibility !== "hidden" &&
      rect.width > 0 &&
      rect.height > 0;
    if (visible) return el;
  }
  if (step.fallback) return document.querySelector(step.fallback);
  return el;
}

function layoutOnboarding() {
  if (!onboardActive) return;
  const step = ONBOARD_STEPS[onboardIndex];
  const root = $("onboard-root");
  const hole = $("onboard-hole");
  const bubble = $("onboard-bubble");
  const meta = $("onboard-meta");
  const title = $("onboard-title");
  const body = $("onboard-body");
  const prev = $("onboard-prev");
  const next = $("onboard-next");
  if (!root || !bubble || !step) return;

  if (meta) meta.textContent = `${onboardIndex + 1} / ${ONBOARD_STEPS.length}`;
  if (title) title.textContent = step.title;
  if (body) body.textContent = step.body;
  if (prev) prev.disabled = onboardIndex <= 0;
  if (next) next.textContent = onboardIndex >= ONBOARD_STEPS.length - 1 ? "完成" : "下一步";

  const target = resolveOnboardTarget(step);
  const pad = 8;
  const vw = window.innerWidth || 800;
  const vh = window.innerHeight || 600;
  let holeRect = { left: vw * 0.2, top: vh * 0.2, width: vw * 0.6, height: 80 };
  if (target) {
    const r = target.getBoundingClientRect();
    holeRect = {
      left: Math.max(8, r.left - pad),
      top: Math.max(8, r.top - pad),
      width: Math.min(vw - 16, r.width + pad * 2),
      height: Math.min(vh - 16, r.height + pad * 2),
    };
  }
  if (hole) {
    hole.hidden = false;
    hole.style.left = `${Math.round(holeRect.left)}px`;
    hole.style.top = `${Math.round(holeRect.top)}px`;
    hole.style.width = `${Math.round(holeRect.width)}px`;
    hole.style.height = `${Math.round(holeRect.height)}px`;
  }

  bubble.style.visibility = "hidden";
  bubble.style.left = "0px";
  bubble.style.top = "0px";
  const b = bubble.getBoundingClientRect();
  let left = holeRect.left;
  let top = holeRect.top + holeRect.height + 12;
  if (top + b.height > vh - 12) top = Math.max(12, holeRect.top - b.height - 12);
  if (left + b.width > vw - 12) left = Math.max(12, vw - b.width - 12);
  if (left < 12) left = 12;
  if (top < 12) top = 12;
  bubble.style.left = `${Math.round(left)}px`;
  bubble.style.top = `${Math.round(top)}px`;
  bubble.style.visibility = "";
}

function stopOnboarding(markSeen) {
  onboardActive = false;
  const root = $("onboard-root");
  if (root) {
    root.hidden = true;
    root.classList.remove("is-active");
    root.setAttribute("aria-hidden", "true");
  }
  if (markSeen) markOnboardingSeen();
  window.removeEventListener("resize", layoutOnboarding);
}

function startOnboarding(opts = {}) {
  const force = !!opts.force;
  if (!force && hasSeenOnboarding()) return;
  const root = $("onboard-root");
  if (!root) return;
  closeGuideOverlaySafe();
  onboardIndex = 0;
  onboardActive = true;
  root.hidden = false;
  root.classList.add("is-active");
  root.setAttribute("aria-hidden", "false");
  layoutOnboarding();
  window.addEventListener("resize", layoutOnboarding);
}

function closeGuideOverlaySafe() {
  const ov = $("guide-overlay");
  if (!ov) return;
  ov.hidden = true;
  ov.setAttribute("aria-hidden", "true");
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
      if (managedAiPaused) {
        const ok = window.confirm("代管 AI 暫停中，確定要關閉工具嗎？");
        if (!ok) return;
      }
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

function shouldBackupBeforeApply() {
  return $("backup-before-apply") ? $("backup-before-apply").checked : true;
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
  const deleter = $("btn-delete-output");
  if (deleter) deleter.hidden = !hasInstance || !selectedOutputDir() || progressBusy;
}

function setAutoOutputDir(path) {
  const input = $("output");
  if (!input) return;
  const value = String(path || "").trim();
  input.dataset.autoPath = value;
  if (!customOutputEnabled()) input.value = value;
  const status = $("output-status");
  if (status) status.textContent = value
    ? "翻譯會在這個位置建立「翻譯結果」；完成後直接套用到整合包資料夾。"
    : "請先選擇整合包資料夾。";
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

function applyTheme(theme) {
  const normalized = theme === "light" ? "light" : "dark";
  document.documentElement.dataset.theme = normalized;
  const button = $("btn-theme");
  const label = $("theme-label");
  const glyph = $("theme-glyph");
  const meta = document.querySelector('meta[name="theme-color"]');
  if (button) {
    button.setAttribute("aria-pressed", normalized === "dark" ? "true" : "false");
    button.title = normalized === "dark" ? "切換到淺色模式" : "切換到深色模式";
  }
  if (label) label.textContent = normalized === "dark" ? "深色" : "淺色";
  if (glyph) glyph.textContent = normalized === "dark" ? "◐" : "○";
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

/** —— SFX：輕量 WebAudio 點音效（不依賴外部 ogg 檔）—— */
const SFX_THROTTLE_MS = 90;
const SFX_ERROR_GUARD_MS = 1500;
const SFX_SUCCESS_GUARD_MS = 2500;
// 優先用很小的本地 ogg 檔；success/error 仍以 WebAudio tone 當保底（避免非使用者觸發時瀏覽器拒絕自動播放）。
const SFX_AUDIO_URLS = {
  click: "assets/audio/pickup2.ogg",
  toggle: "assets/audio/chip-lay-1.ogg",
  tick: "assets/audio/chips-stack-2.ogg",
  scroll: "assets/audio/chips-stack-5.ogg",
};
const sfxAudioPool = {};

function clampSfxVolume(v) {
  const n = Number(v);
  if (!Number.isFinite(n)) return 0.55;
  return Math.min(1, Math.max(0, n));
}

function ensureSfxAudioContext() {
  if (sfxAudioCtx) return sfxAudioCtx;
  const AudioCtx = window.AudioContext || window.webkitAudioContext;
  if (!AudioCtx) return null;
  try {
    sfxAudioCtx = new AudioCtx();
    return sfxAudioCtx;
  } catch (_) {
    sfxAudioCtx = null;
    return null;
  }
}

function playSfx(kind) {
  if (sfxMuted) return;
  const vol = clampSfxVolume(sfxVolume);
  if (!vol) return;
  const now = Date.now();
  if (now - sfxLastPlayAt < SFX_THROTTLE_MS) return;
  sfxLastPlayAt = now;

  const audioUrl = SFX_AUDIO_URLS[kind];
  if (audioUrl) {
    try {
      const a = sfxAudioPool[kind] || new Audio(audioUrl);
      sfxAudioPool[kind] = a;
      a.volume = vol;
      a.currentTime = 0;
      const p = a.play();
      if (p && typeof p.catch === "function") p.catch(() => {});
      return; // 由於 click/toggle/tick/scroll 都是使用者觸發，預期可以播放成功
    } catch (_) {
      /* fallback: 改用 WebAudio tone */
    }
  }

  const ctx = ensureSfxAudioContext();
  if (!ctx) return;
  try {
    if (ctx.state === "suspended" && typeof ctx.resume === "function") ctx.resume().catch(() => null);
  } catch (_) {
    /* ignore */
  }

  const t0 = ctx.currentTime || 0;
  const makeTone = (freq, durMs, wave = "sine", gainMul = 1) => {
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = wave;
    osc.frequency.setValueAtTime(freq, t0);
    const peak = Math.max(0.001, vol * 0.22 * gainMul);
    gain.gain.setValueAtTime(0.0001, t0);
    gain.gain.exponentialRampToValueAtTime(peak, t0 + 0.01);
    gain.gain.exponentialRampToValueAtTime(0.0001, t0 + durMs / 1000);
    osc.connect(gain);
    gain.connect(ctx.destination);
    osc.start(t0);
    osc.stop(t0 + durMs / 1000 + 0.02);
  };

  // 目標：避免「吵」；用短包絡 + 少量頻率變化。
  switch (kind) {
    case "success":
      makeTone(1046.5, 95, "triangle", 1.0);
      makeTone(1318.5, 95, "sine", 0.8);
      break;
    case "error":
      makeTone(220, 115, "sawtooth", 0.9);
      makeTone(165, 115, "square", 0.35);
      break;
    case "tick":
      makeTone(740, 45, "sine", 0.7);
      break;
    case "scroll":
      makeTone(520, 55, "sine", 0.45);
      break;
    case "toggle":
      makeTone(660, 70, "sine", 0.6);
      break;
    case "click":
    default:
      makeTone(880, 55, "sine", 0.55);
      break;
  }
}

function maybePlaySfxError() {
  const now = Date.now();
  if (now - sfxLastErrorAt < SFX_ERROR_GUARD_MS) return;
  sfxLastErrorAt = now;
  playSfx("error");
}

function maybePlaySfxSuccess(message) {
  const now = Date.now();
  if (now - sfxLastSuccessAt < SFX_SUCCESS_GUARD_MS) return;
  const m = String(message || "");
  if (!/全部完成|補翻完成|補譯完成|字體包完成|修復完成|已套用|完成/.test(m)) return;
  sfxLastSuccessAt = now;
  playSfx("success");
}

function initSfxControls() {
  const mutedBox = $("sfx-muted");
  const volInput = $("sfx-volume");
  const volLabel = $("sfx-volume-label");

  try {
    const storedVol = clampSfxVolume(localStorage.getItem(SFX_VOLUME_STORAGE_KEY));
    sfxVolume = storedVol;
  } catch (_) {
    /* ignore */
  }
  try {
    const storedMuted = localStorage.getItem(SFX_MUTED_STORAGE_KEY);
    sfxMuted = storedMuted === "1";
  } catch (_) {
    /* ignore */
  }

  if (mutedBox) {
    mutedBox.checked = !!sfxMuted;
    mutedBox.addEventListener("change", () => {
      sfxMuted = !!mutedBox.checked;
      try {
        localStorage.setItem(SFX_MUTED_STORAGE_KEY, sfxMuted ? "1" : "0");
      } catch (_) {
        /* ignore */
      }
    });
  }

  if (volInput) {
    volInput.value = String(clampSfxVolume(sfxVolume));
    const applyUi = () => {
      const v = clampSfxVolume(volInput.value);
      if (sfxMuted && v > 0 && mutedBox && mutedBox.checked) {
        // 靜音與音量分開：不自動取消靜音
      }
      sfxVolume = v;
      if (volLabel) volLabel.textContent = Math.round(v * 100) + "%";
      try {
        localStorage.setItem(SFX_VOLUME_STORAGE_KEY, String(v));
      } catch (_) {
        /* ignore */
      }
    };
    volInput.addEventListener("input", applyUi);
    applyUi();
  }

  // 使用者點擊：用事件委派觸發 click，避免「滾動/拖曳」造成大量雜訊。
  document.addEventListener(
    "click",
    (ev) => {
      const t = ev.target;
      if (t && t.tagName === "INPUT" && t.type === "checkbox") {
        const id = String(t.id || "");
        if (id === "sfx-muted") return;
        if (sfxMuted) return;
        playSfx("toggle");
        return;
      }
      const el = t && t.closest ? t.closest("button, a") : null;
      if (!el) return;
      if (el instanceof HTMLInputElement) return;
      if (el.tagName === "A" && !el.getAttribute("href")) return;
      const id = String(el.id || "");
      if (!id) return;
      if (!/^btn-|^winbar-|^btn-win-/.test(id) && !el.classList.contains("wb-btn")) {
        return;
      }
      if (el.disabled) return;
      if (sfxMuted) return;
      playSfx("click");
    },
    { capture: true, passive: true }
  );
}

/** UI 日誌：只保留最近幾則供右欄漸進顯示；完整內容看結果資料夾報告檔 */
const progressLogLines = [];
const UI_LOG_VISIBLE = 6;
const MAX_LOG_LINES = 2000;
const RUN_LOG_FILE = "執行日誌.txt";
const FONT_LOG_FILE = "字體執行日誌.txt";
const DEV_TRACE_FILE = "開發進度偵測.txt";
const RUN_LOG_MAX_BYTES = 256 * 1024;
let errorLogCount = 0;
const fontLogLines = [];
const diagnoseLogLines = [];
let fontPreviewFace = null;

function nowStamp() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, "0");
  return p(d.getHours()) + ":" + p(d.getMinutes()) + ":" + p(d.getSeconds());
}

function visibleLogLines() {
  if (progressLogLines.length <= UI_LOG_VISIBLE) return progressLogLines.slice();
  return progressLogLines.slice(-UI_LOG_VISIBLE);
}

function renderLogNow() {
  const el = $("log");
  if (!el) return;
  el.classList.remove("log-empty");
  el.textContent = visibleLogLines().join("\n");
  el.scrollTop = el.scrollHeight;
}

function renderPanelLog(el, lines) {
  if (!el) return;
  el.classList.toggle("log-empty", lines.length === 0);
  const visible = lines.length <= UI_LOG_VISIBLE ? lines.slice() : lines.slice(-UI_LOG_VISIBLE);
  el.textContent = visible.join("\n");
  el.scrollTop = el.scrollHeight;
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

function clearDiagnoseLog(seedMsg) {
  diagnoseLogLines.length = 0;
  if (seedMsg) diagnoseLogLines.push("[" + nowStamp() + "] " + seedMsg);
  renderPanelLog($("diagnose-log"), diagnoseLogLines);
}

function appendDiagnoseLog(msg, level) {
  appendPanelLog(diagnoseLogLines, msg, level, renderPanelLog, $("diagnose-log"));
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

function appendError(msg) {
  appendLog(msg, "error");
}

/** 把記憶體日誌覆寫成執行日誌.txt（有上限），供「報告」開啟 */
async function flushRunLog(workDir) {
  if (!workDir) return;
  const header =
    "【執行日誌】\n最近過程狀態。每次按報告或翻譯任務結束時覆寫。\n\n";
  let body = header + progressLogLines.join("\n") + "\n";
  if (body.length > RUN_LOG_MAX_BYTES) {
    body = body.slice(0, 800) + "\n…（已截斷）…\n" + body.slice(-(RUN_LOG_MAX_BYTES - 1200));
  }
  const path = String(workDir).replace(/[\\/]+$/, "") + "\\" + RUN_LOG_FILE;
  await invoke("write_text_file", { path, content: body });
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
  appendLog("完整紀錄請按「開啟報告」。", "warn");
  maybeHintAiQuota(msg);
}

/** AI 額度用完：提示支持（不提服務商名稱） */
function maybeHintAiQuota(text) {
  const s = String(text || "");
  if (!/額度|餘額|沒有回應|金鑰無效|無權限|沒有有效回應|請我喝珍奶|沒有餘力|免費代管|429|quota exhausted/.test(s)) {
    return;
  }
  appendLog("────────", "warn");
  appendLog("代管翻譯由開發者個人提供，不是無限額度。", "warn");
  appendLog("額度用盡時代管不可用；共享庫與本機轉換仍可繼續。", "warn");
  appendLog(
    "可支持開發，或改用自訂 API：推薦 DeepSeek（便宜划算），到 platform.deepseek.com 申請後選 DeepSeek 填入。",
    "warn"
  );
}

/** 使用者按停止不是錯誤，畫面不該變成一片紅字 */
function isCancellation(e) {
  return /已依你的要求停止/.test(formatInvokeError(e));
}

/** 統一處理各流程的失敗／取消收尾 */
function handleRunFailure(e, whatFailed) {
  if (isCancellation(e)) {
    setProgress(Math.max(lastRealPercent, Math.floor(displayPercent) || 0), "已停止", {
      payload: { ...(lastProgressPayload || {}), state: "cancelling" },
    });
    // 停止掃尾文案由各流程 catch 自行補充；此處只更新進度
    return;
  }
  setProgress(Math.max(lastRealPercent, Math.floor(displayPercent) || 0), whatFailed, {
    failed: true,
    payload: lastProgressPayload,
  });
  appendError(whatFailed);
  appendError(formatInvokeError(e));
}

function formatInvokeError(e) {
  if (e == null) return "未知錯誤";
  if (typeof e === "string") return e;
  if (e.message) return e.message + (e.stack ? "\n" + e.stack : "");
  try {
    return JSON.stringify(e, null, 2);
  } catch (_) {
    return String(e);
  }
}

function classifyManagedAiError(e) {
  const detail = String(formatInvokeError(e || "") || "").trim();
  if (!detail) return null;
  const m = detail.toLowerCase();

  if (
    detail.includes("client upgrade required") ||
    detail.includes("client_upgrade_required") ||
    m.includes("版本已不能使用")
  ) {
    return { kind: "client_upgrade_required", title: "代管 AI 需要更新", message: detail, extra: "" };
  }

  if (
    detail.includes("login_required") ||
    detail.includes("Discord 尚未登入") ||
    detail.includes("請先登入 Discord") ||
    detail.includes("請回到工具重新登入")
  ) {
    return { kind: "login_required", title: "代管 AI 需要 Discord 驗證", message: detail, extra: "" };
  }

  if (
    detail.includes("guild_required") ||
    detail.includes("加入 ZeitFrei 官方 Discord") ||
    detail.includes("尚未加入官方伺服器") ||
    detail.includes("加入官方伺服器")
  ) {
    return { kind: "guild_required", title: "代管 AI 需要加入官方伺服器", message: detail, extra: "" };
  }

  if (
    detail.includes("代管額度已用盡") ||
    detail.includes("當日額度") ||
    detail.includes("餘額") ||
    detail.includes("免費翻譯的當日額度") ||
    detail.includes("額度") ||
    m.includes("quota") ||
    m.includes("balance")
  ) {
    const isSharedWeekly =
      detail.includes("本週共享額度") ||
      detail.includes("共享本週") ||
      detail.includes("sharedPeriod") ||
      detail.includes("sharedResetAtUtc") ||
      m.includes("managed shared weekly quota") ||
      m.includes("shared weekly");
    return {
      kind: isSharedWeekly ? "shared_weekly_quota" : "quota_exhausted",
      title: isSharedWeekly ? "本週共享額度已用完" : "代管 AI 額度用盡",
      message: detail,
      extra: isSharedWeekly ? "每週一重置" : "",
    };
  }

  if (
    detail.includes("server_not_ready") ||
    detail.includes("服務暫時無法使用") ||
    detail.includes("維護") ||
    detail.includes("暫時無法使用")
  ) {
    return { kind: "server_not_ready", title: "代管 AI 目前不可用", message: detail, extra: "" };
  }

  return null;
}

function hideManagedAiErrorModal() {
  managedAiPaused = false;
  managedAiCurrentError = null;
  const ov = $("managed-ai-error-overlay");
  if (ov) {
    ov.hidden = true;
    ov.setAttribute("aria-hidden", "true");
  }
}

function showManagedAiErrorModal(cls) {
  if (!cls) return;
  managedAiPaused = true;
  managedAiCurrentError = cls.kind;

  const ov = $("managed-ai-error-overlay");
  if (!ov) return;
  ov.hidden = false;
  ov.setAttribute("aria-hidden", "false");

  const title = $("managed-ai-error-title");
  const msg = $("managed-ai-error-message");
  const extra = $("managed-ai-error-extra");
  if (title) title.textContent = cls.title || "代管 AI 需要確認";
  if (msg) msg.textContent = cls.message || "";
  if (extra) extra.textContent = cls.extra || "";

  const actions = $("managed-ai-error-actions");
  if (actions) {
    actions.innerHTML = "";

    const mkBtn = (id, text, className) => {
      const b = document.createElement("button");
      b.type = "button";
      b.id = id;
      b.className = className || "secondary-button";
      b.textContent = text;
      return b;
    };

    actions.appendChild(mkBtn("btn-managed-ai-try-later", "稍後再試"));

    if (cls.kind === "login_required" || cls.kind === "guild_required") {
      actions.appendChild(mkBtn("btn-managed-ai-verify-discord", "去驗證 Discord"));
    } else {
      actions.appendChild(mkBtn("btn-managed-ai-continue", "繼續"));
      actions.appendChild(mkBtn("btn-managed-ai-custom-api", "去填自訂 API"));
    }

    const btnTryLater = $("btn-managed-ai-try-later");
    if (btnTryLater) {
      btnTryLater.onclick = () => {
        hideManagedAiErrorModal();
        setTranslationState("idle");
      };
    }

    const btnVerify = $("btn-managed-ai-verify-discord");
    if (btnVerify) {
      btnVerify.onclick = () => {
        hideManagedAiErrorModal();
        setTranslationState("idle");
        try {
          syncAiModeUi("managed");
          if ($("ai-auth-details")) $("ai-auth-details").open = true;
          $("managed-auth-panel")?.scrollIntoView({ behavior: "smooth", block: "nearest" });
        } catch (_) {}
      };
    }

    const btnContinue = $("btn-managed-ai-continue");
    if (btnContinue) {
      btnContinue.onclick = () => {
        hideManagedAiErrorModal();
        setTranslationState("idle");
      };
    }

    const btnCustom = $("btn-managed-ai-custom-api");
    if (btnCustom) {
      btnCustom.onclick = () => {
        hideManagedAiErrorModal();
        setTranslationState("idle");
        void changeAiMode("custom").then(() => {
          $("api-key")?.focus();
          if ($("adv-details")) $("adv-details").open = true;
        });
      };
    }
  }
}

function formatIsoToTaipei(iso) {
  try {
    return new Date(iso).toLocaleString("zh-TW", {
      timeZone: "Asia/Taipei",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch (_) {
    return String(iso || "");
  }
}

let lastManagedUsageRefreshAt = 0;

async function refreshManagedAiUsageIndicator(opts = {}) {
  const el = $("managed-ai-usage-indicator");
  if (!el) return;
  if (!$("managed-ai-usage-personal")) return;

  const now = Date.now();
  const minInterval = opts.force ? 0 : 25 * 1000;
  if (!opts.force && now - lastManagedUsageRefreshAt < minInterval) return;

  try {
    const u = await invoke("managed_ai_usage_cmd");
    if (!u || u.ok !== true) {
      el.hidden = true;
      el.setAttribute("aria-hidden", "true");
      return;
    }

    const personalSpent = Number(u.userSpent || 0);
    const personalBudget = Number(u.userBudget || 0);
    const sharedSpent = Number(u.sharedSpent || 0);
    const sharedBudget = Number(u.sharedBudget || 0);

    const setCount = (id, spent, budget) => {
      const el2 = $(id);
      if (!el2) return;
      el2.textContent =
        budget > 0 ? `${formatCount(spent)} / ${formatCount(budget)}` : `${formatCount(spent)}`;
    };

    setCount("managed-ai-usage-personal", personalSpent, personalBudget);
    setCount("managed-ai-usage-shared", sharedSpent, sharedBudget);

    const pct = (spent, budget) => {
      if (!budget || budget <= 0) return 0;
      return Math.max(0, Math.min(100, Math.round((spent / budget) * 100)));
    };

    const p = pct(personalSpent, personalBudget);
    const s = pct(sharedSpent, sharedBudget);
    $("managed-ai-usage-personal-bar").style.width = p + "%";
    $("managed-ai-usage-shared-bar").style.width = s + "%";

    const resetAtUtc = u.resetAtUtc || "";
    const sharedPeriod = String(u.sharedPeriod || u.shared_period || "").trim();
    const sharedResetAtUtc = u.sharedResetAtUtc || u.shared_reset_at_utc || "";
    if ($("managed-ai-usage-reset")) {
      if (sharedPeriod === "week" && sharedResetAtUtc) {
        $("managed-ai-usage-reset").textContent = `共享重置：${formatIsoToTaipei(sharedResetAtUtc)}（每週一）`;
      } else if (resetAtUtc) {
        $("managed-ai-usage-reset").textContent = `重置：${formatIsoToTaipei(resetAtUtc)}`;
      } else {
        $("managed-ai-usage-reset").textContent = "重置時間：—";
      }
    }

    el.hidden = false;
    el.setAttribute("aria-hidden", "false");
    lastManagedUsageRefreshAt = now;
  } catch (_) {
    el.hidden = true;
    el.setAttribute("aria-hidden", "true");
  }
}

function setGpRewardPromptVisible(visible) {
  const prompt = $("gp-reward-prompt");
  if (!prompt) return;
  prompt.hidden = !visible;
  prompt.setAttribute("aria-hidden", visible ? "false" : "true");
}

async function startGpRewardCountdown() {
  if (gpRewardInFlight) return;
  const prompt = $("gp-reward-prompt");
  const status = $("gp-reward-status");
  const btn = $("btn-gp-reward");
  if (!prompt || !status || !btn) return;

  gpRewardInFlight = true;
  btn.disabled = true;
  status.textContent = "已開啟巴哈姆特貼文，約 60 秒後確認 GP…";

  try {
    const link = "https://forum.gamer.com.tw/C.php?bsn=18673&snA=205441&tnum=1";
    await openExternalUrl(link);
  } catch (_) {
    // 連結開啟失敗也照樣走等待流程
  }

  clearTimeout(gpRewardTimerId);
  gpRewardTimerId = setTimeout(async () => {
    try {
      status.textContent = "正在確認 GP 加成…";
      const r = await invoke("managed_ai_gp_reward_cmd");
      if (r && r.ok === true) {
        status.textContent = "已領取 GP 加成：個人今日總額度 +50 萬（上限 100 萬）。";
        await refreshManagedAiUsageIndicator({ force: true }).catch(() => null);
        try {
          localStorage.setItem(GP_REWARD_STORAGE_KEY, String(Date.now()));
        } catch (_) {}
        setGpRewardPromptVisible(false);
      } else if (r && r.alreadyClaimed === true) {
        status.textContent = "這個帳號已領過 GP 加成，不用再重送。";
        setGpRewardPromptVisible(false);
      } else {
        status.textContent = "GP 確認完成，但加成未成功領取。可稍後再試。";
      }
    } catch (_) {
      status.textContent = "GP 確認失敗。可稍後再試。";
    } finally {
      gpRewardInFlight = false;
      btn.disabled = false;
    }
  }, 60000);
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
  if (onboardActive || progressBusy || shareUploadInFlight || managedAiPaused) return true;
  for (const id of ["guide-overlay", "update-overlay", "feedback-overlay", "managed-ai-error-overlay"]) {
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

    if (r && r.errorType) {
      if (status) status.textContent = "送出失敗：" + String(r.errorType);
    } else {
      if (status) status.textContent = "送出失敗，請稍後再試。";
    }

    try {
      localStorage.setItem(USAGE_FEEDBACK_LAST_NUDGE_AT_KEY, String(Date.now()));
    } catch (_) {}
  } catch (e) {
    if (status) status.textContent = "送出失敗：" + formatInvokeError(e);
  }
}

/** 右側五步：檢查 → 搜尋 → 翻譯 → 補充 → 套用 */
const STEP_ORDER = ["prep", "scan", "translate", "supplement", "done"];
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

function buildStepMeta(payload, stepKey) {
  if (!payload) return "";
  const parts = [];
  const done = payload.done != null ? Number(payload.done) : null;
  const total = payload.total != null ? Number(payload.total) : null;
  const unit = (payload.unit || "").trim();
  let denominatorRaised = false;
  if (stepKey && total != null) {
    denominatorRaised =
      stepKey === lastActiveStepKey &&
      lastActiveStepTotal != null &&
      Number.isFinite(total) &&
      total > lastActiveStepTotal;
    lastActiveStepKey = stepKey;
    lastActiveStepTotal = total;
  } else if (stepKey && stepKey !== lastActiveStepKey) {
    lastActiveStepKey = stepKey;
    lastActiveStepTotal = total;
  }
  if (done != null && total != null) {
    parts.push(
      `${formatCount(Math.max(0, done))} / ${formatCount(Math.max(0, total))}${unit ? ` ${unit}` : ""}`
    );
  }
  if (denominatorRaised) parts.push("分母更新");
  // 不含 payload.detail（AI 長 metrics 只進命中格）
  return parts.filter(Boolean).join(" · ");
}

function updateLinearSteps(percent, message, failed, payload) {
  const root = $("linear-steps");
  if (!root) return;
  const items = root.querySelectorAll(".lin-step");
  const currentKey = resolveStepKey(payload, percent, message);
  let idx = currentKey ? STEP_ORDER.indexOf(currentKey) : -1;
  if (failed) {
    items.forEach((el) => {
      el.classList.remove("active", "done", "error");
      const si = STEP_ORDER.indexOf(el.getAttribute("data-step"));
      if (idx >= 0 && si < idx) el.classList.add("done");
      else if (si === idx) el.classList.add("error");
      const meta = el.querySelector("[data-step-meta]");
      if (meta) {
        const text = si === idx ? buildStepMeta(payload, currentKey) : "";
        meta.textContent = text;
        meta.hidden = !text;
      }
    });
    return;
  }
  const p = Number(percent) || 0;
  if (p <= 0) {
    lastStepIdx = -1;
    lastActiveStepKey = null;
    lastActiveStepTotal = null;
  }
  // 單調前進：翻譯中不因文案回退步驟
  if (idx >= 0 && lastStepIdx >= 0 && idx < lastStepIdx && p > 0 && p < 100 && currentKey !== "done") {
    idx = lastStepIdx;
  }
  if (idx > lastStepIdx) lastStepIdx = idx;
  if (currentKey === "done") lastStepIdx = STEP_ORDER.length - 1;
  items.forEach((el) => {
    el.classList.remove("active", "done", "error");
    const si = STEP_ORDER.indexOf(el.getAttribute("data-step"));
    if (idx < 0) {
      const meta = el.querySelector("[data-step-meta]");
      if (meta) meta.hidden = true;
      return;
    }
    if (si < idx) el.classList.add("done");
    else if (si === idx) el.classList.add(currentKey === "done" ? "done" : "active");
    const meta = el.querySelector("[data-step-meta]");
    if (meta) {
      const text = si === idx ? buildStepMeta(payload, currentKey) : "";
      meta.textContent = text;
      meta.hidden = !text;
    }
  });
  if (currentKey === "done") {
    items.forEach((el) => {
      el.classList.remove("active");
      el.classList.add("done");
      const meta = el.querySelector("[data-step-meta]");
      if (meta) meta.hidden = true;
    });
  }
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
    // 百分比久不動：微幅前進（不超過真實值 +4，且 <99）
    if (stuckMs > 1500 && lastRealPercent < 99 && lastRealPercent > 0) {
      const crawl = Math.min(0.35, stuckMs / 20000);
      displayPercent = Math.min(
        99,
        Math.max(displayPercent, lastRealPercent) + crawl,
        lastRealPercent + 4
      );
      const fill = $("prog-fill");
      if (fill) fill.style.width = displayPercent.toFixed(1) + "%";
      const pctEl = $("prog-pct");
      if (pctEl) pctEl.textContent = Math.floor(displayPercent) + "%";
    }
    const baseMsg = lastRealMessage || "處理中…";
    const msgEl = $("prog-msg");
    if (msgEl) {
      if (stuckMs > 2000) {
        msgEl.textContent = baseMsg + " · 已進行 " + elapsed + " · 仍在運作，請稍候";
        setProgBarWorking(true);
      } else {
        msgEl.textContent = baseMsg + " · 已進行 " + elapsed;
        setProgBarWorking(false);
      }
    }
  }, 1000);
}

/** 正規化進度訊息供日誌去重（去掉易變秒數） */
function progressLogDedupeKey(percent, message) {
  const msg = String(message || "")
    .replace(/本輪\s*\d+\s*秒/g, "本輪*秒")
    .replace(/合計\s*\d+\s*秒/g, "合計*秒")
    .replace(/已進行\s*\d+\s*秒/g, "已進行*秒")
    .replace(/預估剩餘[^·]*/g, "")
    .replace(/\s+/g, " ")
    .trim();
  return Math.floor(Number(percent) || 0) + "|" + msg;
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
  setDirect("pending", m.pending);
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

function shortenProgressMessage(message) {
  const m = String(message || "").trim();
  if (!m) return m;
  if (/等待 Discord|重新登入 Discord|登入已恢復|重新載入 Discord/.test(m)) {
    return m.length > 80 ? m.slice(0, 77) + "…" : m;
  }
  if (m.startsWith("AI 翻譯中…等待本輪回應")) return "AI 翻譯中 · 等待本輪回應";
  if (m.startsWith("AI 翻譯中…")) return "AI 翻譯中";
  if (m.startsWith("AI 限流：")) return "AI 已降速（限流）";
  if (m.startsWith("AI 這一輪沒有新譯文")) return "AI 本輪暫無新譯文";
  if (m.startsWith("AI 有") && m.includes("批失敗")) return "AI 部分批次失敗，將重送未完成";
  return m;
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

function setDiagnoseProgress(percent, message, opts) {
  const p = Math.max(0, Math.min(100, Number(percent) || 0));
  const fill = $("diagnose-prog-fill");
  const pctEl = $("diagnose-prog-pct");
  const msgEl = $("diagnose-prog-msg");
  const countEl = $("diagnose-prog-count");
  if (fill) fill.style.width = (p >= 100 ? 100 : p) + "%";
  if (pctEl) pctEl.textContent = Math.floor(p >= 100 ? 100 : p) + "%";
  if (countEl) countEl.textContent = p > 0 ? Math.floor(p) + "% 分析中" : "尚未分析";
  if (msgEl && message) msgEl.textContent = message;
  const activeStep = p >= 100 ? "result" : p > 0 ? "analyze" : "input";
  document.querySelectorAll("#diagnose-linear-steps .lin-step").forEach((el) => {
    const key = el.getAttribute("data-step");
    const order = ["input", "analyze", "result"];
    const idx = order.indexOf(key);
    const activeIdx = order.indexOf(activeStep);
    el.classList.toggle("active", idx === activeIdx && p < 100);
    el.classList.toggle("done", idx >= 0 && idx < activeIdx);
  });
  if (message && !(opts && opts.skipLog)) appendDiagnoseLog(Math.floor(p) + "%  " + message);
}

function setProgress(percent, message, opts) {
  const job = window.__busyJobKind || document.body.dataset.appPage || "translate";
  if (job === "font") {
    setFontProgress(percent, message, opts);
    return;
  }
  if (job === "diagnose") {
    setDiagnoseProgress(percent, message, opts);
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
  // 詳細完整日誌：去重（AI 等待秒數變化不重寫）
  if (message && !(opts && opts.skipLog)) {
    const key = progressLogDedupeKey(p, message);
    if (key !== lastProgressLogKey) {
      lastProgressLogKey = key;
      appendLog(Math.floor(p) + "%  " + message);
    }
  }
}

function setBusy(busy, jobKind) {
  progressBusy = !!busy;
  const kind = busy ? (jobKind || "translate") : null;
  window.__busyJobKind = kind;
  if (busy) {
    // 忙碌時不讓更新視窗持續干擾：延遲顯示給完成後處理。
    try {
      const overlay = document.getElementById("update-overlay");
      if (overlay && !overlay.hidden) {
        if (typeof window.zfUpdateModalSetPending === "function") window.zfUpdateModalSetPending(window.__mcpl_latestUpdateInfo || null);
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
  const diagnoseStop = $("btn-diagnose-stop");
  if (stop) {
    stop.hidden = !busy || kind !== "translate";
    stop.disabled = false;
    stop.textContent = "停止翻譯";
  }
  if (fontStop) fontStop.hidden = !busy || kind !== "font";
  if (diagnoseStop) diagnoseStop.hidden = !busy || kind !== "diagnose";
  // 仍鎖定：開第二個重任務、改路徑、連線設定等
  const hardLockIds = [
    "btn-run",
    "btn-supplement",
    "btn-repair",
    "btn-delete-backups",
    "btn-output-pick",
    "btn-delete-output",
    "btn-inst",
    "btn-save-adv",
    "btn-test-api",
    "use-ai",
    "backup-before-apply",
    "force-refresh",
    "api-provider",
    "api-key",
    "base-url",
    "api-model",
    "choose-output-dir",
    "ai-source-managed",
    "ai-source-custom",
    "btn-discord-login",
    "btn-discord-join",
    "btn-discord-refresh",
    "btn-discord-logout",
    "btn-turnstile-verify",
    "btn-turnstile-open",
    "btn-turnstile-cancel",
    "btn-open-login-url",
    "btn-copy-login-url",
    "btn-cancel-login",
    "btn-font-pick",
    "btn-font-out",
    "btn-font-build",
    "btn-reference-pick",
    "btn-reference-file",
    "btn-share-confirm",
    "btn-share-cancel",
    "font-size",
    "font-weight",
    "font-shift-x",
    "font-shift-y",
    "font-oversample",
    "font-apply-current",
    "btn-glossary",
    "target-version",
    "btn-diagnose",
    "btn-diagnose-latest",
    "btn-diagnose-report",
    "error-input",
    "diagnose-pack-path",
    "report-category",
    "report-pack-name",
    "report-note",
    "report-unrelated",
    "btn-helper-prepare",
    "btn-helper-rescan",
    "btn-helper-cleanup",
    "helper-ack-ingame",
  ];
  hardLockIds.forEach((id) => {
    const el = $(id);
    if (el) el.disabled = busy;
  });

  // 更新相關按鈕：翻譯中避免誤觸（「稍後」仍可關閉視窗）。
  const updateCheckBtn = $("btn-check-update");
  if (updateCheckBtn) updateCheckBtn.disabled = busy;
  const updateManualBtn = $("btn-update-manual");
  if (updateManualBtn) updateManualBtn.disabled = busy;
  const updateNowBtn = $("btn-update-now");
  if (updateNowBtn) updateNowBtn.disabled = busy;
  const updateLaterBtn = $("btn-update-close");
  if (updateLaterBtn) updateLaterBtn.disabled = false;

  // 忙碌中仍可切分頁瀏覽（開始鈕另由 syncUiState 看 progressBusy）
  ["tab-translate", "tab-font", "tab-diagnose"].forEach((id) => {
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
  const guide = $("btn-guide");
  if (guide) guide.disabled = false;
  if (stop) stop.disabled = false;
  syncOutputField();
  syncUiState();
  if (!busy && typeof window.zfUpdateModalMaybeShowPending === "function") window.zfUpdateModalMaybeShowPending();
  if (!busy && pendingUsageFeedbackNudge && translationState === "complete" && !isBlockingOverlayOpen()) {
    pendingUsageFeedbackNudge = false;
    scheduleUsageFeedbackNudge();
  }
}

/** 翻譯開始：不再收合主介面／最小化；僅靠 setBusy 鎖定操控 */
async function hideUiForTranslateRun() {
  /* no-op：主區保持可見 */
}

/** 分頁：translate | font | diagnose */
function showAppPage(page, opts = {}) {
  const name = page === "font" || page === "diagnose" ? page : "translate";
  const previous = document.body.dataset.appPage || "translate";
  const changed = previous !== name;
  const pageTr = $("page-translate");
  const pageFont = $("page-font");
  const pageDiagnose = $("page-diagnose");
  const tabTr = $("tab-translate");
  const tabFont = $("tab-font");
  const tabDiagnose = $("tab-diagnose");
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
  if (pageDiagnose) {
    pageDiagnose.hidden = name !== "diagnose";
    pageDiagnose.classList.toggle("active", name === "diagnose");
  }
  if (tabDiagnose) {
    tabDiagnose.classList.toggle("active", name === "diagnose");
    tabDiagnose.setAttribute("aria-selected", name === "diagnose" ? "true" : "false");
  }
  document.body.dataset.appPage = name;
  syncStatusRail(name);
  syncUiState();
  const activePanel = name === "font" ? pageFont : name === "diagnose" ? pageDiagnose : pageTr;
  if (changed && !opts.skipTransition) revealPagePanel(activePanel);
}

function syncStatusRail(page) {
  const name = page === "font" || page === "diagnose" ? page : "translate";
  document.querySelectorAll(".rail-panel").forEach((el) => {
    const rail = el.getAttribute("data-rail") || "translate";
    el.hidden = rail !== name;
  });
}

function setTranslationState(state) {
  translationState = ["idle", "ready", "running", "complete", "failed"].includes(state)
    ? state
    : "idle";
  document.body.dataset.translationState = translationState;
  syncUiState();
  if (translationState === "complete" || translationState === "failed") {
    refreshShareableState();
  }
  if (translationState === "complete") {
    scheduleUsageFeedbackNudge();
  }
}

function toggleHidden(id, hidden) {
  const el = $(id);
  if (!el) return;
  el.hidden = !!hidden;
}

function isMoreDrawerOpen() {
  const drawer = $("more-options");
  return !!(drawer && !drawer.hidden);
}

function openMoreDrawer() {
  const drawer = $("more-options");
  const backdrop = $("more-options-backdrop");
  const btn = $("btn-more-options");
  if (!drawer) return;
  drawer.hidden = false;
  if (backdrop) backdrop.hidden = false;
  if (btn) btn.setAttribute("aria-expanded", "true");
  document.body.classList.add("drawer-open");
  setOverflowMenuOpen(false);
}

function closeMoreDrawer() {
  const drawer = $("more-options");
  const backdrop = $("more-options-backdrop");
  const btn = $("btn-more-options");
  if (drawer) drawer.hidden = true;
  if (backdrop) backdrop.hidden = true;
  if (btn) btn.setAttribute("aria-expanded", "false");
  document.body.classList.remove("drawer-open");
}

function setOverflowMenuOpen(open) {
  const menu = $("overflow-menu");
  const btn = $("btn-overflow");
  if (!menu || !btn) return;
  menu.hidden = !open;
  btn.setAttribute("aria-expanded", open ? "true" : "false");
}

function wireShellChrome() {
  initWinbarChrome();
  const moreBtn = $("btn-more-options");
  if (moreBtn) {
    moreBtn.setAttribute("aria-expanded", "false");
    moreBtn.setAttribute("aria-controls", "more-options");
    moreBtn.onclick = () => {
      if (isMoreDrawerOpen()) closeMoreDrawer();
      else openMoreDrawer();
    };
  }
  if ($("btn-more-close")) $("btn-more-close").onclick = () => closeMoreDrawer();
  if ($("more-options-backdrop")) {
    $("more-options-backdrop").onclick = () => closeMoreDrawer();
  }

  const overflowBtn = $("btn-overflow");
  if (overflowBtn) {
    overflowBtn.onclick = (ev) => {
      ev.stopPropagation();
      const menu = $("overflow-menu");
      setOverflowMenuOpen(!!(menu && menu.hidden));
    };
  }
  document.addEventListener("click", (ev) => {
    const wrap = document.querySelector(".overflow-wrap");
    if (!wrap || wrap.contains(ev.target)) return;
    setOverflowMenuOpen(false);
  });
  window.addEventListener("keydown", (ev) => {
    if (ev.key !== "Escape") return;
    if (isMoreDrawerOpen()) {
      closeMoreDrawer();
      return;
    }
    setOverflowMenuOpen(false);
  });

  // 0.2.4：移除 0.2.2/0.2.3 的 selectstart／dblclick／drag 攔截實驗（曾與骨架蓋層疊加導致無法點）
  // 選取防護只留 CSS user-select；不以 JS 攔事件。
}

function syncUiState() {
  const hasInstance = !!($("instance")?.value || "").trim();
  const hasOutput = !!selectedOutputDir();
  const complete = translationState === "complete";
  const failed = translationState === "failed";
  const locked = progressBusy || shareUploadInFlight;
  const page = document.body.dataset.appPage || "translate";
  const instanceReady = hasInstance && !!instanceValidation.ok && !versionBlocked;
  document.body.dataset.instanceReady = instanceReady ? "1" : "0";

  const hideMore = !(hasInstance && !!instanceValidation.ok);
  const moreBtn = $("btn-more-options");
  if (moreBtn) moreBtn.hidden = hideMore;
  if (hideMore) closeMoreDrawer();
  ["field-output", "pack-version-group", "translation-method-group", "reference-details"]
    .forEach((id) => toggleHidden(id, !(hasInstance && !!instanceValidation.ok)));
  const gateHint = $("path-gate-hint");
  if (gateHint) {
    gateHint.hidden = (hasInstance && !!instanceValidation.ok) || page !== "translate";
  }
  const aiGroup = $("ai-options-group");
  if (aiGroup) {
    const showAi = hasInstance && !!instanceValidation.ok;
    aiGroup.hidden = !showAi;
    aiGroup.setAttribute("aria-hidden", showAi ? "false" : "true");
  }
  const primaryAction = document.querySelector(".primary-action");
  if (primaryAction) primaryAction.hidden = !(hasInstance && !!instanceValidation.ok);
  const runDock = document.querySelector(".run-dock");
  if (runDock) {
    runDock.hidden = page === "translate" && !(hasInstance && !!instanceValidation.ok);
  }
  const runBtn = $("btn-run");
  if (runBtn) {
    runBtn.hidden = progressBusy || !instanceReady;
    runBtn.disabled = !instanceReady || progressBusy;
    runBtn.title = !hasInstance
      ? "請先選擇遊戲資料夾"
      : !instanceValidation.ok
        ? instanceValidation.reason || "實例檢查未通過"
        : versionBlocked
          ? versionBlockReason || "Minecraft 版本過舊，無法翻譯"
          : "";
  }
  toggleHidden("btn-supplement", !complete || locked);
  toggleHidden("btn-repair", !failed || locked);
  toggleHidden("btn-glossary", !instanceReady || locked);
  const canShare = hasShareableFiles && !locked;
  toggleHidden("btn-package", !canShare);
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
  const diagnosePackPath = ($("diagnose-pack-path")?.value || "").trim();
  toggleHidden("btn-diagnose-latest", !(hasInstance || diagnosePackPath) || locked);
  toggleHidden("btn-restore", !(hasInstance || diagnosePackPath) || !hasApplyBackups || locked);
  toggleHidden("btn-delete-backups", !(hasInstance || diagnosePackPath) || !hasApplyBackups || locked);

  syncTranslationHelperPanel();

  const packageButton = $("btn-package");
  if (packageButton) {
    packageButton.disabled = !canShare;
    packageButton.textContent = lastShareUrl ? "複製分享連結" : "分享給其他玩家";
  }
  const shareHint = $("share-hint");
  if (shareHint) {
    shareHint.hidden = !canShare;
    if (canShare && lastShareUrl) {
      shareHint.textContent = "連結已在剪貼簿；再按一次只會複製，不會寫進日誌。連結 24 小時有效。";
    } else if (canShare && translationState === "complete") {
      shareHint.textContent =
        "翻譯已完成：可按上方按鈕分享帶密碼自解檔。成功後改為「複製分享連結」，網址不會寫進日誌。";
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
  const instancePath =
    ($("instance")?.value || "").trim() || ($("diagnose-pack-path")?.value || "").trim();
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

async function pickDir(title) {
  if (!dialog.open) throw new Error("無法開啟資料夾選擇視窗");
  const selected = await dialog.open({ directory: true, multiple: false, title });
  return typeof selected === "string" ? selected : null;
}

/** 將已完成的翻譯結果上傳到獨立分享區，連結只保留一天。 */
function clearShareUrlIfInstanceChanged() {
  const path = ($("instance")?.value || "").trim();
  if (lastShareUrl && path !== lastShareInstancePath) {
    lastShareUrl = "";
  }
}

async function copyShareUrl(url) {
  try {
    await navigator.clipboard.writeText(url);
    appendLog("連結已複製（24 小時有效）");
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
  const outputDir = selectedOutputDir();
  if (!outputDir) return log("請先完成翻譯（還沒有可打包的翻譯結果）。");
  const work = resultWorkDir(outputDir);
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

function aiModeFromUi() {
  return $("ai-source-custom")?.checked ? "custom" : "managed";
}

function syncAiModeUi(mode) {
  const normalized = mode === "custom" ? "custom" : "managed";
  if ($("ai-source-managed")) $("ai-source-managed").checked = normalized === "managed";
  if ($("ai-source-custom")) $("ai-source-custom").checked = normalized === "custom";
  if ($("managed-auth-panel")) $("managed-auth-panel").hidden = normalized !== "managed";
  if ($("adv-details")) $("adv-details").hidden = normalized !== "custom";
  if ($("btn-test-api")) $("btn-test-api").hidden = normalized !== "custom";
  if ($("api-test-status") && normalized !== "custom") $("api-test-status").textContent = "";
}

async function refreshAiStatus() {
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
    const message = String(s && s.message ? s.message : "").trim();
    const managedIdentityReady = !!(
      s &&
      (s.loggedIn || s.logged_in) &&
      (s.inGuild || s.in_guild) &&
      (s.serviceAvailable ?? s.service_available) !== false
    );
    syncAiModeUi(mode);
    statusEl.textContent = ready
      ? usingOwnKey
        ? "AI：自訂 API 可用"
        : "AI：免費代管可用"
      : mode === "custom"
        ? "AI：請先設定自訂 API"
        : "AI：尚未完成 Discord 驗證";
    if (statusRow) statusRow.dataset.state = ready ? (usingOwnKey ? "own" : "managed") : "error";

    if (mode === "managed") {
      const loggedIn = !!(s && (s.loggedIn || s.logged_in));
      const inGuild = !!(s && (s.inGuild || s.in_guild));
      const serviceAvailable = s && (s.serviceAvailable ?? s.service_available) !== false;
      const displayName = String((s && (s.displayName || s.display_name)) || "").trim();
      const title = $("discord-auth-title");
      const authNote = $("discord-auth-note");
      if (title) {
        title.textContent = ready
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
          ? message || "請檢查網路後按「重新檢查」。"
          : message || "登入 Discord 並加入官方伺服器後即可使用。";
      }
      if ($("btn-discord-login")) $("btn-discord-login").hidden = loggedIn;
      if ($("btn-discord-logout")) $("btn-discord-logout").hidden = !loggedIn;
      if ($("btn-discord-join")) $("btn-discord-join").hidden = inGuild;
      if (noteEl) {
        noteEl.textContent = ready
          ? "代管翻譯由開發者個人提供，不是無限額度。額度用盡時代管不可用；共享庫與本機轉換仍可用。線上翻譯可能有錯，歡迎診斷回報。"
          : message || "請先登入 Discord 並加入 ZeitFrei 官方伺服器。";
      }
      const gpAlreadySent = (() => {
        try {
          return !!localStorage.getItem(GP_REWARD_STORAGE_KEY);
        } catch (_) {
          return false;
        }
      })();
      const canShow = !!ready && !usingOwnKey && !managedAiPaused;
      if (canShow) {
        void refreshManagedAiUsageIndicator();
        setGpRewardPromptVisible(!gpAlreadySent && !gpRewardInFlight);
      } else {
        setGpRewardPromptVisible(false);
        const usage = $("managed-ai-usage-indicator");
        if (usage) {
          usage.hidden = true;
          usage.setAttribute("aria-hidden", "true");
        }
      }
    } else if (noteEl) {
      noteEl.textContent =
        message ||
        "推薦使用 DeepSeek（便宜划算）。到 platform.deepseek.com 申請金鑰後選 DeepSeek 並填入即可；不需 Discord。";
    }
    return s;
  } catch (e) {
    latestAiStatus = null;
    const detail = formatInvokeError(e);
    statusEl.textContent = "AI：狀態確認失敗";
    if (statusRow) statusRow.dataset.state = "error";
    if (noteEl) {
      noteEl.textContent =
        "無法讀取 AI 狀態：" + detail + "（本機簡繁轉換仍可用；需要 AI 時請檢查網路／Worker 後重試）";
    }
    return null;
  }
}

async function refreshApiSettings() {
  try {
    const s = await invoke("get_api_settings");
    initApiKeyMask();
    setApiKeyMask(String(s.keyMasked || s.key_masked || ""));
    syncAiModeUi(String(s.aiMode || s.ai_mode || "managed"));
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
      note.textContent = "請再填寫 Base URL 與模型名稱；一般使用者不需要改這些設定。";
    } else if (normalized === "glm") {
      note.textContent = "只要填 API Key，工具會自動使用智譜 GLM 的官方設定。";
    } else if (normalized === "openai") {
      note.textContent = "只要填 API Key，工具會自動使用 OpenAI 的官方設定。";
    } else if (normalized === "qwen") {
      note.textContent = "只要填 API Key，工具會自動使用通義千問的官方設定。";
    } else {
      note.innerHTML =
        '推薦：便宜划算（官方 deepseek-v4-flash、非思考模式）。只要填 API Key；金鑰申請 <a href="https://platform.deepseek.com" class="inline-ext-link" data-url="https://platform.deepseek.com">platform.deepseek.com</a>';
      note.querySelector("a.inline-ext-link")?.addEventListener("click", (e) => {
        e.preventDefault();
        const url = e.currentTarget.getAttribute("data-url");
        openExternalUrl(url);
      });
    }
  }
}

async function changeAiMode(mode) {
  syncAiModeUi(mode);
  try {
    await invoke("set_ai_mode_cmd", { aiMode: mode });
  } catch (e) {
    appendError("無法切換 AI 來源：" + formatInvokeError(e));
  }
  return refreshAiStatus();
}

async function ensureAiReadyForAction() {
  await aiModeChangePromise;
  let status = await refreshAiStatus();
  if (status && status.ready !== false) return true;
  const mode = String((status && (status.aiMode || status.ai_mode)) || aiModeFromUi());
  const message = String((status && status.message) || "目前無法確認 AI 狀態。");
  appendLog(message, "warn");
  if (mode === "custom") {
    $("api-key")?.focus();
  } else {
    $("managed-auth-panel")?.scrollIntoView({ behavior: "smooth", block: "nearest" });
    if ($("ai-auth-details")) $("ai-auth-details").open = true;
  }
  return false;
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
    document.body.appendChild(el);
  }
  el.textContent = String(message || "");
  el.classList.add("show");
  if (_appToastTimer) clearTimeout(_appToastTimer);
  _appToastTimer = setTimeout(() => {
    el.classList.remove("show");
  }, ms);
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
  try {
    const result = await invoke("discord_login");
    if (result && result.ok) {
      if (fallback) fallback.hidden = true;
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

async function beginTurnstileVerification() {
  const verifyButton = $("btn-turnstile-verify");
  const openButton = $("btn-turnstile-open");
  const cancelButton = $("btn-turnstile-cancel");
  if (verifyButton) verifyButton.disabled = true;
  if (openButton) openButton.hidden = true;
  if (cancelButton) cancelButton.hidden = false;
  if ($("turnstile-auth-title")) $("turnstile-auth-title").textContent = "等待 Cloudflare 驗證";
  if ($("turnstile-auth-note")) $("turnstile-auth-note").textContent = "請在瀏覽器完成驗證，再回到工具。";
  try {
    const result = await invoke("turnstile_verify");
    if (result && result.ok) {
      turnstileUrl = "";
      appendLog("Cloudflare 安全驗證完成。");
      return true;
    }
    const reason = String((result && result.error) || "驗證未完成");
    const message = reason === "cancelled"
      ? "已取消安全驗證。"
      : reason === "timeout"
        ? "安全驗證逾時，請重新驗證。"
        : reason === "browser_open_failed"
          ? "瀏覽器沒有自動開啟，請按「重新開啟驗證頁」。"
          : "安全驗證未完成：" + reason;
    appendLog(message, "warn");
    return false;
  } catch (e) {
    appendError("Cloudflare 安全驗證失敗：" + formatInvokeError(e));
    return false;
  } finally {
    if (verifyButton) verifyButton.disabled = false;
    if (cancelButton) cancelButton.hidden = true;
    if (openButton) openButton.hidden = !turnstileUrl;
    await refreshAiStatus();
  }
}

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
    const reason = String((result && result.reason) || "").trim() || (ok ? "實例可用。" : "實例檢查未通過。");
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
        "整合包版本尚未偵測，完成翻譯時會使用 R1。可翻譯前自訂名稱；留空則系統產生。";
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
    status.textContent = "尚未指定參考翻譯；可手動選本機繁中／社群漢化資料夾或 zip，或略過。";
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

async function onRun() {
  const instancePath = ($("instance").value || "").trim();
  let outputDir = selectedOutputDir();
  if (!instancePath) return log("請先選擇「遊戲資料夾」。");
  if (!(await validateSelectedInstance(instancePath))) {
    return log(instanceValidation.reason || "實例檢查未通過，無法開始翻譯。");
  }
  if (!outputDir || isLegacySharedWorkPath(outputDir)) {
    outputDir =
      (await invoke("managed_output_for_instance", { instancePath }).catch(() => "")) || "";
    if (outputDir) {
      setAutoOutputDir(outputDir);
      appendLog("此整合包專用結果位置：\n" + outputDir);
    }
  }
  if (!outputDir) return log("翻譯結果位置還沒準備好，請重新選擇遊戲資料夾。");

  const useAi = !!$("use-ai").checked;
  if (useAi && !(await ensureAiReadyForAction())) return;
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

  setBusy(true, "translate");
  lastStepIdx = -1;
  setTranslationState("running");
  lastProgressLogKey = "";
  clearLog("開始翻譯");
  resetCoverageMetrics("翻譯統計蒐集中");
  if ($("btn-package")) $("btn-package").disabled = true;
  appendLog(
    shouldBackupBeforeApply()
      ? "翻譯完成後會先備份，再直接套用到這個遊戲實例。"
      : "翻譯完成後會直接套用，不建立備份。"
  );
  setProgress(1, "準備中…");
  void hideUiForTranslateRun();
  await paintBeforeInvoke();

  try {
    const result = await invoke("one_click_translate", {
      instancePath,
      outputDir,
      packName: packNameForTranslate(),
      useAi,
      backupBeforeApply: shouldBackupBeforeApply(),
      referencePack: (($('reference-pack')?.value || "").trim() || null),
      targetVersion: targetVersion || null,
      coverageTier: "max",
    });
    setProgress(100, "全部完成！");
    let msg = result.playerSummary || result.player_summary || JSON.stringify(result, null, 2);
    if (result.minemenuMsg || result.minemenu_msg) {
      msg += "\n\n" + (result.minemenuMsg || result.minemenu_msg);
    }
    consumeCoverageMessage(msg);
    setLogFinal(msg);
    setTranslationState("complete");
    appendLog("翻譯已完成並直接套用。想分享給其他玩家時，再按「分享給其他玩家」。");
    await cleanupPreparedTranslationHelper();
  } catch (e) {
    if (isCancellation(e)) {
      setTranslationState("idle");
      appendLog("已停止。先前完成的部分仍保留；有效譯文會盡量上傳共享庫（已上傳過的不會重複灌庫）。", "warn");
    } else {
      const cls = classifyManagedAiError(e);
      if (cls) {
        setTranslationState("idle");
        setProgress(Math.max(lastRealPercent, Math.floor(displayPercent) || 0), "已暫停（代管 AI 需要確認）");
        showManagedAiErrorModal(cls);
        appendLog("代管 AI 已暫停。依彈窗指引處理後再重試。", "warn");
        return;
      }
      setTranslationState("failed");
    }
    handleRunFailure(e, "翻譯失敗");
    if (!isCancellation(e)) {
      appendLog("可把上方錯誤訊息留下來方便排查。");
    }
  } finally {
    setBusy(false);
    refreshBackupState();
  }
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

function supplementTranslationMode() {
  return $("force-refresh")?.checked ? "force" : null;
}

/** 修復：重建 zip／對齊工作階段；可選一併 AI 補缺。不修世界閃退。 */
async function onRepair() {
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

  try {
    const result = await invoke("repair_translation_pack", {
      outputDir,
      useAi,
      backupBeforeApply: shouldBackupBeforeApply(),
      translationMode: supplementTranslationMode(),
    });
    setProgress(100, "修復完成！");
    setLogFinal(result.playerSummary || result.player_summary || JSON.stringify(result, null, 2));
    setTranslationState("complete");
  } catch (e) {
    if (isCancellation(e)) {
      setTranslationState("idle");
      appendLog("已停止。先前完成的部分仍保留；有效譯文會盡量上傳共享庫（已上傳過的不會重複灌庫）。", "warn");
    } else {
      const cls = classifyManagedAiError(e);
      if (cls) {
        setTranslationState("idle");
        setProgress(Math.max(lastRealPercent, Math.floor(displayPercent) || 0), "已暫停（代管 AI 需要確認）");
        showManagedAiErrorModal(cls);
        appendLog("代管 AI 已暫停。依彈窗指引處理後再重試。", "warn");
        return;
      }
      setTranslationState("failed");
    }
    handleRunFailure(e, "修復失敗");
  } finally {
    setBusy(false);
    refreshBackupState();
  }
}

/** 只補缺漏：不重掃 mods，讀工作階段 + AI */
async function onSupplement() {
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
  resetCoverageMetrics("補翻統計蒐集中");
  setProgress(3, "準備中…");
  void hideUiForTranslateRun();
  await paintBeforeInvoke();

  try {
    const result = await invoke("supplement_translate", {
      outputDir,
      useAi,
      backupBeforeApply: shouldBackupBeforeApply(),
      translationMode: supplementTranslationMode(),
    });
    setProgress(100, "補譯完成！");
    let msg = result.playerSummary || result.player_summary || JSON.stringify(result, null, 2);
    consumeCoverageMessage(msg);
    setLogFinal(msg);
    setTranslationState("complete");
    appendLog("複查完成，結果已重新套用到遊戲。", "info");
    await cleanupPreparedTranslationHelper();
  } catch (e) {
    if (isCancellation(e)) {
      setTranslationState("idle");
      appendLog("已停止。先前完成的部分仍保留；有效譯文會盡量上傳共享庫（已上傳過的不會重複灌庫）。", "warn");
    } else {
      const cls = classifyManagedAiError(e);
      if (cls) {
        setTranslationState("idle");
        setProgress(Math.max(lastRealPercent, Math.floor(displayPercent) || 0), "已暫停（代管 AI 需要確認）");
        showManagedAiErrorModal(cls);
        appendLog("代管 AI 已暫停。依彈窗指引處理後再重試。", "warn");
        return;
      }
      setTranslationState("failed");
    }
    handleRunFailure(e, "再補一些失敗");
  } finally {
    setBusy(false);
    refreshBackupState();
  }
}

/** 停止：後端在下一個檢查點乾淨收尾，已完成的檔案保留 */
async function onStop() {
  const btn = $("btn-stop");
  if (btn) {
    btn.disabled = true;
    btn.textContent = "停止中…";
  }
  setProgressStateBadge("cancelling");
  appendLog(
    "已要求停止，等目前這一步做完就會收尾；有效譯文會上傳共享庫，已上傳過的不會重複灌庫。",
    "warn"
  );
  try {
    await invoke("cancel_task");
  } catch (e) {
    appendError("無法送出停止要求：" + formatInvokeError(e));
    if (btn) {
      btn.disabled = false;
      btn.textContent = "停止";
    }
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

async function submitDiagnoseReport() {
  const category = ($("report-category")?.value || "").trim();
  const unrelated = !!$("report-unrelated")?.checked;
  const packName = ($("report-pack-name")?.value || "").trim();
  const note = ($("report-note")?.value || "").trim();
  const status = $("report-status");
  if (!category) {
    if (status) status.textContent = "請先選類別。";
    return appendDiagnoseLog("請先選擇回報類別。");
  }
  if (!unrelated && !packName) {
    if (status) status.textContent = "請填整合包名稱，或勾選與包無關。";
    return appendDiagnoseLog("請填整合包名稱，或勾選「與包無關」。");
  }
  const instancePath =
    ($("diagnose-pack-path")?.value || "").trim() || ($("instance")?.value || "").trim();
  const diagnosis = lastDiagnosisResult
    ? JSON.stringify({
        verdict: lastDiagnosisResult.verdict,
        summary: lastDiagnosisResult.summary,
        errorCode: lastDiagnosisResult.errorCode || lastDiagnosisResult.error_code,
        confidence: lastDiagnosisResult.confidence,
        nextSteps: lastDiagnosisResult.nextSteps || lastDiagnosisResult.next_steps,
      })
    : "";
  if (status) status.textContent = "正在打包上傳…";
  try {
    const result = await invoke("submit_diagnose_report_cmd", {
      request: {
        reportCategory: category,
        packName: unrelated ? "" : packName,
        packUnrelated: unrelated,
        packVersion: "",
        errorCode: lastDiagnosisResult?.errorCode || lastDiagnosisResult?.error_code || "",
        userNote: note,
        instancePath: instancePath || null,
        outputDir: selectedOutputDir() || null,
        diagnosisJson: diagnosis || null,
      },
    });
    const msg = result.message || result.playerSummary || "已送出（資料 3 天內刪除）。";
    if (status) status.textContent = msg;
    appendDiagnoseLog(msg, "info");
  } catch (e) {
    const err = formatInvokeError(e);
    if (status) status.textContent = err;
    appendDiagnoseLog("診斷回報失敗：" + err, "error");
  }
}

function showDiagnosis(result) {
  lastDiagnosisResult = result || null;
  const evidence = Array.isArray(result?.evidence) ? result.evidence : [];
  const missing = Array.isArray(result?.missing) ? result.missing : [];
  const suspectedMods = Array.isArray(result?.suspectedMods)
    ? result.suspectedMods
    : Array.isArray(result?.suspected_mods)
      ? result.suspected_mods
      : [];
  const nextSteps = Array.isArray(result?.nextSteps)
    ? result.nextSteps
    : Array.isArray(result?.next_steps)
      ? result.next_steps
      : [];
  const confidence = result?.confidence || "low";
  const confidenceLabel = { high: "高", medium: "中", low: "低" }[confidence] || confidence;
  const verdictLabel = {
    missing_mod: "缺少模組或前置",
    runtime: "Java／JVM／顯示環境",
    mod_loading: "模組載入或版本相容性",
    world_content: "世界內容更新或建立世界",
    maybe_our_files: "可能是翻譯輸出",
    content_missing: "內容或註冊資料缺失",
    content_data: "資料檔載入失敗",
    unknown: "證據不足",
    no_logs: "沒有可分析記錄",
  }[result?.verdict] || result?.verdict || "未分類";
  const modeRaw = result?.analysisMode || result?.analysis_mode || "";
  const modeLabel =
    modeRaw === "pack_dir" ? "整合包目錄交叉驗證" : modeRaw === "pasted_log" ? "貼上的記錄" : "";
  const summary = String(result?.summary || "沒有足夠資料").replace(/\*\*/g, "");
  const gameExitCode = result?.gameExitCode ?? result?.game_exit_code;
  const source = result?.source || "";
  const packRoot = result?.packRoot || result?.pack_root || "";
  const errorCode = result?.errorCode || result?.error_code || "";
  const codeLabel = {
    CLASS_MISSING: "缺少類別",
    CLASS_MISSING_SRP: "缺少 SRParasites 類別",
    GRAPHICS_RUNTIME: "顯示／原生崩潰",
    INSUFFICIENT_EVIDENCE: "證據不足",
    NO_LOGS: "沒有記錄",
  }[errorCode];
  const text = [
    `判定：${verdictLabel}`,
    summary,
    modeLabel ? `分析模式：${modeLabel}` : "",
    `證據強度：${confidenceLabel}`,
    errorCode ? `分析代碼：${errorCode}${codeLabel ? `（${codeLabel}）` : ""}` : "",
    gameExitCode ? `遊戲退出碼：${gameExitCode}` : "",
    result?.primaryError || result?.primary_error ? `最接近的錯誤：${result.primaryError || result.primary_error}` : "",
    missing.length ? `可能缺少：${missing.join(", ")}` : "",
    suspectedMods.length ? `可疑模組：${suspectedMods.join(", ")}` : "",
    evidence.length ? `證據：\n- ${evidence.join("\n- ")}` : "",
    nextSteps.length ? `建議下一步：\n- ${nextSteps.join("\n- ")}` : "",
    result?.translationRelated || result?.translation_related
      ? "翻譯關聯：記錄指向翻譯輸出，可試「還原上一次套用」。"
      : "翻譯關聯：目前沒有直接證據顯示是翻譯造成。",
    packRoot ? `解析目錄：${packRoot}` : "",
    source ? `資料來源：${source}` : "",
  ]
    .filter(Boolean)
    .join("\n\n");
  const rail = $("diagnose-rail-summary");
  if (rail) {
    rail.classList.remove("log-empty");
    rail.textContent = `${verdictLabel}｜證據${confidenceLabel}\n${summary}\n\n${text}`;
  }
  appendDiagnoseLog("分析完成：" + (errorCode || verdictLabel));
  appendDiagnoseLog(text);
  if ((result?.translationRelated || result?.translation_related) && hasApplyBackups) {
    appendDiagnoseLog("可直接按「還原上一次套用」排除翻譯輸出。", "warn");
  }
}

async function diagnosePastedText() {
  const packPath = ($("diagnose-pack-path")?.value || "").trim();
  const text = ($("error-input")?.value || "").trim();
  if (!packPath && !text) return appendDiagnoseLog("請先選整合包資料夾，或貼上錯誤報告。");
  if (progressBusy) return appendDiagnoseLog("其他工作進行中，請稍候。");
  setBusy(true, "diagnose");
  clearDiagnoseLog("開始分析…");
  setProgress(5, "正在分析…");
  try {
    let result;
    if (packPath) {
      result = await invoke("diagnose_pack_dir_cmd", { path: packPath });
    } else {
      result = await invoke("diagnose_error_text", { text });
    }
    setProgress(100, "分析完成");
    showDiagnosis(result);
  } catch (e) {
    setProgress(0, "分析失敗", { failed: true });
    showDiagnosis({ summary: "分析失敗：" + formatInvokeError(e) });
  } finally {
    setBusy(false);
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
    if ($("minimize-on-close")) $("minimize-on-close").checked = min;
  } catch (e) {
    /* 預設已勾選 */
  }
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
        weight: Number($("font-weight")?.value || 400),
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
    ["font-weight", "weight", "font-weight-value"],
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
  try {
    panel.hidden = false;
    applyFontPreviewStyles();
    const b64 = await invoke("read_font_file_base64", { fontPath: path });
    const ext = String(path).toLowerCase().endsWith(".otf") ? "opentype" : "truetype";
    if (fontPreviewFace) {
      try {
        document.fonts.delete(fontPreviewFace);
      } catch (_) {
        /* ignore */
      }
    }
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
  const startupRevealFallback = window.setTimeout(() => {
    forceRevealUi();
    revealInitialContent();
  }, 2000);

  let apiSettingsTask = Promise.resolve();
  try {
    initTheme();
    initUiScale();
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
    // 背景任務：不 await，避免卡住按鈕接線
    apiSettingsTask.then(() => refreshAiStatus().catch(() => null));
    loadUiPrefs().catch(() => null);
    refreshBackupState().catch(() => null);
    setProgress(0, "尚未開始");
    resetCoverageMetrics("尚未開始");
    showAppPage("translate", { skipTransition: true });
    setTranslationState("idle");
    wireCoverageTiers();
    wireShellChrome();
    initSfxControls();
    initReloadGuard();
  } catch (e) {
    forceRevealUi();
    try {
      console.error("[boot]", e);
      appendLog("啟動初始化失敗（介面仍可操作）：" + String(e), "warn");
    } catch (_) {
      /* ignore */
    }
  }

  // —— 以下全部為同步接線（不可插入 await）——
  syncUiState();
  ["instance", "output", "font-output", "diagnose-pack-path"].forEach((id) => {
    const input = $(id);
    if (!input) return;
    input.addEventListener("input", () => {
      hasApplyBackups = false;
      if (id === "instance" && !input.value.trim()) {
        instanceValidation = { ok: false, reason: "尚未選擇遊戲資料夾。" };
        setInstanceValidateStatus(false, instanceValidation.reason, "idle");
        setTranslationState("idle");
      } else {
        if (id === "output" && customOutputEnabled()) input.dataset.customPath = input.value.trim();
        if (id === "instance") {
          window.clearTimeout(input._validateTimer);
          input._validateTimer = window.setTimeout(() => {
            validateSelectedInstance(input.value.trim());
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
  ["font-size", "font-weight", "font-shift-x", "font-shift-y", "font-oversample"].forEach((id) => {
    const input = $(id);
    const output = $(id + "-value");
    if (!input || !output) return;
    const syncValue = () => {
      output.textContent = input.value;
    };
    input.addEventListener("input", syncValue);
    syncValue();
  });
  if ($("tab-translate")) {
    $("tab-translate").onclick = () => showAppPage("translate");
  }
  if ($("tab-font")) $("tab-font").onclick = () => showAppPage("font");
  if ($("tab-diagnose")) $("tab-diagnose").onclick = () => showAppPage("diagnose");
  if ($("btn-theme")) {
    $("btn-theme").onclick = () => {
      const current = document.documentElement.dataset.theme === "light" ? "light" : "dark";
      applyTheme(current === "dark" ? "light" : "dark");
    };
  }
  if ($("btn-scale")) {
    $("btn-scale").onclick = () => {
      if (isUiAutoScaleOn()) {
        zoomAutoHint();
        return;
      }
      applyUiScale(1);
    };
  }
  if ($("ui-autoscale")) {
    $("ui-autoscale").onchange = () => setUiAutoScale(!!$("ui-autoscale").checked);
  }

  if ($("use-ai")) $("use-ai").onchange = () => syncUiState();
  if ($("api-provider")) {
    $("api-provider").onchange = () => syncCustomProviderUi($("api-provider").value);
  }
  document.querySelectorAll('input[name="ai-source"]').forEach((radio) => {
    radio.addEventListener("change", () => {
      if (radio.checked) aiModeChangePromise = changeAiMode(radio.value);
    });
  });
  if ($("btn-discord-login")) $("btn-discord-login").onclick = beginDiscordLogin;
  if ($("btn-discord-refresh")) $("btn-discord-refresh").onclick = refreshAiStatus;
  if ($("btn-turnstile-verify")) $("btn-turnstile-verify").onclick = beginTurnstileVerification;
  if ($("btn-turnstile-open")) {
    $("btn-turnstile-open").onclick = () => openExternalUrl(turnstileUrl);
  }
  if ($("btn-turnstile-cancel")) {
    $("btn-turnstile-cancel").onclick = async () => {
      await invoke("cancel_turnstile_verification_cmd");
      $("btn-turnstile-cancel").hidden = true;
    };
  }
  if ($("btn-discord-join")) {
    $("btn-discord-join").onclick = () => {
      const invite = (latestAiStatus && (latestAiStatus.inviteUrl || latestAiStatus.invite_url)) || "https://discord.gg/zeitfrei";
      return openExternalUrl(invite);
    };
  }
  if ($("btn-managed-ai-close")) {
    $("btn-managed-ai-close").onclick = () => {
      hideManagedAiErrorModal();
      setTranslationState("idle");
    };
  }
  if ($("btn-gp-reward")) {
    $("btn-gp-reward").onclick = () => startGpRewardCountdown();
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
  document.querySelectorAll('input[name="feedback-pain"], input[name="feedback-wish"], input[name="feedback-rating"]').forEach((el) => {
    el.addEventListener("change", () => updateFeedbackNoteVisibility());
  });
  if ($("btn-discord-logout")) {
    $("btn-discord-logout").onclick = async () => {
      try {
        await invoke("discord_logout");
        turnstileUrl = "";
        if ($("btn-turnstile-open")) $("btn-turnstile-open").hidden = true;
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
          : "將從遊戲實例自動偵測（須為 1.13 以上）";
      }
      syncUiState();
    };
  }
  if ($("minimize-on-close")) {
    $("minimize-on-close").onchange = async () => {
      try {
        const msg = await invoke("set_ui_prefs", {
          minimizeOnClose: !!$("minimize-on-close").checked,
        });
        log(msg || "已更新關閉行為");
      } catch (e) {
        log("無法儲存關閉偏好：\n" + String(e));
      }
    };
  }
  if ($("backup-before-apply")) {
    $("backup-before-apply").onchange = saveBackupPreference;
  }
  if ($("btn-quit")) {
    $("btn-quit").onclick = async () => {
      try {
        await invoke("quit_app");
      } catch (e) {
        window.close();
      }
    };
  }
  if ($("btn-save-adv")) $("btn-save-adv").onclick = onSaveAdv;
  if ($("btn-test-api")) $("btn-test-api").onclick = onTestApiKey;
  if ($("btn-run")) $("btn-run").onclick = onRun;
  if ($("btn-stop")) $("btn-stop").onclick = onStop;
  if ($("btn-glossary")) $("btn-glossary").onclick = onOpenGlossary;
  if ($("btn-supplement")) $("btn-supplement").onclick = onSupplement;
  if ($("btn-repair")) $("btn-repair").onclick = onRepair;
  if ($("btn-helper-prepare")) $("btn-helper-prepare").onclick = prepareTranslationHelper;
  if ($("btn-helper-rescan")) $("btn-helper-rescan").onclick = rescanAfterTranslationHelper;
  if ($("btn-helper-cleanup")) $("btn-helper-cleanup").onclick = cleanupTranslationHelperFromPanel;
  if ($("helper-ack-ingame")) {
    $("helper-ack-ingame").onchange = () => syncTranslationHelperPanel();
  }
  function openGuideOverlay() {
    const ov = $("guide-overlay");
    if (!ov) return;
    ov.hidden = false;
    ov.setAttribute("aria-hidden", "false");
    // 滾到頂
    const body = ov.querySelector(".guide-content");
    if (body) body.scrollTop = 0;
  }
  function closeGuideOverlay() {
    const ov = $("guide-overlay");
    if (!ov) return;
    ov.hidden = true;
    ov.setAttribute("aria-hidden", "true");
  }
  if ($("btn-guide")) {
    $("btn-guide").onclick = () => {
      openGuideOverlay();
    };
  }
  if ($("btn-guide-close")) {
    $("btn-guide-close").onclick = () => closeGuideOverlay();
  }
  if ($("btn-onboard")) {
    $("btn-onboard").onclick = () => {
      const menu = $("overflow-menu");
      if (menu) menu.hidden = true;
      const overflow = $("btn-overflow");
      if (overflow) overflow.setAttribute("aria-expanded", "false");
      startOnboarding({ force: true });
    };
  }
  if ($("onboard-skip")) {
    $("onboard-skip").onclick = () => stopOnboarding(true);
  }
  if ($("onboard-prev")) {
    $("onboard-prev").onclick = () => {
      if (onboardIndex > 0) {
        onboardIndex -= 1;
        layoutOnboarding();
      }
    };
  }
  if ($("onboard-next")) {
    $("onboard-next").onclick = () => {
      if (onboardIndex >= ONBOARD_STEPS.length - 1) {
        stopOnboarding(true);
        return;
      }
      onboardIndex += 1;
      layoutOnboarding();
    };
  }
  if ($("onboard-shade")) {
    $("onboard-shade").onclick = () => stopOnboarding(true);
  }
  // 目錄錨點在 overlay 內平滑滾動
  document.querySelectorAll(".guide-toc a[href^='#']").forEach((a) => {
    a.addEventListener("click", (ev) => {
      const id = a.getAttribute("href");
      if (!id || id.length < 2) return;
      const target = document.querySelector(id);
      if (target) {
        ev.preventDefault();
        target.scrollIntoView({ behavior: "smooth", block: "start" });
      }
    });
  });
  // Ctrl＋方向鍵／Ctrl＋滾輪調整介面比例；阻止 WebView 直接縮放整個頁面
  window.addEventListener("keydown", (ev) => {
    if (ev.ctrlKey && (ev.key === "ArrowUp" || ev.key === "ArrowDown" || ev.key === "0")) {
      ev.preventDefault();
      if (isUiAutoScaleOn()) {
        zoomAutoHint();
        return;
      }
      if (ev.key === "ArrowUp") adjustUiScale(UI_SCALE_STEP);
      else if (ev.key === "ArrowDown") adjustUiScale(-UI_SCALE_STEP);
      else applyUiScale(1);
      return;
    }
    if (ev.key === "Escape") {
      if (onboardActive) {
        stopOnboarding(true);
        return;
      }
      closeGuideOverlay();
    }
  });
  window.addEventListener(
    "wheel",
    (ev) => {
      if (!ev.ctrlKey) return;
      ev.preventDefault();
      if (isUiAutoScaleOn()) {
        zoomAutoHint();
        return;
      }
      adjustUiScale(ev.deltaY < 0 ? UI_SCALE_STEP : -UI_SCALE_STEP);
    },
    { passive: false }
  );
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
    $("btn-font-build").onclick = async () => {
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
            weight: Number($("font-weight")?.value || 400),
            shiftX: Number($("font-shift-x")?.value || 0),
            shiftY: Number($("font-shift-y")?.value || 0.5),
            oversample: Number($("font-oversample")?.value || 4),
          },
          targetVersion,
        });
        let finalMessage = r.playerSummary || r.player_summary || JSON.stringify(r, null, 2);
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
            finalMessage += "\n\n" + (applied.playerSummary || applied.player_summary || JSON.stringify(applied, null, 2));
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
              result?.attribution || "已下載 CFPA 參考包；只填缺並轉台灣用語，不上傳共享 R2。";
          }
          if ($("reference-ack-license")) $("reference-ack-license").checked = true;
          appendLog("CFPA 下載完成：" + path);
        }
      } catch (e) {
        appendLog("CFPA 下載略過：" + formatInvokeError(e), "warn");
        if ($("reference-status")) {
          $("reference-status").textContent = "CFPA 下載失敗，可改選本機 zip／資料夾。";
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
      const rail = $("font-rail-msg");
      if (rail) rail.textContent = `已套用預設「${btn.textContent}」。可再微調後建立。`;
    });
  });
  applyFontPrefs(readFontPrefs());
  ["font-size", "font-weight", "font-shift-x", "font-shift-y", "font-oversample", "font-pack-name"].forEach((id) => {
    const el = $(id);
    if (!el) return;
    el.addEventListener("change", writeFontPrefs);
  });

  if ($("pack-name")) {
    $("pack-name").addEventListener("input", () => {
      $("pack-name").dataset.auto = "0";
    });
  }

  if ($("btn-inst")) $("btn-inst").onclick = async () => {
    try {
      const p = await pickDir("選擇遊戲／整合包資料夾");
      if (p) {
        $("instance").value = p;
        const ok = await validateSelectedInstance(p);
        setTranslationState(ok ? "ready" : "idle");
        await detectVersionForInstance(p, false);
        await refreshPackTranslationName(p);
        await refreshReferencePack();
        $("output").value = "";
        $("output").dataset.autoPath = "";
        $("output").dataset.customPath = "";
        if (!($("output").value || "").trim()) {
          try {
            const base =
              (await invoke("managed_output_for_instance", { instancePath: p }).catch(() => null)) ||
              (await invoke("suggest_output_dir", { instancePath: p }).catch(() => null));
            if (base) {
              setAutoOutputDir(base);
              appendLog(
                "此整合包專用結果位置：\n" +
                  base +
                  "\n翻譯完成會直接套用到整合包資料夾。多包請勿共用同一結果資料夾。"
              );
            }
          } catch (_) {
            /* 略 */
          }
        }
        syncUiState();
        await refreshTranslationHelper();
      }
    } catch (e) {
      log(String(e));
    }
  };
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
  if ($("btn-suggest-rp")) {
    $("btn-suggest-rp").onclick = async () => {
      const instancePath = ($("instance").value || "").trim();
      if (!instancePath) {
        return log("請先選「遊戲資料夾」。");
      }
      try {
        const p =
          (await invoke("suggest_output_dir", { instancePath }).catch(() => null)) ||
          (await invoke("suggest_resourcepacks_dir", { instancePath }));
        setAutoOutputDir(p);
        appendLog("已建議結果位置：\n" + p);
      } catch (e) {
        log("無法建議路徑：\n" + String(e));
      }
    };
  }
  if ($("btn-package")) {
    $("btn-package").onclick = () => packageShare();
  }
  if ($("btn-share-confirm")) $("btn-share-confirm").onclick = confirmShareUpload;
  if ($("btn-share-cancel")) $("btn-share-cancel").onclick = closeShareConfirmation;
  ["share-confirm-reviewed", "share-confirm-private"].forEach((id) => {
    $(id)?.addEventListener("change", syncUiState);
  });
  if ($("btn-diagnose")) $("btn-diagnose").onclick = diagnosePastedText;
  if ($("btn-diagnose-pack-pick")) {
    $("btn-diagnose-pack-pick").onclick = async () => {
      try {
        const p = await pickDir("選擇要診斷的整合包／實例資料夾");
        if (!p) return;
        if ($("diagnose-pack-path")) $("diagnose-pack-path").value = p;
        scheduleBackupStateRefresh();
        syncUiState();
      } catch (e) {
        log("選取資料夾失敗：" + formatInvokeError(e));
      }
    };
  }
  if ($("btn-diagnose-latest")) {
    $("btn-diagnose-latest").onclick = async () => {
      const diagnosePath = ($("diagnose-pack-path")?.value || "").trim();
      const instancePath = diagnosePath || ($("instance")?.value || "").trim();
      if (!instancePath) return log("請先在診斷頁選整合包資料夾，或到翻譯頁選擇遊戲資料夾。");
      if (progressBusy) return log("翻譯或其他工作進行中，請等結束或按停止後再診斷。");
      setBusy(true, "diagnose");
      try {
        const result = await invoke("diagnose_pack_dir_cmd", { path: instancePath });
        showDiagnosis(result);
      } catch (e) {
        showDiagnosis({ summary: "讀取最近記錄失敗：" + formatInvokeError(e) });
      } finally {
        setBusy(false);
      }
    };
  }
  if ($("btn-restore")) {
    $("btn-restore").onclick = async () => {
      const instancePath =
        ($("diagnose-pack-path")?.value || "").trim() || ($("instance")?.value || "").trim();
      if (!instancePath) return log("請先選擇遊戲資料夾或診斷頁的整合包資料夾。");
      if (!window.confirm("請先關閉 Minecraft。這會回到此備份對應的套用前狀態（若多次套用曾重用同一備份，可能跨過好幾次）。確定繼續？")) return;
      try {
        const result = await invoke("restore_last_apply_cmd", {
          instancePath,
          outputDir: selectedOutputDir() || null,
        });
        const summary = result.playerSummary || result.player_summary || "已還原上一次套用。";
        const warnings = result.warnings || [];
        appendDiagnoseLog(summary, "warn");
        appendDiagnoseLog("若懷疑共享庫，補翻時可勾選「重新翻譯缺漏（略過共享庫查找）」再重跑。", "warn");
        if (warnings.length) appendDiagnoseLog("還原警告：\n" + warnings.join("\n"), "warn");
        await refreshBackupState();
      } catch (e) {
        appendDiagnoseLog("還原失敗：" + formatInvokeError(e), "error");
      }
    };
  }
  if ($("btn-diagnose-report")) {
    $("btn-diagnose-report").onclick = submitDiagnoseReport;
  }
  if ($("btn-delete-backups")) {
    $("btn-delete-backups").onclick = async () => {
      const instancePath = ($("instance").value || "").trim();
      if (!instancePath) return log("請先選擇遊戲實例。");
      if (
        !window.confirm(
          "這會刪除翻譯結果資料夾內所有由工具建立的備份，且無法還原。確定要刪除嗎？"
        )
      ) {
        return;
      }
      try {
        const result = await invoke("delete_apply_backups_cmd", {
          instancePath,
          outputDir: selectedOutputDir() || null,
        });
        appendLog(
          result.playerSummary || result.player_summary || "備份刪除完成。",
          result.failed?.length ? "warn" : "info"
        );
        await refreshBackupState();
      } catch (e) {
        appendError("刪除備份失敗");
        appendError(formatInvokeError(e));
      }
    };
  }
  if ($("btn-delete-output")) {
    $("btn-delete-output").onclick = async () => {
      const outputDir = selectedOutputDir();
      if (!outputDir) return log("還沒有可刪除的翻譯結果資料夾。");
      if (!window.confirm("這會完整刪除工具建立的翻譯結果資料夾與其中備份，確定要繼續嗎？")) return;
      try {
        const result = await invoke("delete_result_folder_cmd", { outputDir });
        appendLog(result.playerSummary || result.player_summary || "翻譯結果資料夾已刪除。", "warn");
        setTranslationState("ready");
        const input = $("output");
        if (input) {
          input.dataset.autoPath = "";
          input.dataset.customPath = "";
          input.value = "";
        }
        hasApplyBackups = false;
        syncUiState();
      } catch (e) {
        appendError("刪除翻譯結果失敗");
        appendError(formatInvokeError(e));
      }
    };
  }
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
  if ($("btn-clear-diagnose-log")) {
    $("btn-clear-diagnose-log").onclick = () => clearDiagnoseLog("診斷日誌已清除");
  }
  ["font-size", "font-weight", "font-shift-x", "font-shift-y"].forEach((id) => {
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
  window.setTimeout(() => {
    try {
      startOnboarding({ force: false });
    } catch (_) {
      /* ignore */
    }
  }, 350);

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
    await listen("turnstile-url", (ev) => {
      const payload = (ev && ev.payload) || {};
      turnstileUrl = String(payload.url || "").trim();
      if ($("btn-turnstile-open")) $("btn-turnstile-open").hidden = !turnstileUrl;
    });
  } catch (e) {
    /* 後端仍會直接開啟瀏覽器；事件只供手動重開。 */
  }

});

// ───────────────────────── 檢查更新（自足模組）─────────────────────────
// 介面由 GPT 維護；本區塊只負責把「檢查更新」接到後端，且完全防禦式：
// 有 #btn-check-update 就接上點擊；沒有也不影響其他功能。
// 契約：invoke("check_update") → { current, latest, updateAvailable, url, notes, ok, message }
//       invoke("download_update") → { path, launched, automatic, shouldExit, message }
(function wireUpdateChecker() {
  const _invoke =
    (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) || null;
  let latestUpdateInfo = null;
  let updateInFlight = false;
  let pendingUpdateInfo = null;

  function parseUpdateNotes(notes) {
    const raw = String(notes || "").trim();
    if (!raw) return [];
    return raw
      .split(/[\n;；]+/)
      .map((s) => s.trim())
      .filter(Boolean);
  }

  function isLegacy102Client(version) {
    return String(version || "")
      .trim()
      .replace(/^v/i, "") === "1.0.2";
  }

  function showUpdateModal(info) {
    const overlay = document.getElementById("update-overlay");
    const line = document.getElementById("update-version-line");
    const list = document.getElementById("update-notes-list");
    const title = document.getElementById("update-title");
    const migration = document.getElementById("update-migration-hint");
    if (!overlay) return;
    if (title) title.textContent = "MCPL " + (info.latest || "");
    if (line) {
      line.textContent =
        "發現新版本 " + info.latest + "（目前 " + info.current + "）";
    }
    if (migration) {
      if (isLegacy102Client(info.current)) {
        migration.textContent =
          "您目前是 1.0.2：請按「手動下載」，關閉舊工具後開啟；之後版本才支援一鍵自動更新。";
        migration.hidden = false;
        migration.setAttribute("aria-hidden", "false");
      } else {
        migration.textContent = "";
        migration.hidden = true;
        migration.setAttribute("aria-hidden", "true");
      }
    }
    if (list) {
      list.innerHTML = "";
      const items = parseUpdateNotes(info.notes);
      if (items.length > 0) {
        list.hidden = false;
        items.forEach((item) => {
          const li = document.createElement("li");
          li.textContent = item;
          list.appendChild(li);
        });
      } else {
        list.hidden = true;
      }
    }
    overlay.hidden = false;
    overlay.setAttribute("aria-hidden", "false");
  }

  function hideUpdateModal() {
    const overlay = document.getElementById("update-overlay");
    if (!overlay) return;
    overlay.hidden = true;
    overlay.setAttribute("aria-hidden", "true");
  }

  function setPendingUpdateInfo(info) {
    if (info) pendingUpdateInfo = info;
  }

  // 給 setBusy 使用：翻譯/其他工作完成後，自動補回被延遲的更新。
  window.zfUpdateModalMaybeShowPending = () => {
    try {
      if (pendingUpdateInfo && !progressBusy) {
        const info = pendingUpdateInfo;
        pendingUpdateInfo = null;
        latestUpdateInfo = info;
        window.__mcpl_latestUpdateInfo = latestUpdateInfo;
        showUpdateModal(info);
      }
    } catch (_) {
      /* ignore */
    }
  };

  window.zfUpdateModalHide = hideUpdateModal;
  window.zfUpdateModalSetPending = setPendingUpdateInfo;

  function scheduleClientExit() {
    setTimeout(() => {
      try {
        const win = window.__TAURI__ && window.__TAURI__.window && window.__TAURI__.window.getCurrentWindow;
        if (typeof win === "function") win().close().catch(() => {});
      } catch (_) {
        /* ignore */
      }
      setTimeout(() => {
        try {
          window.close();
        } catch (_) {
          /* ignore */
        }
      }, 500);
    }, 1200);
  }

  async function runUpdateCheck(interactive) {
    if (!_invoke) return;
    let info;
    try {
      info = await _invoke("check_update");
    } catch (e) {
      if (interactive && typeof appendLog === "function") {
        appendLog("暫時無法檢查更新：" + String(e), "warn");
      }
      return;
    }
    const status = document.getElementById("update-status");
    if (!info || !info.ok) {
      if (status) status.textContent = info && info.message ? info.message : "暫時無法檢查更新";
      if (interactive && typeof appendLog === "function") {
        appendLog((info && info.message) || "暫時無法檢查更新", "warn");
      }
      return;
    }
    latestUpdateInfo = info;
    window.__mcpl_latestUpdateInfo = latestUpdateInfo;
    if (status) status.textContent = info.message || "";
    if (!info.updateAvailable) {
      if (interactive && typeof appendLog === "function") appendLog(info.message || "已是最新版");
      return;
    }
    const btn = document.getElementById("btn-check-update");
    if (btn) {
      btn.textContent = "下載新版 " + info.latest;
      btn.dataset.mode = "download";
    }
    if (progressBusy) {
      setPendingUpdateInfo(info);
      if (status) status.textContent = interactive ? "目前正在翻譯/其他工作，完成後會顯示更新。" : status.textContent;
      if (interactive && typeof appendLog === "function") appendLog("忙碌中：更新延遲顯示", "warn");
      return;
    }
    pendingUpdateInfo = null;
    showUpdateModal(info);
    if (interactive && typeof appendLog === "function") {
      appendLog("發現新版本 " + info.latest + "（目前 " + info.current + "）。", "warn");
    }
  }

  async function openManualDownload() {
    hideUpdateModal();
    const url = latestUpdateInfo && latestUpdateInfo.url;
    if (!url || !_invoke) return;
    try {
      await _invoke("open_url", { url });
      if (typeof appendLog === "function") appendLog("已用瀏覽器開啟官方免安裝版下載。", "warn");
    } catch (e) {
      if (typeof appendLog === "function") appendLog("無法開啟手動下載：" + String(e), "error");
    }
  }

  async function runDownload() {
    if (!_invoke || updateInFlight) return;
    updateInFlight = true;
    hideUpdateModal();
    const btn = document.getElementById("btn-check-update");
    const status = document.getElementById("update-status");
    const nowBtn = document.getElementById("btn-update-now");
    if (btn) {
      btn.disabled = true;
      btn.textContent = "正在更新…";
    }
    if (nowBtn) nowBtn.disabled = true;
    if (status) status.textContent = "正在下載並驗證免安裝版";
    try {
      if (typeof appendLog === "function") appendLog("正在下載並驗證新版 EXE，請勿關閉工具…");
      const DOWNLOAD_INVOKE_TIMEOUT_MS = 180000; // UI 層硬超時：避免前端等待無限久
      let timer = null;
      let r;
      try {
        r = await Promise.race([
          _invoke("download_update"),
          new Promise((_, reject) => {
            timer = window.setTimeout(() => {
              const err = new Error("update_invoke_timeout");
              err.code = "update_invoke_timeout";
              reject(err);
            }, DOWNLOAD_INVOKE_TIMEOUT_MS);
          }),
        ]);
      } finally {
        if (timer) clearTimeout(timer);
      }
      const message = (r && r.message) || "免安裝更新檔已啟動。";
      if (typeof appendLog === "function") appendLog(message);
      if (status) status.textContent = r && r.automatic ? "正在更新，稍後會重新開啟" : "請開啟下載的免安裝版完成更新";
      if (r && (r.shouldExit || r.automatic)) scheduleClientExit();
    } catch (e) {
      const isTimeout = e && (e.code === "update_invoke_timeout" || String(e).includes("update_invoke_timeout"));
      if (isTimeout) {
        if (typeof appendLog === "function") appendLog("更新呼叫超時，前端將恢復但更新可能仍在背景進行。", "warn");
        if (status) status.textContent = "更新正在背景進行，稍後再看狀態（必要時會重開）。";
      } else {
        if (typeof appendLog === "function") appendLog("下載更新失敗：" + String(e), "error");
        if (status) status.textContent = "自動更新失敗";
        if (latestUpdateInfo && latestUpdateInfo.url) showUpdateModal(latestUpdateInfo);
      }
    } finally {
      updateInFlight = false;
      if (btn) {
        btn.disabled = false;
        btn.textContent = latestUpdateInfo && latestUpdateInfo.latest
          ? "下載新版 " + latestUpdateInfo.latest
          : "檢查更新";
      }
      if (nowBtn) nowBtn.disabled = false;
    }
  }

  function attach() {
    const btn = document.getElementById("btn-check-update");
    if (btn && !btn.dataset.wired) {
      btn.dataset.wired = "1";
      btn.addEventListener("click", () => {
        if (btn.dataset.mode === "download") runDownload();
        else runUpdateCheck(true);
      });
    }
    const later = document.getElementById("btn-update-close");
    if (later && !later.dataset.wired) {
      later.dataset.wired = "1";
      later.addEventListener("click", hideUpdateModal);
    }
    const manual = document.getElementById("btn-update-manual");
    if (manual && !manual.dataset.wired) {
      manual.dataset.wired = "1";
      manual.addEventListener("click", openManualDownload);
    }
    const now = document.getElementById("btn-update-now");
    if (now && !now.dataset.wired) {
      now.dataset.wired = "1";
      now.addEventListener("click", runDownload);
    }
    runUpdateCheck(false);
  }

  if (document.readyState === "loading") {
    window.addEventListener("DOMContentLoaded", attach);
  } else {
    attach();
  }
  window.zfCheckUpdate = () => runUpdateCheck(true);
})();
