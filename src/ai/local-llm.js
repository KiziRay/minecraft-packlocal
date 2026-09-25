import { $, dialog, invoke, listen } from "../core/dom.js";
import { LOCAL_LLM_CONSENT_KEY, LOCAL_LLM_DIR_KEY } from "../core/storage.js";
import { confirmDialog } from "../ui/confirm.js";
import { isRunning, runExclusive } from "../ui/once.js";
import { GPT_COPY } from "./copy.js";

const MB = 1024 * 1024;
const PROBE_CACHE_KEY = "modpack-i18n-local-llm-probe-v1";

let lastProbeView = null;
let probeInFlight = null;
let lastInstallError = "";

function formatGb(bytes) {
  const n = Number(bytes) || 0;
  if (n <= 0) return "";
  const gb = n / (1024 * MB);
  if (gb >= 10) return `${Math.round(gb)} GB`;
  return `${gb.toFixed(1)} GB`;
}

/** 0＝不知道，就回空字串。舊版 `Math.max(1, …)` 會把「未知」硬湊成「1 MB」。 */
function formatMb(bytes) {
  const n = Number(bytes) || 0;
  if (n <= 0) return "";
  if (n >= 1024 * MB) return formatGb(n);
  return `${Math.max(1, Math.round(n / MB))} MB`;
}

function planKnown(view) {
  const flag = view?.planKnown ?? view?.plan_known;
  if (flag != null) return !!flag;
  // 舊版後端沒有這個欄位時，用「有沒有算出大小」推。
  return Number(view?.needBytes ?? view?.need_bytes ?? 0) > 0;
}

function escapeHtml(value) {
  return String(value || "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function savedDir() {
  try {
    return String(localStorage.getItem(LOCAL_LLM_DIR_KEY) || "").trim();
  } catch (_) {
    return "";
  }
}

function setSavedDir(dir) {
  try {
    if (dir) localStorage.setItem(LOCAL_LLM_DIR_KEY, dir);
  } catch (_) {
    /* ignore */
  }
}

function hasConsent() {
  try {
    return localStorage.getItem(LOCAL_LLM_CONSENT_KEY) === "1";
  } catch (_) {
    return false;
  }
}

function setConsent(ok) {
  try {
    if (ok) localStorage.setItem(LOCAL_LLM_CONSENT_KEY, "1");
    else localStorage.removeItem(LOCAL_LLM_CONSENT_KEY);
  } catch (_) {
    /* ignore */
  }
}

function overlay() {
  return $("local-llm-overlay");
}

function setHidden(el, hidden) {
  if (!el) return;
  el.hidden = !!hidden;
  el.setAttribute("aria-hidden", hidden ? "true" : "false");
}

function setProgress(percent, message) {
  const bar = $("local-llm-progress-bar");
  const label = $("local-llm-progress-label");
  const num = Math.max(0, Math.min(100, Number(percent) || 0));
  if (bar) bar.style.width = `${num}%`;
  if (label) label.textContent = message || `${num}%`;
  const wrap = $("local-llm-progress-wrap");
  setHidden(wrap, false);
}

function cacheProbe(view) {
  lastProbeView = view || null;
  try {
    // 傳 null／falsy 代表「清掉快取」（例如剛刪除本地模型檔案）：連 sessionStorage
    // 裡的舊值也要一起清，否則 readCachedProbe() 會從 storage 撿回已經過期的內容。
    if (view) sessionStorage.setItem(PROBE_CACHE_KEY, JSON.stringify(view));
    else sessionStorage.removeItem(PROBE_CACHE_KEY);
  } catch (_) {
    /* ignore */
  }
}

function readCachedProbe() {
  if (lastProbeView) return lastProbeView;
  try {
    const raw = sessionStorage.getItem(PROBE_CACHE_KEY);
    if (!raw) return null;
    lastProbeView = JSON.parse(raw);
    return lastProbeView;
  } catch (_) {
    return null;
  }
}

/**
 * 「這個資料夾裡有什麼」的狀態。
 *
 * 使用者實測回報：把存放資料夾改成一個沒有模型的位置，畫面還是顯示
 * 「已就緒」並給出「關閉並開始翻譯」按鈕。原因是舊版的 ready／installed 是
 * **全域**狀態（llama-server 正在跑就算 ready），跟輸入框裡指的資料夾無關；
 * 探測摘要又從快取重畫，於是「檔案放哪」顯示的還是上一個資料夾。
 *
 * 這裡改成把資料夾本身納入狀態：畫面只信任「探測過的那個資料夾」的結果，
 * 路徑一改就作廢，直到重新探測完成為止。
 */
let dirState = { dir: "", installed: false, checked: false };

function dirStateMatchesInput() {
  return dirState.checked && dirState.dir === currentDir();
}

function invalidateDirState(dir) {
  dirState = { dir: String(dir || ""), installed: false, checked: false };
  // 換資料夾等於換一個「這台電腦上的安裝」，舊的探測摘要一律作廢，
  // 否則畫面會拿上一個資料夾的內容來描述新資料夾。
  cacheProbe(null);
}

/**
 * 「將如何安裝」。
 *
 * 還沒拿到遠端清單時（未登入 Discord）什麼都還不知道，就照實說，不要編造大小。
 * 舊版無條件自己組字串、忽略後端給的 `message`，配上 `formatMb(0)` 硬湊出的「1 MB」，
 * 畫面會很有自信地講一句完全錯的話。
 */
function installHowCopy(view) {
  if (!planKnown(view)) {
    return String(view?.message || "").trim() ||
      "還需要先登入 Discord 才能取得下載清單，因此目前無法估算下載大小。";
  }
  const gpu = String(view.gpuName || view.gpu_name || "").trim();
  const need = formatMb(view.needBytes || view.need_bytes || 0);
  const size = need ? `約占 ${need}（含解壓所需空間）` : "大小待確認";
  if (gpu) {
    return `這張顯示卡適合用來加速翻譯。將下載一套對應的翻譯模型與執行程式，${size}。只裝一套，空間不夠就不會下載。`;
  }
  return `未偵測到可用顯示卡，將改用處理器來跑翻譯，速度會比較慢。將下載一套翻譯模型與執行程式，${size}。空間不夠就不會下載。`;
}

function setProbeSummary(view) {
  const el = $("local-llm-probe-summary");
  if (!el || !view) return;
  const gpu = String(view.gpuName || view.gpu_name || "").trim();
  const vram = formatGb(view.vramBytes || view.vram_bytes || 0);
  const ram = formatGb(view.ramBytes || view.ram_bytes || 0);
  const dir = String(view.installDir || view.install_dir || currentDir() || "").trim();
  const gpuLine = gpu
    ? `顯示卡：${escapeHtml(gpu)}`
    : "顯示卡：未偵測到可用的顯示卡";
  // 讀不到顯示記憶體就整行不顯示。使用者不需要知道我們讀不到——那不影響能不能跑，
  // 寫出來只會讓人以為出了問題。
  const vramLine = gpu && vram ? `顯示記憶體：約 ${escapeHtml(vram)}` : "";
  const ramLine = ram ? `系統記憶體：約 ${escapeHtml(ram)}` : "";
  const pathHtml = dir
    ? `<button type="button" class="local-llm-path-btn" id="btn-local-llm-open-dir">${escapeHtml(dir)}</button>`
    : "尚未選擇資料夾";
  const known = planKnown(view);
  const movedNotice = String(view.movedNotice || view.moved_notice || "").trim();
  el.innerHTML = `
    ${movedNotice ? `<p class="local-llm-moved-notice">${escapeHtml(movedNotice)}</p>` : ""}
    <h3>這台電腦</h3>
    <p>${gpuLine}</p>
    ${vramLine ? `<p>${vramLine}</p>` : ""}
    ${ramLine ? `<p>${ramLine}</p>` : ""}
    <h3>${known ? "將如何安裝" : "還需要一步"}</h3>
    <p>${escapeHtml(installHowCopy(view))}</p>
    <h3>檔案放哪</h3>
    <p>${pathHtml}</p>
    <p>檔案只在這台電腦。之後可自行刪除整個資料夾。</p>
  `;
  setHidden(el, false);
  const openBtn = $("btn-local-llm-open-dir");
  if (openBtn) {
    openBtn.onclick = () => {
      const path = currentDir() || dir;
      if (!path) return;
      invoke("open_path", { path }).catch(() => {});
    };
  }
}

function showInstallButton(show) {
  setHidden($("btn-local-llm-install"), !show);
}

/**
 * 一則錯誤只留兩個出口：面板 inline 一份、overlay 一份（overlay 開著時使用者只看得到它）。
 *
 * 舊版還會同時蓋掉 `#local-llm-note`（說明文案）與 `#local-llm-title`，再加上
 * refreshAiStatus 同步到 `#ai-source-note`、`#key-status`，以及 catch 裡的 appendLog——
 * 同一句話出現六次，而且把「這個欄位本來在教我什麼」的參照吃掉了。
 */
function setInstallError(message) {
  lastInstallError = String(message || "").trim();
  const panelErr = $("local-llm-error");
  const overlayErr = $("local-llm-overlay-error");
  const note = $("local-llm-note");
  const setup = $("btn-local-llm-setup");
  const retry = $("btn-local-llm-install");
  if (panelErr) {
    panelErr.textContent = lastInstallError;
    setHidden(panelErr, !lastInstallError);
  }
  if (overlayErr) {
    overlayErr.textContent = lastInstallError;
    setHidden(overlayErr, !lastInstallError);
  }
  // 說明文案永遠是說明文案，不當錯誤欄用。
  if (note) note.textContent = GPT_COPY.noteLocal;
  if (setup) setup.textContent = lastInstallError ? "再試一次" : "同意並偵測";
  if (retry) retry.textContent = lastInstallError ? "再試一次" : "開始下載";
}

export function closeLocalLlmOverlay() {
  setHidden(overlay(), true);
}

/**
 * overlay 該不該顯示「已就緒」而不是「同意並偵測」的下載流程。
 *
 * 舊版只認 `ready`（服務這一刻正在跑）——工具剛重開時服務還沒被叫起來，
 * 明明檔案早就裝好，也會被判定成「尚未就緒」，逼使用者重新走一次同意／下載
 * 流程。這裡改成也認 `installed`（檔案在，不管服務現在有沒有在跑）：
 * 真正把服務叫起來的動作交給 `ensureLocalLlmReady()`（按「關閉並開始翻譯」
 * 或直接按「開始翻譯」時才觸發），這裡只負責「不要嚇使用者以為要重裝」。
 */
function isReadyView(view) {
  return !!(view && (view.ready === true || view.local_ready === true || view.installed === true));
}

function syncConsentUi() {
  const agreed = !!$("local-llm-agree")?.checked;
  const cached = readCachedProbe();
  if (isReadyView(cached)) {
    // 已經裝好就別再擺出下載流程——那讓人以為每次都要重裝一次。
    setProbeSummary(cached);
    showInstalledState(true);
    return;
  }
  showInstalledState(false);
  if (agreed && cached) {
    setProbeSummary(cached);
    // 清單還沒拿到就不給「開始下載」——按下去只會撞同一道 Discord 門檻。
    showInstallButton(planKnown(cached));
  } else {
    showInstallButton(false);
  }
}

export async function openLocalLlmOverlay() {
  const root = overlay();
  if (!root) return;
  showInstalledState(false);

  // 關掉 overlay 完全不影響下載：安裝是 Tauri 背景指令，跟這塊 UI 的生死無關。
  // 但舊版重新打開時不知道這件事，會把畫面重置回「同意並偵測」的起始畫面，
  // 使用者以為下載停了、又點一次「開始下載」——runExclusive 會擋掉重複點擊，
  // 不會真的重下，但畫面看起來像壞掉一樣。這裡改成：安裝仍在跑，就直接顯示
  // 目前進度，不要假裝什麼都沒發生過。
  if (isRunning("local-llm-install")) {
    setHidden($("local-llm-progress-wrap"), false);
    setHidden(root, false);
    return;
  }

  // 先問一次後端「現在到底裝好了沒」，不要只靠這個 session 的快取判斷。
  try {
    const status = await localLlmStatus();
    if (localLlmReady(status) || status?.installed) {
      cacheProbe({ ...(readCachedProbe() || {}), ...status });
    }
  } catch (_) {
    /* 讀不到就照原本流程走 */
  }
  const dirInput = $("local-llm-dir");
  if (dirInput && !dirInput.value) dirInput.value = savedDir();
  const agree = $("local-llm-agree");
  if (agree) agree.checked = hasConsent();
  setHidden($("local-llm-progress-wrap"), !lastInstallError);
  const cached = readCachedProbe();
  if (cached) setProbeSummary(cached);
  else setHidden($("local-llm-probe-summary"), true);
  syncConsentUi();
  setInstallError(lastInstallError);
  setHidden(root, false);
  if (agree?.checked && !cached && !probeInFlight) {
    probeLocalLlm().catch((e) => setInstallError(String(e?.message || e)));
  }
}

async function pickDir() {
  if (!dialog.open) throw new Error("無法開啟資料夾選擇視窗");
  const selected = await dialog.open({
    directory: true,
    multiple: false,
    title: "選擇本地模型存放資料夾",
  });
  if (!selected) return;
  const path = Array.isArray(selected) ? selected[0] : selected;
  if ($("local-llm-dir")) $("local-llm-dir").value = String(path || "");
  setSavedDir(String(path || ""));
  invalidateDirState(path);
  syncReadyActions();
  await reportDirContents(String(path || ""));
}

/**
 * 依「目前資料夾的探測結果」決定要不要顯示「關閉並開始翻譯」。
 *
 * 這顆按鈕代表「模型就在這個資料夾、可以直接用」，所以只有在探測過、
 * 而且探測的就是輸入框現在指的那個資料夾時才該出現。
 */
function syncReadyActions() {
  const ready = dirStateMatchesInput() && dirState.installed;
  showInstalledState(ready);
}

/**
 * 換資料夾之後，就地告訴使用者「這個資料夾裡有沒有模型」。
 *
 * 刻意不彈對話框：使用者剛按完「瀏覽…」，再跳一個視窗問問題就是多餘的阻擋。
 * 面板內就地更新標題與按鈕，看得到、不擋路。
 */
async function reportDirContents(dir) {
  if (!dir) return;
  const title = $("local-llm-title");
  const install = $("btn-local-llm-install");
  if (title) title.textContent = "正在檢查這個資料夾…";
  setDirStatusLine("正在檢查這個資料夾裡有沒有模型…");
  try {
    // 只查「這個資料夾裡有什麼」，不重新列舉硬體——硬體不會因為換資料夾而變，
    // 每次都重測是使用者反映「偵測很慢」的主因。
    const status = await invoke("local_llm_status_cmd", { installDir: dir });
    const installed = !!(status && status.installed);
    dirState = { dir: String(dir), installed, checked: true };
    if (title) {
      title.textContent = installed
        ? "這個資料夾已有模型，可以直接使用"
        : "這個資料夾還沒有模型";
    }
    setDirStatusLine(
      installed
        ? "已找到模型檔案，可以直接開始翻譯。"
        : "這個資料夾裡沒有模型，按下面的按鈕就會下載到這裡。"
    );
    if (install) {
      install.textContent = installed ? "重新下載到這個資料夾" : "下載到這個資料夾";
      setHidden(install, installed);
    }
    setInstallError("");
  } catch (e) {
    // 探測失敗（權限、磁碟拔除、路徑無效）要講清楚是「讀不到」而不是「沒有模型」，
    // 否則使用者會以為要重下載幾 GB。
    dirState = { dir: String(dir), installed: false, checked: true };
    if (title) title.textContent = "無法讀取這個資料夾";
    setDirStatusLine("讀不到這個資料夾，請確認磁碟還在、或換一個位置。");
    setInstallError(
      "讀不到這個資料夾（" + String(e?.message || e) + "）。請確認磁碟還在、路徑有讀寫權限，或換一個資料夾。"
    );
  }
  syncReadyActions();
}

/**
 * 目前狀態一行字。
 *
 * 使用者要的是「現在在哪一步」，不是顯示卡型號與記憶體大小——那些移到
 * 收合的詳細資訊裡，預設不佔畫面。
 */
function setDirStatusLine(text) {
  const el = $("local-llm-dir-status");
  if (!el) return;
  el.textContent = String(text || "");
  setHidden(el, !text);
}

function currentDir() {
  return String($("local-llm-dir")?.value || savedDir() || "").trim();
}

function requireAgree() {
  if (!$("local-llm-agree")?.checked) {
    throw new Error("請先勾選同意注意事項。");
  }
  setConsent(true);
}

/**
 * 偵測期間的畫面狀態。
 *
 * 硬體偵測要跑 nvidia-smi ＋ 兩次 PowerShell（WMI、登錄檔），大約 2–4 秒。
 * 舊版這段時間畫面上什麼都沒有：沒有進度、沒有「偵測中」、也還沒出現「開始下載」，
 * 看起來就是卡住了。
 */
function setProbingState(on) {
  const setup = $("btn-local-llm-setup");
  if (setup) {
    setup.disabled = !!on; // 偵測中不接受再次點擊，避免重複觸發硬體偵測
    if (on) setup.textContent = "偵測中…";
  }
  const summary = $("local-llm-probe-summary");
  if (on && summary) {
    summary.innerHTML = `
      <h3>正在檢查這台電腦</h3>
      <p>讀取處理器、記憶體與顯示卡資訊，大約需要幾秒鐘…</p>`;
    setHidden(summary, false);
  }
  if (on) {
    setProgress(0, "正在檢查這台電腦…");
    const bar = $("local-llm-progress-bar");
    if (bar) bar.classList.add("indeterminate");
  } else {
    const bar = $("local-llm-progress-bar");
    if (bar) bar.classList.remove("indeterminate");
    setHidden($("local-llm-progress-wrap"), true);
    if (setup) setup.disabled = false;
  }
}

export async function probeLocalLlm() {
  requireAgree();
  if (probeInFlight) return probeInFlight;
  const cached = readCachedProbe();
  if (cached) {
    setProbeSummary(cached);
    showInstallButton(planKnown(cached));
    return cached;
  }
  setProbingState(true);
  probeInFlight = (async () => {
    const view = await invoke("local_llm_probe_cmd", { installDir: currentDir() || null });
    if (view && (view.installDir || view.install_dir)) {
      setSavedDir(String(view.installDir || view.install_dir));
      if ($("local-llm-dir")) $("local-llm-dir").value = String(view.installDir || view.install_dir);
    }
    cacheProbe(view);
    setProbeSummary(view);
    if ($("local-llm-agree")?.checked) showInstallButton(planKnown(view));
    return view;
  })();
  try {
    return await probeInFlight;
  } finally {
    probeInFlight = null;
    setProbingState(false);
    const setup = $("btn-local-llm-setup");
    if (setup) setup.textContent = lastInstallError ? "再試一次" : "重新偵測";
  }
}

/** 安裝完成：把 overlay 切成明確的「好了」狀態，而不是把進度條停在 100% 就沒下文。 */
function showInstalledState(on) {
  setHidden($("local-llm-done"), !on);
  setHidden($("btn-local-llm-start-translate"), !on);
  if (on) showInstallButton(false);
  const setup = $("btn-local-llm-setup");
  if (setup && on) setup.textContent = "重新偵測";
}

export async function installLocalLlm() {
  requireAgree();
  setProgress(2, "開始依這台電腦準備檔案…");
  await probeLocalLlm();
  const view = await invoke("local_llm_install_cmd", { installDir: currentDir() || null });
  setProgress(100, String((view && view.message) || "本地模型可以使用。"));
  cacheProbe(view);
  showInstalledState(true);
  return view;
}

export async function localLlmStatus() {
  try {
    return await invoke("local_llm_status_cmd");
  } catch (_) {
    return { installed: false, ready: false, message: "無法讀取本地模型狀態。" };
  }
}

export function localLlmReady(status) {
  return !!(status && (status.ready || status.localReady || status.local_ready));
}

/**
 * 讓「同意並偵測」按鈕的文字在使用者點開 overlay 之前就對。
 *
 * 舊版只有 `showInstalledState()`／`probeLocalLlm()`（都要先點開 overlay 才會跑）
 * 會校正這顆按鈕的文字，剛開工具時即使後端早就就緒，按鈕仍寫著「同意並偵測」，
 * 跟旁邊「本地模型可以使用」的狀態互相矛盾。
 *
 * 有 `lastInstallError` 時不覆蓋——那代表使用者正卡在一個錯誤上，
 * 按鈕此刻該顯示「再試一次」，不能被一次不相關的狀態刷新悄悄改掉。
 */
export function syncSetupButtonLabel(ready) {
  if (lastInstallError) return;
  const setup = $("btn-local-llm-setup");
  if (setup && !setup.disabled) {
    setup.textContent = ready ? "重新偵測" : "同意並偵測";
  }
}

export function wireLocalLlm({ refreshAiStatus, appendLog, onReadyToTranslate } = {}) {
  const log = typeof appendLog === "function" ? appendLog : () => {};
  if ($("btn-local-llm-start-translate")) {
    $("btn-local-llm-start-translate").onclick = () => {
      closeLocalLlmOverlay();
      if (typeof onReadyToTranslate === "function") onReadyToTranslate();
    };
  }
  if ($("btn-local-llm-setup")) {
    $("btn-local-llm-setup").onclick = () => openLocalLlmOverlay();
  }
  if ($("btn-local-llm-overlay-close")) {
    $("btn-local-llm-overlay-close").onclick = closeLocalLlmOverlay;
  }
  if ($("btn-local-llm-pick-dir")) {
    $("btn-local-llm-pick-dir").onclick = () => pickDir().catch((e) => log(String(e?.message || e), "warn"));
  }
  const agree = $("local-llm-agree");
  if (agree) {
    agree.onchange = () => {
      const on = !!agree.checked;
      if (on) setConsent(true);
      else setConsent(false);
      syncConsentUi();
      if (on && !readCachedProbe() && !probeInFlight) {
        probeLocalLlm().catch((e) => {
          const detail = String(e?.message || e);
          setInstallError(detail);
          log("偵測失敗：" + detail, "warn");
        });
      }
    };
  }
  if ($("btn-local-llm-install")) {
    // 幾 GB 的下載，連點兩下會變成兩個程序互寫同一個檔案（rules/50 R50-4）。
    $("btn-local-llm-install").onclick = () =>
      runExclusive("local-llm-install", async () => {
        try {
          setInstallError("");
          await installLocalLlm();
          if (typeof refreshAiStatus === "function") await refreshAiStatus();
        } catch (e) {
          const detail = String(e?.message || e);
          setInstallError(detail);
          log("安裝失敗：" + detail, "warn");
        }
      }, { onBusy: () => log("安裝已經在進行中，請稍候。", "warn") });
  }
  if ($("btn-local-llm-stop")) {
    $("btn-local-llm-stop").onclick = async () => {
      try {
        await invoke("local_llm_stop_cmd");
        log("已停用本地模型，記憶體與顯示記憶體已釋放。下次翻譯會自動重新啟動。", "info");
      } catch (e) {
        log("停用本地模型失敗：" + String(e?.message || e), "warn");
      }
      if (typeof refreshAiStatus === "function") await refreshAiStatus();
    };
  }
  if ($("btn-local-llm-delete")) {
    $("btn-local-llm-delete").onclick = () =>
      runExclusive("local-llm-delete", async () => {
        const dir = currentDir();
        const ok = await confirmDialog({
          title: "刪除本地模型檔案？",
          body:
            "會刪除已下載的翻譯模型與執行程式，釋放磁碟空間。之後要用本地模型翻譯，\n" +
            "需要重新下載。不會影響已經翻好的翻譯結果。",
          affected: dir ? [dir] : [],
          danger: true,
          confirmLabel: "刪除",
          cancelLabel: "取消",
        });
        if (!ok) return;
        try {
          const msg = await invoke("local_llm_delete_cmd", { installDir: dir || null });
          log(String(msg || "已刪除。"), "info");
          cacheProbe(null);
          lastInstallError = "";
          showInstalledState(false);
          showInstallButton(false);
          setHidden($("local-llm-probe-summary"), true);
        } catch (e) {
          log("刪除失敗：" + String(e?.message || e), "warn");
        }
        if (typeof refreshAiStatus === "function") await refreshAiStatus();
      });
  }
  if ($("btn-local-llm-cancel")) {
    $("btn-local-llm-cancel").onclick = async () => {
      try {
        await invoke("cancel_task");
      } catch (_) {
        /* ignore */
      }
      closeLocalLlmOverlay();
    };
  }
  listen("local-llm-progress", (ev) => {
    const p = (ev && ev.payload) || {};
    setProgress(p.percent, p.message);
  }).catch(() => {});
}

export async function ensureLocalLlmReady({ refreshAiStatus, appendLog, silent } = {}) {
  const status = await localLlmStatus();
  if (localLlmReady(status)) return true;
  const log = typeof appendLog === "function" ? appendLog : () => {};
  // 檔案已經裝好，只是服務還沒啟動（最常見：工具剛重開）——直接把服務叫起來就好，
  // 不必彈「同意並偵測 → 開始下載」那套為了「要不要下載幾 GB」設計的完整流程。
  // 使用者已經同意過一次、資料夾也還在，每次重開都要求再走一次等於逼退使用意願。
  if (status && status.installed) {
    try {
      await invoke("local_llm_ensure_ready_cmd", { installDir: currentDir() || null });
      if (typeof refreshAiStatus === "function") await refreshAiStatus();
      return true;
    } catch (e) {
      // 啟動指令回報失敗不代表真的沒起來：llama-server 首次載入模型可能超過
      // 指令本身的等待時間（實測會出現「overlay 開著、但底部同時寫著已就緒」
      // 這種前後矛盾的畫面）。所以失敗後改成輪詢實際狀態，起來了就當成功。
      if (!silent) log("本地模型啟動中，正在確認狀態…");
      if (await waitForReady(15000)) {
        if (typeof refreshAiStatus === "function") await refreshAiStatus();
        return true;
      }
      if (!silent) log("本地模型啟動失敗：" + String(e?.message || e), "warn");
    }
  }
  // 開 overlay 前的最後一道檢查：這段期間可能已經就緒，就不要再把視窗蓋上來。
  if (localLlmReady(await localLlmStatus())) {
    if (typeof refreshAiStatus === "function") await refreshAiStatus();
    return true;
  }
  if (!silent) {
    log(lastInstallError || String(status.message || "尚未安裝本地模型。請先同意並完成下載。"), "warn");
  }
  await openLocalLlmOverlay();
  if (typeof refreshAiStatus === "function") await refreshAiStatus();
  return false;
}

/** 輪詢本地模型狀態直到就緒或逾時。回 true 代表這段期間內服務起來了。 */
async function waitForReady(timeoutMs) {
  const deadline = Date.now() + Math.max(0, Number(timeoutMs) || 0);
  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 1000));
    try {
      if (localLlmReady(await localLlmStatus())) return true;
    } catch (_) {
      /* 查詢失敗就繼續等，逾時再放棄 */
    }
  }
  return false;
}
