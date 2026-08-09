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

/** Linear 步驟：依百分比與訊息對應 */
const STEP_ORDER = ["prep", "scan", "polish", "ai", "pack", "done"];

function stepFromProgress(percent, message) {
  const p = Number(percent) || 0;
  const m = String(message || "");
  if (p <= 0) return null;
  if (p >= 100 || /全部完成|補翻完成|字體包完成/.test(m)) return "done";
  if (/失敗|錯誤/.test(m) && p === 0) return "error";
  if (/字體/.test(m)) return p >= 90 ? "pack" : "prep";
  if (p < 6 || /準備中|啟動|讀取上次|工作階段/.test(m)) return "prep";
  // 本地蒐集／整理（進度文案會帶「本地」）
  if (p < 41 || /本地整理|本地蒐集|讀模組|資源包|KubeJS|OpenCC|詞典|合併/.test(m)) {
    if (p >= 33 || /合併|OpenCC|詞典|快捷選單|整理完成/.test(m)) return "polish";
    return "scan";
  }
  if (p < 90 || /純 AI|AI 階段|AI 翻譯|補約|缺漏/.test(m)) return "ai";
  return "pack";
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
          baseMsg + " · 已進行 " + elapsed + " · 仍在運作，請稍候";
        setProgBarWorking(true);
      } else {
        msgEl.textContent = baseMsg + " · 已進行 " + elapsed;
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
    const elapsed =
      progressBusy && progressStartedAt
        ? " · 已進行 " + formatElapsed(Date.now() - progressStartedAt)
        : "";
    $("prog-msg").textContent = message + elapsed;
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

async function onApply() {
  const instancePath = ($("instance").value || "").trim();
  const outputDir = ($("output").value || "").trim();
  if (!instancePath) return log("套用需要「遊戲資料夾」。");
  if (!outputDir) return log("套用需要「翻譯結果」位置。");
  setBusy(true);
  lastProgressLogKey = "";
  clearLog("套用到遊戲");
  appendLog("請先完全關閉 Minecraft。", "warn");
  setProgress(5, "套用中…");
  try {
    const result = await invoke("apply_translation_to_game", {
      instancePath,
      outputDir,
      packName: ($("pack-name").value || "").trim() || null,
    });
    setProgress(100, "套用完成");
    setLogFinal(result.playerSummary || result.player_summary || JSON.stringify(result, null, 2));
  } catch (e) {
    setProgress(0, "套用失敗", { failed: true });
    appendError("套用失敗");
    appendError(formatInvokeError(e));
  } finally {
    setBusy(false);
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
    "btn-apply",
    "btn-inst",
    "btn-out",
    "btn-save-adv",
    "use-ai",
    "btn-font-pick",
    "btn-font-out",
    "btn-font-build",
    "font-size",
    "font-weight",
    "font-shift-x",
    "font-shift-y",
    "font-oversample",
    "btn-glossary",
    "minimize-on-close",
    "target-version",
    "tab-translate",
    "tab-font",
  ].forEach((id) => {
    const el = $(id);
    if (el) el.disabled = busy;
  });
  // 翻譯中鎖定路徑與資源包名稱（不可改）
  ["pack-name", "instance", "output", "font-pack-name", "font-file", "font-output"].forEach((id) => {
    const el = $(id);
    if (el) {
      el.readOnly = !!busy;
      el.setAttribute("aria-readonly", busy ? "true" : "false");
    }
  });
  const guide = $("btn-guide");
  if (guide) guide.disabled = false;
}

/** 分頁：translate | font */
function showAppPage(page) {
  const name = page === "font" ? "font" : "translate";
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
  document.body.dataset.appPage = name;
}

async function pickDir(title) {
  if (!dialog.open) throw new Error("無法開啟資料夾選擇視窗");
  const selected = await dialog.open({ directory: true, multiple: false, title });
  return typeof selected === "string" ? selected : null;
}

function syncAiPanel() {
  const panel = $("ai-panel");
  const enabled = !!$("use-ai")?.checked;
  if (panel) {
    panel.style.display = enabled ? "block" : "none";
    panel.setAttribute("aria-hidden", enabled ? "false" : "true");
  }
}

async function refreshAiStatus() {
  const statusEl = $("key-status");
  const noteEl = $("ai-source-note");
  const statusRow = statusEl?.closest(".ai-status");
  if (!statusEl) return;
  try {
    const s = await invoke("ai_status");
    const ready = s && s.ready !== false;
    const usingOwnKey = !!(s && (s.usingOwnKey || s.using_own_key));
    const managedFree = !!(s && (s.managedFree || s.managed_free));
    const message = String(s && s.message ? s.message : "").trim();
    statusEl.textContent = ready
      ? usingOwnKey
        ? "AI：使用自己的金鑰"
        : managedFree
          ? "AI：開發者代管翻譯可用"
          : "AI：目前可用"
      : "AI：目前無法使用";
    if (statusRow) statusRow.dataset.state = ready ? (usingOwnKey ? "own" : "managed") : "error";
    if (noteEl) {
      noteEl.textContent = message || (ready
        ? "可直接使用 AI；自備金鑰可在進階設定中填寫。"
        : "請稍後再試，或在進階設定確認自己的 API 設定。");
    }
  } catch (e) {
    statusEl.textContent = "AI：狀態暫時無法確認";
    if (statusRow) statusRow.dataset.state = "error";
    if (noteEl) noteEl.textContent = "不影響簡繁轉換；需要補翻英文時請稍後再確認。";
  }
}

async function refreshApiSettings() {
  try {
    const s = await invoke("get_api_settings");
    // 不在介面顯示具體服務商網址
    const bu = (s.baseUrl || s.base_url || "").trim();
    $("base-url").value = /deepseek/i.test(bu) ? "" : bu;
  } catch (e) {
    /* AI 狀態由 refreshAiStatus 顯示；設定讀取失敗不阻擋本機翻譯。 */
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

async function onSaveAdv() {
  try {
    await invoke("save_api_settings_cmd", {
      apiKey: ($("api-key").value || "").trim(),
      baseUrl: ($("base-url").value || "").trim(),
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
  const outputDir = ($("output").value || "").trim();
  if (!instancePath) return log("請先選擇「遊戲資料夾」。");
  if (!outputDir) return log("請先選擇「翻譯結果放哪」。");

  const useAi = !!$("use-ai").checked;
  let targetVersion = ($("target-version")?.value || "").trim();
  if (!targetVersion) {
    targetVersion = (await detectVersionForInstance(instancePath, true)) || "";
  }

  setBusy(true);
  lastProgressLogKey = "";
  clearLog("開始翻譯");
  appendLog("完成後請關閉遊戲，再按「套用到遊戲」。");
  setProgress(1, "準備中…");

  try {
    const result = await invoke("one_click_translate", {
      instancePath,
      outputDir,
      packName: ($("pack-name").value || "繁體中文翻譯").trim(),
      useAi,
      referencePack: null,
      targetVersion: targetVersion || null,
    });
    setProgress(100, "全部完成！");
    let msg = result.playerSummary || result.player_summary || JSON.stringify(result, null, 2);
    if (result.minemenuMsg || result.minemenu_msg) {
      msg += "\n\n" + (result.minemenuMsg || result.minemenu_msg);
    }
    setLogFinal(msg);
  } catch (e) {
    handleRunFailure(e, "翻譯失敗");
    if (!isCancellation(e)) {
      appendLog("可把上方錯誤訊息留下來方便排查。");
    }
  } finally {
    setBusy(false);
  }
}

/** 修復：重建 zip／對齊工作階段；可選一併 AI 補缺。不修世界閃退。 */
async function onRepair() {
  const outputDir = ($("output").value || "").trim();
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
  if (useAi) {
    try {
      const ai = await invoke("ai_status");
      if (ai && ai.ready === false) {
        appendLog("AI 目前無法使用：仍可修復既有檔案，但不會補翻新的文字。", "warn");
      }
    } catch (_) {
      /* 修復本身仍可交給後端處理 */
    }
  }

  setBusy(true);
  lastProgressLogKey = "";
  clearLog("開始修復");
  appendLog("這不能修好「進世界閃退」。");
  setProgress(2, "準備修復…");

  try {
    const result = await invoke("repair_translation_pack", {
      outputDir,
      useAi,
    });
    setProgress(100, "修復完成！");
    setLogFinal(result.playerSummary || result.player_summary || JSON.stringify(result, null, 2));
  } catch (e) {
    handleRunFailure(e, "修復失敗");
  } finally {
    setBusy(false);
  }
}

/** 只補缺漏：不重掃 mods，讀工作階段 + AI */
async function onSupplement() {
  const outputDir = ($("output").value || "").trim();
  if (!outputDir) {
    return log("請選與上次相同的「翻譯結果」位置。");
  }
  try {
    const ai = await invoke("ai_status");
    if (ai && ai.ready === false) {
      return log("目前沒有可用的 AI 翻譯，請稍後再試或檢查進階設定。");
    }
  } catch (_) {
    return log("目前無法確認 AI 狀態，請稍後再試。");
  }
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
  lastProgressLogKey = "";
  clearLog("開始再補一些");
  setProgress(3, "準備中…");

  try {
    if (!$("use-ai").checked) {
      $("use-ai").checked = true;
      syncAiPanel();
    }
    const result = await invoke("supplement_translate", { outputDir });
    setProgress(100, "補譯完成！");
    let msg = result.playerSummary || result.player_summary || JSON.stringify(result, null, 2);
    setLogFinal(msg);
  } catch (e) {
    handleRunFailure(e, "再補一些失敗");
  } finally {
    setBusy(false);
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

window.addEventListener("DOMContentLoaded", async () => {
  initTheme();
  syncAiPanel();
  refreshApiSettings();
  refreshAiStatus();
  loadUiPrefs();
  setProgress(0, "尚未開始");
  showAppPage("translate");
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
  if ($("tab-font")) {
    $("tab-font").onclick = () => showAppPage("font");
  }
  if ($("btn-theme")) {
    $("btn-theme").onclick = () => {
      const current = document.documentElement.dataset.theme === "light" ? "light" : "dark";
      applyTheme(current === "dark" ? "light" : "dark");
    };
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
      if (level === "error") appendError(message);
      else if (level === "warn") appendLog(message, "warn");
      else appendLog(message, "info");
    });
  } catch (e) {
    /* 略 */
  }

  $("use-ai").onchange = syncAiPanel;
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
  if ($("btn-apply")) $("btn-apply").onclick = onApply;
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
  // Esc 關閉說明
  window.addEventListener("keydown", (ev) => {
    if (ev.key === "Escape") closeGuideOverlay();
  });
  if ($("btn-font-pick")) {
    $("btn-font-pick").onclick = async () => {
      try {
        if (!dialog.open) throw new Error("無法開啟檔案選擇");
        const f = await dialog.open({
          multiple: false,
          filters: [{ name: "字體", extensions: ["ttf", "otf", "ttc"] }],
          title: "選擇你喜歡的字體檔",
        });
        if (typeof f === "string") $("font-file").value = f;
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
        setProgress(100, "字體包完成");
        setLogFinal(r.playerSummary || r.player_summary || JSON.stringify(r, null, 2));
      } catch (e) {
        setProgress(0, "失敗", { failed: true });
        appendLog("建立字體包失敗：");
        appendLog(String(e));
      } finally {
        setBusy(false);
      }
    };
  }

  $("btn-inst").onclick = async () => {
    try {
      const p = await pickDir("選擇遊戲／整合包資料夾");
      if (p) {
        $("instance").value = p;
        await detectVersionForInstance(p, false);
        if (!($("output").value || "").trim()) {
          try {
            const base =
              (await invoke("suggest_output_dir", { instancePath: p }).catch(() => null)) ||
              (await invoke("suggest_resourcepacks_dir", { instancePath: p }));
            $("output").value = base;
            appendLog("已建議結果位置：\n" + base);
          } catch (_) {
            /* 略 */
          }
        }
      }
    } catch (e) {
      log(String(e));
    }
  };
  $("btn-out").onclick = async () => {
    try {
      const p = await pickDir("選擇翻譯結果要放的資料夾");
      if (p) $("output").value = p;
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
        $("output").value = p;
        appendLog("已建議結果位置：\n" + p);
      } catch (e) {
        log("無法建議路徑：\n" + String(e));
      }
    };
  }
  $("btn-open").onclick = async () => {
    const outputInput = document.body.dataset.appPage === "font" ? $("font-output") : $("output");
    const outputDir = (outputInput?.value || "").trim();
    if (!outputDir) return log("還沒選結果位置。");
    try {
      // 優先開「翻譯結果」
      const work = outputDir.replace(/[\\/]+$/, "") + "\\翻譯結果";
      try {
        await invoke("open_path", { path: work });
      } catch (_) {
        await invoke("open_path", { path: outputDir });
      }
    } catch (e) {
      log(String(e));
    }
  };

  // 推廣連結：雲端／工具箱／Discord
  document.querySelectorAll(".promo-card[data-url]").forEach((el) => {
    el.addEventListener("click", async () => {
      const url = el.getAttribute("data-url");
      if (!url) return;
      try {
        await invoke("open_url", { url });
      } catch (e) {
        // 備援：系統預設瀏覽器
        try {
          window.open(url, "_blank");
        } catch (e2) {
          log("無法開啟連結：" + String(e));
        }
      }
    });
  });
});

// ───────────────────────── 檢查更新（自足模組）─────────────────────────
// 介面由 GPT 維護；本區塊只負責把「檢查更新」接到後端，且完全防禦式：
// 有 #btn-check-update 就接上點擊；沒有也不影響其他功能。
// 契約：invoke("check_update") → { current, latest, updateAvailable, url, notes, ok, message }
//       invoke("download_update") → { path, launched, message }
(function wireUpdateChecker() {
  const _invoke =
    (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) || null;

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
        "發現新版本 " + info.latest + "。\n要現在下載安裝檔嗎？（下載後需手動點一次完成安裝）"
      );
      if (ok) await runDownload();
    }
  }

  async function runDownload() {
    if (!_invoke) return;
    try {
      if (typeof appendLog === "function") appendLog("正在下載新版安裝檔…");
      const r = await _invoke("download_update");
      if (typeof appendLog === "function") appendLog((r && r.message) || "已下載。");
    } catch (e) {
      if (typeof appendLog === "function") appendLog("下載更新失敗：" + String(e), "error");
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
