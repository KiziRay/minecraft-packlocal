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
const BACKUP_STORAGE_KEY = "modpack-i18n-backup-before-apply";
const FONT_PREFS_STORAGE_KEY = "modpack-i18n-font-prefs";
const UI_SCALE_MIN = 1;
const UI_SCALE_MAX = 1.5;
const UI_SCALE_STEP = 0.05;
let uiScale = 1;
let latestAiStatus = null;
let discordLoginUrl = "";
let turnstileUrl = "";
let aiModeChangePromise = Promise.resolve();
let translationState = "idle";
let shareConfirmationOpen = false;
let shareUploadInFlight = false;
let apiKeyDraft = "";
let apiKeySavedMask = "";
let apiKeyEditing = false;
let hasApplyBackups = false;
let backupProbeToken = 0;
let backupProbeTimer = 0;
let translationHelperStatus = null;
let coverageSkippedSeen = new Set();
let coverageMetrics = {
  glossary: 0,
  tm: 0,
  shared: 0,
  ai: 0,
  pending: null,
  skipped: 0,
  summary: "尚未開始",
};
const STARTUP_SKELETON_MAX_MS = 320;
const PAGE_SWITCH_SKELETON_MS = 160;
const CONTENT_FADE_MS = 260;
let startupContentRevealed = false;
let pageTransitionToken = 0;

function prefersReducedMotion() {
  return !!window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches;
}

function waitMs(ms) {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

async function waitForStartupSkeleton(tasks) {
  if (prefersReducedMotion()) return;
  const guarded = tasks.map((task) => Promise.resolve(task).catch(() => null));
  await Promise.race([Promise.allSettled(guarded), waitMs(STARTUP_SKELETON_MAX_MS)]);
}

function revealInitialContent() {
  if (!document.body || startupContentRevealed) return;
  startupContentRevealed = true;
  document.body.classList.remove("is-loading");
  document.querySelectorAll(".page-panel.is-loading, .status-section.is-loading").forEach((el) => {
    el.classList.remove("is-loading");
  });
  if (prefersReducedMotion()) return;
  document.body.classList.add("content-fade-in");
  window.setTimeout(() => {
    document.body.classList.remove("content-fade-in");
  }, CONTENT_FADE_MS + 90);
}

function revealPagePanel(panel, withSkeleton = true) {
  if (!panel) return;
  const token = ++pageTransitionToken;
  document.querySelectorAll(".page-panel.is-loading").forEach((el) => {
    if (el !== panel) el.classList.remove("is-loading");
  });
  panel.classList.remove("content-fade-in");
  if (prefersReducedMotion()) {
    panel.classList.remove("is-loading");
    return;
  }
  if (withSkeleton) panel.classList.add("is-loading");
  window.setTimeout(() => {
    window.requestAnimationFrame(() => {
      if (token !== pageTransitionToken || panel.hidden) return;
      panel.classList.remove("is-loading");
      panel.classList.add("content-fade-in");
      window.setTimeout(() => {
        if (token === pageTransitionToken) panel.classList.remove("content-fade-in");
      }, CONTENT_FADE_MS + 90);
    });
  }, withSkeleton ? PAGE_SWITCH_SKELETON_MS : 0);
}

function resetCoverageMetrics(summary = "等待翻譯開始") {
  coverageSkippedSeen = new Set();
  coverageMetrics = {
    glossary: 0,
    tm: 0,
    shared: 0,
    ai: 0,
    pending: null,
    skipped: 0,
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
  setText("metric-summary", coverageMetrics.summary || "等待資料");
  const translated =
    (Number(coverageMetrics.glossary) || 0) +
    (Number(coverageMetrics.tm) || 0) +
    (Number(coverageMetrics.shared) || 0) +
    (Number(coverageMetrics.ai) || 0);
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

function consumeCoverageMessage(message) {
  const text = String(message || "");
  if (!text) return;
  let changed = false;
  const finalHit = text.match(/補譯\s+(\d+)\s+條（術語表\s+(\d+)、共享庫\s+(\d+)、翻譯記憶\s+(\d+)、AI\s+(\d+)）/);
  if (finalHit) {
    coverageMetrics.glossary = Math.max(coverageMetrics.glossary, Number(finalHit[2]) || 0);
    coverageMetrics.shared = Math.max(coverageMetrics.shared, Number(finalHit[3]) || 0);
    coverageMetrics.tm = Math.max(coverageMetrics.tm, Number(finalHit[4]) || 0);
    coverageMetrics.ai = Math.max(coverageMetrics.ai, Number(finalHit[5]) || 0);
    coverageMetrics.summary = `已命中／補譯 ${finalHit[1]} 條`;
    changed = true;
  }
  const freeHit = text.match(/免費命中\s+(\d+)\s+句（術語表\s+(\d+)、翻譯記憶\s+(\d+)）.*?只剩\s+(\d+)\s+句/);
  if (freeHit) {
    coverageMetrics.glossary = Math.max(coverageMetrics.glossary, Number(freeHit[2]) || 0);
    coverageMetrics.tm = Math.max(coverageMetrics.tm, Number(freeHit[3]) || 0);
    coverageMetrics.pending = Number(freeHit[4]) || coverageMetrics.pending;
    coverageMetrics.summary = `免費命中 ${freeHit[1]} 句`;
    changed = true;
  }
  const sharedHit = text.match(/社群共享庫命中\s+(\d+)\s+條/);
  if (sharedHit) {
    coverageMetrics.shared = Math.max(coverageMetrics.shared, Number(sharedHit[1]) || 0);
    changed = true;
  }
  const aiHit = text.match(/AI\s+(?:新補|新譯|翻譯)\s*(?:約\s*)?(\d+)\s*(?:條|句)/);
  if (aiHit) {
    coverageMetrics.ai = Math.max(coverageMetrics.ai, Number(aiHit[1]) || 0);
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

function applyUiScale(value, save = true) {
  uiScale = clampUiScale(value);
  document.documentElement.style.setProperty("--ui-scale", String(uiScale));
  const label = $("scale-label");
  const button = $("btn-scale");
  const percent = Math.round(uiScale * 100);
  if (label) label.textContent = `介面 ${percent}%`;
  if (button) {
    button.title = `Ctrl＋↑／↓或 Ctrl＋滾輪調整介面大小；點擊重設為 100%（目前 ${percent}%）`;
  }
  if (save) {
    try {
      localStorage.setItem(UI_SCALE_STORAGE_KEY, String(uiScale));
    } catch (_) {
      /* 瀏覽器儲存不可用時仍保留本次縮放 */
    }
  }
}

function initUiScale() {
  let saved = 1;
  try {
    saved = clampUiScale(localStorage.getItem(UI_SCALE_STORAGE_KEY) || 1);
  } catch (_) {
    /* 使用預設比例 */
  }
  applyUiScale(saved, false);
}

function adjustUiScale(delta) {
  applyUiScale(uiScale + delta);
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
}

function resultWorkDir(outputDir) {
  const clean = String(outputDir || "").replace(/[\\/]+$/, "");
  return /(?:^|[\\/])翻譯結果$/i.test(clean) ? clean : clean + "\\翻譯結果";
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
  if (meta) meta.setAttribute("content", normalized === "dark" ? "#111315" : "#f7f7f5");
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

/** 完整進度／錯誤日誌（時間戳 + 累積） */
const progressLogLines = [];
const MAX_LOG_LINES = 5000;
let errorLogCount = 0;

function nowStamp() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, "0");
  return p(d.getHours()) + ":" + p(d.getMinutes()) + ":" + p(d.getSeconds());
}

function renderLog() {
  const el = $("log");
  if (!el) return;
  el.classList.remove("log-empty");
  el.textContent = progressLogLines.join("\n");
  el.scrollTop = el.scrollHeight;
}

function clearLog(seedMsg) {
  progressLogLines.length = 0;
  errorLogCount = 0;
  lastProgressLogKey = "";
  displayPercent = 0;
  lastRealPercent = 0;
  lastRealMessage = "";
  if (seedMsg) progressLogLines.push("[" + nowStamp() + "] " + seedMsg);
  renderLog();
}

function appendLog(msg, level) {
  const text = String(msg == null ? "" : msg).replace(/\r\n/g, "\n");
  if (!text) return;
  const lv = level || "info";
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
  renderLog();
}

function appendError(msg) {
  appendLog(msg, "error");
}

/** 相容舊呼叫：整段覆寫改為附加；完成摘要用 setLogFinal */
function log(msg) {
  appendLog(msg);
}

function setLogFinal(msg) {
  appendLog("────────");
  if (errorLogCount > 0) {
    appendLog("本次共記錄 " + errorLogCount + " 筆錯誤／警告相關行。", "warn");
    appendLog("完整錯誤也會寫在結果資料夾「翻譯錯誤日誌.txt」（若有）。", "warn");
  }
  appendLog(msg);
  maybeHintAiQuota(msg);
}

/** AI 額度用完：提示支持（不提服務商名稱） */
function maybeHintAiQuota(text) {
  const s = String(text || "");
  if (!/額度|餘額|沒有回應|金鑰無效|無權限|沒有有效回應|請我喝珍奶|沒有餘力/.test(s)) {
    return;
  }
  appendLog("────────", "warn");
  appendLog("開發者目前無法再為 AI 加值，需要你的支持。", "warn");
  appendLog("有自己的金鑰請重新儲存後再試；也歡迎按下方「請我喝珍奶」。", "warn");
}

/** 使用者按停止不是錯誤，畫面不該變成一片紅字 */
function isCancellation(e) {
  return /已依你的要求停止/.test(formatInvokeError(e));
}

/** 統一處理各流程的失敗／取消收尾 */
function handleRunFailure(e, whatFailed) {
  if (isCancellation(e)) {
    setProgress(0, "已停止");
    appendLog("已停止。先前完成的部分仍保留在結果資料夾。", "warn");
    return;
  }
  setProgress(0, whatFailed, { failed: true });
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

/** mockup 四步：準備 → 掃描 → 翻譯 → 完成（polish/ai/pack 併入 translate） */
const STEP_ORDER = ["prep", "scan", "translate", "done"];

function stepFromProgress(percent, message) {
  const p = Number(percent) || 0;
  const m = String(message || "");
  if (p <= 0) return null;
  if (p >= 100 || /全部完成|補翻完成|字體包完成|已套用/.test(m)) return "done";
  if (/失敗|錯誤/.test(m) && p === 0) return "error";
  if (/字體/.test(m)) return p >= 90 ? "done" : "prep";
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
  if (p < 100) return "translate";
  return "done";
}

function updateLinearSteps(percent, message, failed) {
  const root = $("linear-steps");
  if (!root) return;
  const items = root.querySelectorAll(".lin-step");
  if (failed) {
    items.forEach((el) => {
      el.classList.remove("active", "done");
      el.classList.add("error");
    });
    return;
  }
  const cur = stepFromProgress(percent, message);
  const idx = cur ? STEP_ORDER.indexOf(cur) : -1;
  items.forEach((el) => {
    el.classList.remove("active", "done", "error");
    const si = STEP_ORDER.indexOf(el.getAttribute("data-step"));
    if (idx < 0) return;
    if (si < idx) el.classList.add("done");
    else if (si === idx) el.classList.add(cur === "done" ? "done" : "active");
  });
  if (cur === "done") {
    items.forEach((el) => {
      el.classList.remove("active");
      el.classList.add("done");
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
  const s = Math.floor(ms / 1000);
  if (s < 60) return s + " 秒";
  const m = Math.floor(s / 60);
  const r = s % 60;
  return m + " 分 " + r + " 秒";
}

function formatProgressEta() {
  const percent = Number(lastRealPercent) || 0;
  if (!progressBusy || !progressStartedAt || percent < 3 || percent >= 99) return "";
  const elapsed = Date.now() - progressStartedAt;
  if (elapsed < 2500) return "";
  const remaining = Math.round((elapsed * (100 - percent)) / percent);
  if (!Number.isFinite(remaining) || remaining <= 0 || remaining > 24 * 60 * 60 * 1000) return "";
  return " · 預估剩餘 " + formatElapsed(remaining);
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
        msgEl.textContent =
          baseMsg + " · 已進行 " + elapsed + formatProgressEta() + " · 仍在運作，請稍候";
        setProgBarWorking(true);
      } else {
        msgEl.textContent = baseMsg + " · 已進行 " + elapsed + formatProgressEta();
        setProgBarWorking(false);
      }
    }
  }, 1000);
}

function setProgress(percent, message, opts) {
  const p = Math.max(0, Math.min(100, Number(percent) || 0));
  const failed = opts && opts.failed;
  if (!failed && p > 0 && p < 100) {
    lastProgressAt = Date.now();
    lastRealPercent = p;
    // 真實進度追上時拉高 display
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
    consumeCoverageMessage(message);
    const elapsed =
      progressBusy && progressStartedAt
        ? " · 已進行 " + formatElapsed(Date.now() - progressStartedAt)
        : "";
    $("prog-msg").textContent = message + elapsed + formatProgressEta();
  }
  updateLinearSteps(p, message, failed);
  setProgBarWorking(false);
  // 詳細完整日誌：每筆進度都留下（同文案＋同百分比不重複洗版）
  if (message && !(opts && opts.skipLog)) {
    const key = Math.floor(p) + "|" + message;
    if (key !== lastProgressLogKey) {
      lastProgressLogKey = key;
      appendLog(Math.floor(p) + "%  " + message);
    }
  }
}

function setBusy(busy) {
  progressBusy = !!busy;
  if (busy) {
    displayPercent = Math.max(displayPercent, lastRealPercent);
    startProgressHeartbeat();
  } else {
    stopProgressHeartbeat();
  }
  // 停止鈕只在工作中出現，而且刻意不跟著 disabled 一起鎖起來
  const stop = $("btn-stop");
  if (stop) {
    stop.hidden = !busy;
    stop.disabled = false;
    stop.textContent = "停止";
  }
  // 完整使用說明翻譯中仍可點；結束程式也可
  [
    "btn-run",
    "btn-supplement",
    "btn-repair",
    "btn-delete-backups",
    "btn-output-pick",
    "btn-delete-output",
    "btn-inst",
    "btn-save-adv",
    "use-ai",
    "backup-before-apply",
    "translation-mode",
    "translation-quality",
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
    "tab-translate",
    "tab-font",
  ].forEach((id) => {
    const el = $(id);
    if (el) el.disabled = busy;
  });
  // 翻譯中鎖定路徑與資源包名稱（不可改）
  ["pack-name", "instance", "output", "reference-pack", "font-pack-name", "font-file", "font-output"].forEach((id) => {
    const el = $(id);
    if (el) {
      el.readOnly = !!busy;
      el.setAttribute("aria-readonly", busy ? "true" : "false");
    }
  });
  const guide = $("btn-guide");
  if (guide) guide.disabled = false;
  syncOutputField();
  syncUiState();
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
  if (changed && !opts.skipTransition) revealPagePanel(activePanel, opts.skeleton !== false);
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
}

function toggleHidden(id, hidden) {
  const el = $(id);
  if (!el) return;
  el.hidden = !!hidden;
}

function syncUiState() {
  const hasInstance = !!($("instance")?.value || "").trim();
  const hasOutput = !!selectedOutputDir();
  const complete = translationState === "complete";
  const failed = translationState === "failed";
  const locked = progressBusy || shareUploadInFlight;
  const page = document.body.dataset.appPage || "translate";

  ["field-output", "field-pack-name", "field-version", "translation-options", "translation-options-heading", "reference-details"]
    .forEach((id) => toggleHidden(id, !hasInstance));
  const runBtn = $("btn-run");
  if (runBtn) {
    runBtn.hidden = progressBusy;
    runBtn.disabled = !hasInstance || progressBusy;
  }
  toggleHidden("btn-supplement", !complete || locked);
  toggleHidden("btn-repair", !failed || locked);
  toggleHidden("btn-glossary", !hasInstance || locked);
  toggleHidden("btn-package", !complete || locked);
  const fontOutReady = !!($("font-output")?.value || "").trim();
  const translateOutReady = hasOutput;
  const fontApplyCurrent = $("font-apply-current");
  if (fontApplyCurrent) {
    fontApplyCurrent.disabled = locked || !hasInstance;
  }
  toggleHidden("btn-open", page !== "translate" || !translateOutReady);
  toggleHidden("btn-open-report", page !== "translate" || !translateOutReady);
  toggleHidden("btn-open-font", page !== "font" || !fontOutReady);
  toggleHidden("btn-diagnose-latest", !hasInstance || locked);
  toggleHidden("btn-restore", !hasInstance || !hasApplyBackups || locked);
  toggleHidden("btn-delete-backups", !hasInstance || !hasApplyBackups || locked);

  syncTranslationHelperPanel();

  const packageButton = $("btn-package");
  if (packageButton) packageButton.disabled = !complete || locked;
  const confirmPanel = $("share-confirm-panel");
  if (confirmPanel) confirmPanel.hidden = !shareConfirmationOpen || !complete;
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
  const hasCommand = !!status.command && ["installed", "existing"].includes(status.state);
  if (command) command.hidden = !hasCommand;
  if (commandText) commandText.textContent = status.command || "";
  const prepare = $("btn-helper-prepare");
  if (prepare) {
    prepare.hidden = status.state !== "available" || progressBusy;
    prepare.disabled = progressBusy;
    prepare.textContent = "準備任務補充";
  }
  const rescan = $("btn-helper-rescan");
  if (rescan) {
    const canRescan = status.supported && ["installed", "existing"].includes(status.state);
    rescan.hidden = !canRescan || progressBusy;
    rescan.disabled = progressBusy;
  }
  const cleanup = $("btn-helper-cleanup");
  if (cleanup) {
    cleanup.hidden = !status.installedByTool || progressBusy;
    cleanup.disabled = progressBusy;
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

async function pickDir(title) {
  if (!dialog.open) throw new Error("無法開啟資料夾選擇視窗");
  const selected = await dialog.open({ directory: true, multiple: false, title });
  return typeof selected === "string" ? selected : null;
}

/** 將已完成的翻譯結果上傳到獨立分享區，連結只保留一天。 */
function packageShare() {
  if (translationState !== "complete") {
    return log("請先完成翻譯，再檢查結果並建立分享檔。");
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
    const turnstileRequired = ai?.turnstileRequired ?? ai?.turnstile_required;
    const turnstileVerified = !!(ai && (ai.turnstileVerified || ai.turnstile_verified));
    // 代管模式且 Worker 強制 Turnstile 時才擋；自訂 API 不走此閘門。
    if ((ai?.aiMode === "managed" || ai?.ai_mode === "managed") && turnstileRequired !== false && !turnstileVerified) {
      if (!(await beginTurnstileVerification())) return;
    }
    const name = ($("pack-name").value || "模組包翻譯分享").trim();
    appendLog("正在整理可安裝檔案並上傳…");
    const result = await invoke("upload_share_package_cmd", { workRoot: work, name });
    const url = result.url || result;
    const expires = Number(result.expiresAt || result.expires_at || 0);
    try {
      await navigator.clipboard.writeText(url);
      appendLog("分享連結已複製：\n" + url);
    } catch (_) {
      appendLog("分享連結：\n" + url);
    }
    if (expires) appendLog("這個連結只會保留 24 小時，逾期就不能下載。", "warn");
  } catch (e) {
    log("分享失敗：\n" + formatInvokeError(e));
  }
}

function syncAiPanel(refreshStatus = true) {
  const panel = $("ai-panel");
  const enabled = !!$("use-ai")?.checked && !!($("instance")?.value || "").trim();
  if (panel) {
    panel.hidden = !enabled;
    panel.setAttribute("aria-hidden", enabled ? "false" : "true");
  }
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
    const managedTurnstileReady = !!(s && (s.turnstileVerified || s.turnstile_verified));
    const managedTurnstileRequired = !(
      s && (s.turnstileRequired === false || s.turnstile_required === false)
    );
    syncAiModeUi(mode);
    statusEl.textContent = ready
      ? usingOwnKey
        ? "AI：自訂 API 可用"
        : "AI：開發者 API 可用"
      : mode === "custom"
        ? "AI：請先設定自訂 API"
        : managedIdentityReady && !managedTurnstileReady
          ? "AI：請完成安全驗證"
          : "AI：尚未完成 Discord 驗證";
    if (statusRow) statusRow.dataset.state = ready ? (usingOwnKey ? "own" : "managed") : "error";

    if (mode === "managed") {
      const loggedIn = !!(s && (s.loggedIn || s.logged_in));
      const inGuild = !!(s && (s.inGuild || s.in_guild));
      const serviceAvailable = s && (s.serviceAvailable ?? s.service_available) !== false;
      const turnstileVerified = !!(s && (s.turnstileVerified || s.turnstile_verified));
      const identityReady = loggedIn && inGuild && serviceAvailable;
      const displayName = String((s && (s.displayName || s.display_name)) || "").trim();
      const title = $("discord-auth-title");
      const authNote = $("discord-auth-note");
      if (title) {
        title.textContent = ready
          ? `Discord 已驗證${displayName ? `：${displayName}` : ""}`
          : !loggedIn
            ? "Discord 尚未登入"
            : !serviceAvailable
              ? "Discord 驗證服務暫時無法使用"
              : !inGuild
                ? "尚未加入官方伺服器"
                : "Discord 尚未驗證";
      }
      if (authNote) authNote.textContent = message || "登入 Discord 並加入官方伺服器後即可使用。";
      if ($("btn-discord-login")) $("btn-discord-login").hidden = loggedIn;
      if ($("btn-discord-logout")) $("btn-discord-logout").hidden = !loggedIn;
      if ($("btn-discord-join")) $("btn-discord-join").hidden = inGuild;
      const turnstileTitle = $("turnstile-auth-title");
      const turnstileNote = $("turnstile-auth-note");
      if (turnstileTitle) {
        turnstileTitle.textContent = turnstileVerified
          ? "Cloudflare 安全驗證完成"
          : !managedTurnstileRequired
            ? "Cloudflare 安全驗證（目前不需要）"
          : identityReady
            ? "Cloudflare 尚未驗證"
            : "Cloudflare 等待 Discord 驗證";
      }
      if (turnstileNote) {
        turnstileNote.textContent = turnstileVerified
          ? "短效憑證只保留在本次開啟的工具記憶體中。"
          : !managedTurnstileRequired
            ? "目前服務端未要求這項驗證。"
          : identityReady
            ? "完成後即可使用開發者提供的翻譯額度。"
            : "先完成 Discord 登入與伺服器資格確認。";
      }
      if ($("btn-turnstile-verify")) {
        $("btn-turnstile-verify").hidden =
          !identityReady || turnstileVerified || !managedTurnstileRequired;
      }
      if (noteEl) {
        noteEl.textContent = ready
          ? "Discord 資格與安全憑證會在每次代管翻譯時再次確認。"
          : message || "請先登入 Discord 並加入 ZeitFrei 官方伺服器。";
      }
    } else if (noteEl) {
      noteEl.textContent = message || "使用自己的金鑰與額度，不需要 Discord 驗證。";
    }
    return s;
  } catch (e) {
    latestAiStatus = null;
    statusEl.textContent = "AI：狀態暫時無法確認";
    if (statusRow) statusRow.dataset.state = "error";
    if (noteEl) noteEl.textContent = "不影響本機簡繁轉換；需要 AI 補翻時請稍後再確認。";
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
    note.textContent = isOther
      ? "請再填寫 Base URL 與模型名稱；一般使用者不需要改這些設定。"
      : normalized === "glm"
        ? "只要填 API Key，工具會自動使用智譜 GLM 的官方設定。"
        : normalized === "openai"
          ? "只要填 API Key，工具會自動使用 OpenAI 的官方設定。"
          : normalized === "qwen"
            ? "只要填 API Key，工具會自動使用通義千問的官方設定。"
            : "只要填 API Key，工具會自動使用 DeepSeek 的官方設定。";
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
    const loggedIn = !!(status && (status.loggedIn || status.logged_in));
    const inGuild = !!(status && (status.inGuild || status.in_guild));
    const serviceAvailable = status && (status.serviceAvailable ?? status.service_available) !== false;
    const turnstileVerified = !!(status && (status.turnstileVerified || status.turnstile_verified));
    const turnstileRequired = !(
      status && (status.turnstileRequired === false || status.turnstile_required === false)
    );
    if (loggedIn && inGuild && serviceAvailable && turnstileRequired && !turnstileVerified) {
      if (await beginTurnstileVerification()) {
        status = await refreshAiStatus();
        return !!(status && status.ready !== false);
      }
    }
  }
  return false;
}

async function openExternalUrl(url) {
  if (!url) return;
  try {
    await invoke("open_url", { url });
  } catch (_) {
    window.open(url, "_blank");
  }
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
    if (detected && !Array.from(select.options).some((option) => option.value === detected)) {
      select.add(new Option(detected, detected));
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
      status.textContent = "找不到版本，可從下拉選單手動指定";
    }
    return detected || null;
  } catch (e) {
    if (status && !silent) status.textContent = "版本偵測失敗，可手動指定";
    return null;
  }
}

async function refreshInstanceTarget(instancePath) {
  try {
    const target = await invoke("check_install_target", { instancePath });
    const mcDir = target && (target.mcDir || target.mc_dir);
    if (!target || target.ok === false) return false;
    return true;
  } catch (_) {
    return false;
  }
}

async function refreshPackTranslationName(instancePath) {
  try {
    const info = await invoke("detect_pack_translation_name", { instancePath });
    const name = info && (info.packName || info.pack_name);
    if ($("pack-name") && name) $("pack-name").value = name;
    if ($("pack-version-status")) {
      const version = info && info.version ? info.version : "R1";
      const source = info && info.source ? info.source : "未找到版本檔，使用複查編號";
      $("pack-version-status").textContent = `資源包版本：${version}（${source}）`;
    }
  } catch (_) {
    if ($("pack-version-status")) $("pack-version-status").textContent = "整合包版本尚未偵測，完成翻譯時會使用 R1。";
  }
}

async function refreshReferencePack() {
  const input = $("reference-pack");
  const status = $("reference-status");
  if (!input || (input.value || "").trim()) return input?.value || "";
  try {
    const found = await invoke("get_default_reference_pack");
    if (found) {
      input.value = found;
      if (status) status.textContent = "已找到本機參考翻譯，翻譯時會優先套用。";
      return found;
    }
  } catch (_) {
    /* 參考翻譯是選用功能，找不到時仍可繼續。 */
  }
  if (status) status.textContent = "未找到參考翻譯；你仍可直接開始，或手動選取資料夾。";
  return "";
}

async function onSaveAdv() {
  try {
    await invoke("save_api_settings_cmd", {
      apiKey: apiKeyDraft.trim(),
      baseUrl: ($("base-url").value || "").trim(),
      provider: ($("api-provider").value || "deepseek").trim(),
      model: ($("api-model").value || "").trim(),
    });
    $("api-key").value = ""; // 輸入框清空，畫面上不留金鑰
    await refreshApiSettings();
    await refreshAiStatus();
    log("設定已儲存。");
  } catch (e) {
    log("儲存失敗：\n" + String(e));
  }
}

async function onRun() {
  const instancePath = ($("instance").value || "").trim();
  let outputDir = selectedOutputDir();
  if (!instancePath) return log("請先選擇「遊戲資料夾」。");
  if (!outputDir) {
    outputDir = (await invoke("managed_output_base").catch(() => "")) || "";
    if (outputDir) setAutoOutputDir(outputDir);
  }
  if (!outputDir) return log("翻譯結果位置還沒準備好，請重新選擇遊戲資料夾。");

  const useAi = !!$("use-ai").checked;
  if (useAi && !(await ensureAiReadyForAction())) return;
  let targetVersion = ($("target-version")?.value || "").trim();
  if (!targetVersion) {
    targetVersion = (await detectVersionForInstance(instancePath, true)) || "";
  }

  setBusy(true);
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

  try {
    const result = await invoke("one_click_translate", {
      instancePath,
      outputDir,
      packName: ($("pack-name").value || "").trim(),
      useAi,
      backupBeforeApply: shouldBackupBeforeApply(),
      referencePack: (($('reference-pack')?.value || "").trim() || null),
      targetVersion: targetVersion || null,
      translationMode: ($("translation-mode")?.value || "append"),
      translationQuality: ($("translation-quality")?.value || "balanced"),
      coverageTier: (document.querySelector('input[name="coverage-tier"]:checked')?.value || "standard"),
    });
    setProgress(100, "全部完成！");
    let msg = result.playerSummary || result.player_summary || JSON.stringify(result, null, 2);
    if (result.minemenuMsg || result.minemenu_msg) {
      msg += "\n\n" + (result.minemenuMsg || result.minemenu_msg);
    }
    consumeCoverageMessage(msg);
    setLogFinal(msg);
    setTranslationState("complete");
    appendLog("翻譯已完成並直接套用。想分享給其他玩家時，再按「翻譯完成後分享」。");
    await cleanupPreparedTranslationHelper();
  } catch (e) {
    setTranslationState("failed");
    handleRunFailure(e, "翻譯失敗");
    if (!isCancellation(e)) {
      appendLog("可把上方錯誤訊息留下來方便排查。");
    }
  } finally {
    setBusy(false);
    refreshBackupState();
  }
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

  setBusy(true);
  setTranslationState("running");
  lastProgressLogKey = "";
  clearLog("開始修復");
  appendLog("這不能修好「進世界閃退」。");
  setProgress(2, "準備修復…");

  try {
    const result = await invoke("repair_translation_pack", {
      outputDir,
      useAi,
      backupBeforeApply: shouldBackupBeforeApply(),
    });
    setProgress(100, "修復完成！");
    setLogFinal(result.playerSummary || result.player_summary || JSON.stringify(result, null, 2));
    setTranslationState("complete");
  } catch (e) {
    setTranslationState("failed");
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

  setBusy(true);
  setTranslationState("running");
  lastProgressLogKey = "";
  clearLog("開始再補一些");
  resetCoverageMetrics("補翻統計蒐集中");
  setProgress(3, "準備中…");

  try {
    const result = await invoke("supplement_translate", {
      outputDir,
      useAi,
      backupBeforeApply: shouldBackupBeforeApply(),
    });
    setProgress(100, "補譯完成！");
    let msg = result.playerSummary || result.player_summary || JSON.stringify(result, null, 2);
    consumeCoverageMessage(msg);
    setLogFinal(msg);
    setTranslationState("complete");
    appendLog("複查完成，結果已重新套用到遊戲。", "info");
    await cleanupPreparedTranslationHelper();
  } catch (e) {
    setTranslationState("failed");
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
  appendLog("已要求停止，等目前這一步做完就會收尾…", "warn");
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

function showDiagnosis(result) {
  const box = $("diagnose-result");
  if (!box) return;
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
  const summary = String(result?.summary || "沒有足夠資料").replace(/\*\*/g, "");
  const gameExitCode = result?.gameExitCode ?? result?.game_exit_code;
  const source = result?.source || "";
  const text = [
    `判定：${verdictLabel}\n${summary}`,
    `證據強度：${confidenceLabel}（這是規則命中的程度，不是保證）`,
    result?.errorCode || result?.error_code ? `分析代碼：${result.errorCode || result.error_code}` : "",
    gameExitCode ? `遊戲退出碼：${gameExitCode}（退出碼通常不是根因）` : "",
    result?.primaryError || result?.primary_error ? `最接近的錯誤：${result.primaryError || result.primary_error}` : "",
    missing.length ? `可能缺少：${missing.join(", ")}` : "",
    suspectedMods.length ? `可疑模組：${suspectedMods.join(", ")}` : "",
    evidence.length ? `證據：\n- ${evidence.join("\n- ")}` : "",
    nextSteps.length ? `建議下一步：\n- ${nextSteps.join("\n- ")}` : "",
    result?.translationRelated || result?.translation_related
      ? "翻譯關聯：記錄有直接指向翻譯輸出的證據。請先關遊戲，再用「還原上一次套用」後重試。"
      : "翻譯關聯：目前沒有直接證據顯示是翻譯造成的。",
    source ? `資料來源：${source}` : "",
  ]
    .filter(Boolean)
    .join("\n\n");
  box.textContent = text;
  const rail = $("diagnose-rail-summary");
  if (rail) rail.textContent = `${verdictLabel}｜證據${confidenceLabel}\n${summary}`;
  const railMsg = $("diagnose-rail-msg");
  if (railMsg) {
    railMsg.textContent =
      result?.verdict === "maybe_our_files"
        ? "可能與翻譯輸出有關，可考慮「還原上一次套用」。"
        : "分析完成；完整細節見左側結果卡。";
  }
  if ((result?.translationRelated || result?.translation_related) && hasApplyBackups) {
    appendLog("可直接按「還原上一次套用」排除翻譯輸出。", "warn");
  }
}

async function diagnosePastedText() {
  const text = ($("error-input")?.value || "").trim();
  if (!text) return log("請先貼上錯誤報告或錯誤碼。");
  try {
    const result = await invoke("diagnose_error_text", { text });
    showDiagnosis(result);
    appendLog("錯誤分析完成：" + (result.errorCode || result.error_code || "UNKNOWN"));
  } catch (e) {
    showDiagnosis({ summary: "分析失敗：" + formatInvokeError(e) });
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

function wireCoverageTier() {
  const applyQualityHint = () => {
    const tier = document.querySelector('input[name="coverage-tier"]:checked')?.value || "standard";
    const quality = $("translation-quality");
    if (!quality) return;
    if (tier === "quick") quality.value = "fast";
    else if (tier === "max") quality.value = "thorough";
    else quality.value = "balanced";
  };
  document.querySelectorAll('input[name="coverage-tier"]').forEach((el) => {
    el.addEventListener("change", applyQualityHint);
  });
  const ack = $("coverage-ack-hard");
  if (ack) {
    try {
      const saved = localStorage.getItem(COVERAGE_ACK_STORAGE_KEY);
      if (saved === "0") ack.checked = false;
      else if (saved === "1") ack.checked = true;
    } catch (_) {
      /* ignore */
    }
    ack.addEventListener("change", () => {
      try {
        localStorage.setItem(COVERAGE_ACK_STORAGE_KEY, ack.checked ? "1" : "0");
      } catch (_) {
        /* ignore */
      }
    });
  }
}

let fontPreviewUrl = null;

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

async function updateFontPreview(path) {
  const panel = $("font-preview");
  const sample = $("font-preview-sample");
  if (!panel || !sample || !path) return;
  try {
    if (fontPreviewUrl) URL.revokeObjectURL(fontPreviewUrl);
    // Tauri 本機路徑無法直接 FontFace；用 CSS 近似 size／shift，並標示路徑已選。
    panel.hidden = false;
    const size = Number($("font-size")?.value || 11);
    const shiftY = Number($("font-shift-y")?.value || 0.5);
    sample.style.fontSize = `${Math.max(14, size * 1.6)}px`;
    sample.style.transform = `translateY(${shiftY}px)`;
    sample.title = path;
  } catch (_) {
    panel.hidden = true;
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  const startupRevealFallback = window.setTimeout(revealInitialContent, 900);
  initTheme();
  initUiScale();
  loadBackupPreference();
  syncAiPanel(false);
  const apiSettingsTask = refreshApiSettings();
  const startupTasks = [
    apiSettingsTask,
    apiSettingsTask.catch(() => null).then(() => refreshAiStatus()),
    loadUiPrefs(),
    refreshBackupState(),
  ];
  setProgress(0, "尚未開始");
  resetCoverageMetrics("尚未開始");
  showAppPage("translate", { skipTransition: true });
  setTranslationState("idle");
  wireCoverageTier();
  await waitForStartupSkeleton(startupTasks);
  window.clearTimeout(startupRevealFallback);
  revealInitialContent();
  ["instance", "output", "font-output"].forEach((id) => {
    const input = $(id);
    if (!input) return;
    input.addEventListener("input", () => {
      hasApplyBackups = false;
      if (id === "instance" && !input.value.trim()) {
        setTranslationState("idle");
      } else {
        if (id === "output" && customOutputEnabled()) input.dataset.customPath = input.value.trim();
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
    $("btn-scale").onclick = () => applyUiScale(1);
  }

  // 即時進度（白話 + 百分比）
  try {
    await listen("translate-progress", (ev) => {
      const p = (ev && ev.payload) || {};
      const percent = p.percent != null ? p.percent : 0;
      const message = p.message || "處理中…";
      setProgress(percent, message);
      // 失敗類進度再標錯誤（setProgress 也會寫一般進度行）
      if (/失敗|錯誤|無法|不存在|損壞/.test(String(message))) {
        appendError(message);
      }
    });
  } catch (e) {
    /* 無 event 時仍可跑完後顯示 */
  }
  // 後端專用日誌通道（錯誤／警告／重要資訊）
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

  $("use-ai").onchange = () => syncUiState();
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
      if ($("version-status")) {
        $("version-status").textContent = value
          ? "已手動指定：Minecraft " + value
          : "將從遊戲實例自動偵測";
      }
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
  $("btn-save-adv").onclick = onSaveAdv;
  $("btn-run").onclick = onRun;
  if ($("btn-stop")) $("btn-stop").onclick = onStop;
  if ($("btn-glossary")) $("btn-glossary").onclick = onOpenGlossary;
  if ($("btn-supplement")) $("btn-supplement").onclick = onSupplement;
  if ($("btn-repair")) $("btn-repair").onclick = onRepair;
  if ($("btn-helper-prepare")) $("btn-helper-prepare").onclick = prepareTranslationHelper;
  if ($("btn-helper-rescan")) $("btn-helper-rescan").onclick = rescanAfterTranslationHelper;
  if ($("btn-helper-cleanup")) $("btn-helper-cleanup").onclick = cleanupTranslationHelperFromPanel;
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
      if (ev.key === "ArrowUp") adjustUiScale(UI_SCALE_STEP);
      else if (ev.key === "ArrowDown") adjustUiScale(-UI_SCALE_STEP);
      else applyUiScale(1);
      return;
    }
    if (ev.key === "Escape") closeGuideOverlay();
  });
  window.addEventListener(
    "wheel",
    (ev) => {
      if (!ev.ctrlKey) return;
      ev.preventDefault();
      adjustUiScale(ev.deltaY < 0 ? UI_SCALE_STEP : -UI_SCALE_STEP);
    },
    { passive: false }
  );
  // 推廣連結：Discord／支持開發（含主畫面醒目的支持鈕，可重複出現）
  document.querySelectorAll(".promo-card[data-url], .support-cta[data-url]").forEach((el) => {
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
        }
      } catch (e) {
        log(String(e));
      }
    };
  }
  if ($("btn-font-out")) {
    $("btn-font-out").onclick = async () => {
      try {
        const p = await pickDir("選擇字體資源包輸出位置");
        if (p) $("font-output").value = p;
      } catch (e) {
        log(String(e));
      }
    };
  }
  if ($("btn-font-build")) {
    $("btn-font-build").onclick = async () => {
      const fontPath = ($("font-file").value || "").trim();
      const outputDir = (($('font-output')?.value || $('output')?.value || "")).trim();
      if (!fontPath) return log("請先選擇字體檔。");
      if (!outputDir) return log("請先選擇字體包的輸出位置。");
      setBusy(true);
      lastProgressLogKey = "";
      clearLog("開始建立字體資源包");
      setProgress(10, "正在建立字體資源包…");
      try {
        const r = await invoke("create_font_pack", {
          fontPath,
          outputDir,
          packName: ($("font-pack-name").value || "我的遊戲字體").trim(),
          packDesc: "自訂遊戲字體",
          fontOptions: {
            size: Number($('font-size')?.value || 11),
            weight: Number($('font-weight')?.value || 400),
            shiftX: Number($('font-shift-x')?.value || 0),
            shiftY: Number($('font-shift-y')?.value || 0.5),
            oversample: Number($('font-oversample')?.value || 4),
          },
        });
        let finalMessage = r.playerSummary || r.player_summary || JSON.stringify(r, null, 2);
        const packPath = r.packPath || r.pack_path || "";
        const shouldApplyFont = !!$("font-apply-current")?.checked;
        const instancePath = ($("instance")?.value || "").trim();
        if (shouldApplyFont) {
          if (!instancePath) {
            appendLog("未選整合包，字體包已建立但未套用。", "warn");
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
        setLogFinal(finalMessage);
      } catch (e) {
        setProgress(0, "失敗", { failed: true });
        appendLog("建立字體包失敗：");
        appendLog(String(e));
      } finally {
        setBusy(false);
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

  $("btn-inst").onclick = async () => {
    try {
      const p = await pickDir("選擇遊戲／整合包資料夾");
      if (p) {
        $("instance").value = p;
        setTranslationState("ready");
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
              (await invoke("managed_output_base").catch(() => null)) ||
              (await invoke("suggest_output_dir", { instancePath: p }).catch(() => null)) ||
              (await invoke("suggest_resourcepacks_dir", { instancePath: p }));
            if (base) {
              setAutoOutputDir(base);
              appendLog(
                "翻譯結果位置已準備好：\n" + base +
                  "\n翻譯完成會直接套用到整合包資料夾。"
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
  if ($("btn-diagnose-latest")) {
    $("btn-diagnose-latest").onclick = async () => {
      const instancePath = ($("instance").value || "").trim();
      if (!instancePath) return log("請先選擇遊戲資料夾。");
      try {
        const result = await invoke("diagnose_launch_failure", { instancePath });
        showDiagnosis(result);
      } catch (e) {
        showDiagnosis({ summary: "讀取最近記錄失敗：" + formatInvokeError(e) });
      }
    };
  }
  if ($("btn-restore")) {
    $("btn-restore").onclick = async () => {
      const instancePath = ($("instance").value || "").trim();
      if (!instancePath) return log("請先選擇遊戲資料夾。");
      if (!window.confirm("請先關閉 Minecraft。這會回到此備份對應的套用前狀態（若多次套用曾重用同一備份，可能跨過好幾次）。確定繼續？")) return;
      try {
        const result = await invoke("restore_last_apply_cmd", {
          instancePath,
          outputDir: selectedOutputDir() || null,
        });
        const summary = result.playerSummary || result.player_summary || "已還原上一次套用。";
        const warnings = result.warnings || [];
        appendLog(summary, "warn");
        if (warnings.length) appendLog("還原警告：\n" + warnings.join("\n"), "warn");
        const box = $("diagnose-result");
        if (box) {
          box.hidden = false;
          box.textContent = [summary, warnings.length ? "警告：\n" + warnings.join("\n") : ""]
            .filter(Boolean)
            .join("\n\n");
        }
        await refreshBackupState();
      } catch (e) {
        appendError("還原失敗：" + formatInvokeError(e));
        const box = $("diagnose-result");
        if (box) {
          box.hidden = false;
          box.textContent = "還原失敗：" + formatInvokeError(e);
        }
      }
    };
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
    if (!outputDir) return log("還沒選結果位置。");
    try {
      const work = resultWorkDir(outputDir);
      try {
        await invoke("open_path", { path: work });
      } catch (_) {
        await invoke("open_path", { path: outputDir });
      }
    } catch (e) {
      log(String(e));
    }
  }
  $("btn-open").onclick = () => openResultFolder(false);
  if ($("btn-open-font")) $("btn-open-font").onclick = () => openResultFolder(true);

  if ($("btn-clear-log")) {
    $("btn-clear-log").onclick = () => {
      clearLog("日誌已清除");
      const el = $("log");
      if (el) el.classList.add("log-empty");
    };
  }

  if ($("btn-open-report")) {
    $("btn-open-report").onclick = async () => {
      const outputDir = ($("output")?.value || "").trim();
      if (!outputDir) return log("還沒選結果位置。");
      const work = resultWorkDir(outputDir);
      const report = work + "\\覆蓋範圍說明.txt";
      try {
        await invoke("open_path", { path: report });
      } catch (_) {
        try {
          await invoke("open_path", { path: work });
          appendLog("尚未找到覆蓋範圍說明.txt，已改開輸出資料夾。", "warn");
        } catch (e) {
          log(String(e));
        }
      }
    };
  }

});

// ───────────────────────── 檢查更新（自足模組）─────────────────────────
// 介面由 GPT 維護；本區塊只負責把「檢查更新」接到後端，且完全防禦式：
// 有 #btn-check-update 就接上點擊；沒有也不影響其他功能。
// 契約：invoke("check_update") → { current, latest, updateAvailable, url, notes, ok, message }
//       invoke("download_update") → { path, launched, automatic, message }
(function wireUpdateChecker() {
  const _invoke =
    (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) || null;
  let latestUpdateInfo = null;
  let updateInFlight = false;

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
    if (status) status.textContent = info.message || "";
    if (!info.updateAvailable) {
      if (interactive && typeof appendLog === "function") appendLog(info.message || "已是最新版");
      return;
    }
    // 有新版：靜默檢查也提示，但只有互動時才問要不要下載
    if (typeof appendLog === "function") {
      appendLog("發現新版本 " + info.latest + "（目前 " + info.current + "）。", "warn");
      if (info.notes) appendLog("更新內容：" + info.notes);
    }
    const btn = document.getElementById("btn-check-update");
    if (btn) {
      btn.textContent = "下載新版 " + info.latest;
      btn.dataset.mode = "download";
    }
    if (interactive) {
      const ok = window.confirm(
        "發現新版本 " + info.latest + "。\n要立即下載、驗證並自動更新嗎？\n\n更新完成後工具會重新開啟。"
      );
      if (ok) await runDownload();
    }
  }

  async function openManualDownload() {
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
    const btn = document.getElementById("btn-check-update");
    const status = document.getElementById("update-status");
    if (btn) {
      btn.disabled = true;
      btn.textContent = "正在更新…";
    }
      if (status) status.textContent = "正在下載並驗證免安裝版";
    try {
      if (typeof appendLog === "function") appendLog("正在下載並驗證新版 EXE，請勿關閉工具…");
      const r = await _invoke("download_update");
      const message = (r && r.message) || "免安裝更新檔已啟動。";
      if (typeof appendLog === "function") appendLog(message);
      if (status) status.textContent = r && r.automatic ? "正在更新，稍後會重新開啟" : "請開啟下載的免安裝版完成更新";
    } catch (e) {
      if (typeof appendLog === "function") appendLog("下載更新失敗：" + String(e), "error");
      if (status) status.textContent = "自動更新失敗";
      if (latestUpdateInfo && latestUpdateInfo.url) {
        const manual = window.confirm("自動更新失敗。\n要改用瀏覽器下載官方免安裝版嗎？");
        if (manual) await openManualDownload();
      }
    } finally {
      updateInFlight = false;
      if (btn) {
        btn.disabled = false;
        btn.textContent = latestUpdateInfo && latestUpdateInfo.latest
          ? "下載新版 " + latestUpdateInfo.latest
          : "檢查更新";
      }
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
    // 啟動時安靜檢查一次（不打擾，只更新狀態文字／按鈕標籤）
    runUpdateCheck(false);
  }

  if (document.readyState === "loading") {
    window.addEventListener("DOMContentLoaded", attach);
  } else {
    attach();
  }
  // 讓 UI 端若想手動觸發也有入口
  window.zfCheckUpdate = () => runUpdateCheck(true);
})();
