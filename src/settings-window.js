import { opDelete, opSet, getPath as readPath } from "./core/settings-patch.js";
import { SETTINGS_UPDATED_EVENT } from "./core/settings-sync.js";
import {
  OUTPUT_CUSTOM_ROOT_KEY,
  OUTPUT_STORAGE_MODE_KEY,
  SFX_MUTED_STORAGE_KEY,
  SFX_VOLUME_STORAGE_KEY,
  THEME_STORAGE_KEY,
} from "./core/storage.js";
import { clampScalePercent, computeAutoScalePercent, parseStoredAuto } from "./ui-scale-logic.js";
import { wireSettingsActions, refreshSettingsActions } from "./settings-window-actions.js";
import { CLOUD_TOPUP_CONSENT } from "./core/cloud-topup-consent.js";
import { confirmDialog } from "./ui/confirm.js";
import { ROW_COPY, SETTINGS_DIALOGS, describeOnlineAi } from "./settings/settings-copy.js";
import { refreshDataPane, wireDataPane } from "./settings/data-pane.js";
import { SETTINGS_NOTICE_EVENTS } from "./flow/main-state.js";
import { wireTabKeys } from "./ui/tab-keys.js";

const tauri = window.__TAURI__ || {};
const invoke = tauri.core?.invoke || (() => Promise.reject(new Error("設定服務尚未就緒。")));
const emit = tauri.event?.emit || (async () => {});
const listen = tauri.event?.listen || (async () => () => {});
const dialog = tauri.dialog || {};
const $ = (id) => document.getElementById(id);

const SCALE_KEY = "mcpl-webview-scale";
const AUTOSCALE_KEY = "mcpl-webview-autoscale";
const REMEMBER_KEY = "modpack-i18n-remember-api-key-v1";
const TOPUP_KEY = "modpack-i18n-local-cloud-topup-v1";
const PANES = ["general", "translate", "data", "help"];

let appSettings = { version: 1 };

function bootInfo() {
  const value = window.__MCPL_SETTINGS_BOOT;
  return value && typeof value === "object" ? value : { pane: "general", theme: "dark" };
}

function readLocal(key) {
  try {
    return localStorage.getItem(key);
  } catch (_) {
    return null;
  }
}

function writeLocal(key, value) {
  try {
    if (value === null || value === undefined) localStorage.removeItem(key);
    else localStorage.setItem(key, String(value));
  } catch (_) {
    /* localStorage 壞掉不影響設定檔那一邊 */
  }
}

/** 設定檔優先，其次 localStorage（主視窗與設定視窗共用同一份），最後才用預設值。 */
function readSetting(path, localKey, fallback) {
  const fromFile = readPath(appSettings, path);
  if (fromFile !== undefined && fromFile !== null) return String(fromFile);
  const fromLocal = localKey ? readLocal(localKey) : null;
  return fromLocal === null ? fallback : fromLocal;
}

export function setStatus(message, kind = "") {
  const node = $("status");
  if (!node) return;
  node.textContent = message;
  node.dataset.kind = kind;
}

export function settingsSnapshot() {
  return appSettings;
}

/** 依路徑合併寫入（後端 patch_app_settings_cmd）；絕不整份覆寫設定檔。 */
export async function patchSettings(ops) {
  const result = await invoke("patch_app_settings_cmd", { ops });
  if (result?.settings && typeof result.settings === "object") appSettings = result.settings;
  if (result?.path) $("settings-path").textContent = `設定檔：${result.path}`;
  return result;
}

/**
 * 儲存一個設定並通知主視窗立刻套用。
 * value 是 null＝明確清除（送刪除操作），不是「讀不到值」。
 */
export async function saveSetting(path, value, { localKey = "", message = "已儲存。" } = {}) {
  const cleared = value === null || value === undefined;
  // 設定檔寫成功才寫 localStorage：寫檔失敗時兩邊都維持原值，不會一邊新一邊舊
  await patchSettings([cleared ? opDelete(path) : opSet(path, String(value))]);
  if (localKey) writeLocal(localKey, cleared ? null : String(value));
  await emit(SETTINGS_UPDATED_EVENT, { path, value: cleared ? null : String(value) });
  setStatus(message, "ok");
}

function normalizeTheme(value) {
  return value === "light" ? "light" : "dark";
}

function applyTheme(value) {
  const theme = normalizeTheme(value);
  document.documentElement.dataset.theme = theme;
  document.querySelector('meta[name="theme-color"]')?.setAttribute("content", theme === "light" ? "#f4f6fa" : "#20242c");
  const select = $("theme-select");
  if (select) select.value = theme;
}

function screenWidth() {
  try {
    return window.screen?.availWidth || window.screen?.width || 1920;
  } catch (_) {
    return 1920;
  }
}

/** 縮放範圍與規則和主視窗共用 ui-scale-logic.js（80–170%，自動檔依螢幕寬）。 */
function applyScale(value, auto) {
  const scale = auto ? computeAutoScalePercent(screenWidth()) : clampScalePercent(value);
  document.documentElement.style.zoom = String(scale / 100);
  $("ui-scale").value = String(scale);
  $("ui-scale").disabled = !!auto;
  $("ui-autoscale").checked = !!auto;
  $("ui-scale-value").textContent = auto ? `${scale}%（自動）` : `${scale}%`;
  return scale;
}

async function saveScale(percent, auto, message) {
  const scale = applyScale(percent, auto);
  await patchSettings([opSet("appearance.uiScale", String(scale)), opSet("appearance.uiAutoScale", auto ? "1" : "0")]);
  writeLocal(SCALE_KEY, String(scale));
  writeLocal(AUTOSCALE_KEY, auto ? "1" : "0");
  if (auto) await emit(SETTINGS_UPDATED_EVENT, { path: "appearance.uiAutoScale", value: "1" });
  else await emit(SETTINGS_UPDATED_EVENT, { path: "appearance.uiScale", value: String(scale) });
  setStatus(message, "ok");
}

const OUTPUT_HINTS = {
  managed: "工具會替每個模組整合包建立獨立的資料夾。",
  beside: "翻譯結果會放在模組整合包旁邊的翻譯輸出資料夾。",
  custom: "翻譯結果會放在你指定的資料夾裡，每個模組整合包分開存放。",
};

function normalizeOutputMode(value) {
  return value === "beside" || value === "custom" ? value : "managed";
}

function renderOutputStorage() {
  const mode = normalizeOutputMode(readSetting("translate.outputStorageMode", OUTPUT_STORAGE_MODE_KEY, "managed"));
  const root = readSetting("translate.outputCustomRoot", OUTPUT_CUSTOM_ROOT_KEY, "");
  $("output-storage-mode").value = mode;
  $("output-storage-hint").textContent = OUTPUT_HINTS[mode];
  $("output-custom-root-row").hidden = mode !== "custom";
  $("output-custom-root").textContent = root || "尚未選擇";
  $("clear-output-custom-root").disabled = !root;
}

async function pickOutputRoot() {
  if (typeof dialog.open !== "function") throw new Error("無法開啟資料夾選擇視窗");
  const selected = await dialog.open({ directory: true, multiple: false, title: "選擇放翻譯結果的資料夾" });
  return typeof selected === "string" && selected.trim() ? selected.trim() : "";
}

/** 改了結果位置：就地說明舊結果還在、工具仍找得到（規格 §1.3）。 */
function showOutputNote(text) {
  const note = $("output-storage-note");
  if (!note) return;
  note.textContent = text;
  note.hidden = !text;
}

async function onOutputModeChange(mode) {
  const next = normalizeOutputMode(mode);
  if (next === "custom" && !readSetting("translate.outputCustomRoot", OUTPUT_CUSTOM_ROOT_KEY, "")) {
    const picked = await pickOutputRoot();
    if (!picked) {
      renderOutputStorage();
      setStatus("沒有選資料夾，存放位置維持原本的設定。", "warn");
      return;
    }
    await saveSetting("translate.outputCustomRoot", picked, { localKey: OUTPUT_CUSTOM_ROOT_KEY });
  }
  await saveSetting("translate.outputStorageMode", next, {
    localKey: OUTPUT_STORAGE_MODE_KEY,
    message: "已更新翻譯結果放哪裡。",
  });
  renderOutputStorage();
  showOutputNote(ROW_COPY.outputChanged);
}

function renderToggles() {
  $("remember-api-key").checked = readSetting("privacy.rememberApiKey", REMEMBER_KEY, "0") === "1";
  renderCloudTopUp();
  const muted = readSetting("appearance.sfxMuted", SFX_MUTED_STORAGE_KEY, "0") === "1";
  const volume = Number(readSetting("appearance.sfxVolume", SFX_VOLUME_STORAGE_KEY, "0.55"));
  $("sfx-muted").checked = muted;
  $("sfx-volume").value = String(Number.isFinite(volume) ? volume : 0.55);
  $("sfx-volume-value").textContent = `${Math.round(Number($("sfx-volume").value) * 100)}%`;
}

/** 還沒選過＝不勾，並講明「第一次需要時會問你」；只有明確同意過才顯示勾選。 */
function renderCloudTopUp() {
  const raw = String(readSetting("translate.localCloudTopUp", TOPUP_KEY, "") || "").trim().toLowerCase();
  const chosen = raw !== "";
  const enabled = chosen && !["0", "false", "off", "no"].includes(raw);
  $("local-cloud-topup").checked = enabled;
  $("local-cloud-topup-state").textContent = !chosen
    ? "尚未選擇（第一次需要時會問你）"
    : enabled
      ? "已開啟：翻不好的那幾句會送到線上 AI，用到你自己的額度。"
      : "已關閉：翻不好的句子保留原文，文字不會送出。";
}

/** 補完會用哪個線上 AI、能不能用（自訂金鑰優先，其次已登入的 ChatGPT；與後端同一順序）。 */
async function renderOnlineAi() {
  const line = $("local-cloud-topup-ai");
  if (!line) return;
  let hasKey = false;
  let provider = "";
  let gptUsable = false;
  try {
    const api = await invoke("get_api_settings");
    hasKey = !!(api?.hasKey ?? api?.has_key);
    provider = String(api?.provider || "");
  } catch (_) {
    /* 讀不到當成沒有 */
  }
  if (!hasKey) {
    try {
      gptUsable = !!(await invoke("gpt_auth_status_cmd"))?.usable;
    } catch (_) {
      gptUsable = false;
    }
  }
  line.textContent = describeOnlineAi({ hasKey, provider, gptUsable });
}

function switchPane(requested) {
  const aliases = { prefs: "general", legal: "help", guide: "help", ai: "translate" };
  const wanted = aliases[requested] || requested;
  const pane = PANES.includes(wanted) ? wanted : "general";
  for (const button of document.querySelectorAll("[data-pane]")) {
    const active = button.dataset.pane === pane;
    button.classList.toggle("active", active);
    button.setAttribute("aria-selected", active ? "true" : "false");
  }
  for (const name of PANES) $(`pane-${name}`).hidden = name !== pane;
  syncTabKeys();
}

/** 分頁方向鍵（規格 §6）；wireControls 接上後才有作用。 */
let syncTabKeys = () => {};

async function closeWindow() {
  try {
    const api = tauri.window || {};
    const current = (typeof api.getCurrentWindow === "function" && api.getCurrentWindow()) || api.appWindow;
    if (current?.close) {
      await current.close();
      return;
    }
  } catch (_) {
    /* fallback below */
  }
  window.close();
}

/** 失敗時把勾選還原，不讓畫面顯示一個沒真的生效的開關。 */
function onToggle(id, handler) {
  $(id).addEventListener("change", async (event) => {
    const box = event.target;
    try {
      await handler(!!box.checked);
    } catch (error) {
      box.checked = !box.checked;
      setStatus(`沒有儲存成功：${String(error)}`, "warn");
    }
  });
}

function wireControls() {
  document.querySelectorAll("[data-pane]").forEach((button) => {
    button.addEventListener("click", () => switchPane(button.dataset.pane));
  });
  syncTabKeys = wireTabKeys(document.querySelector('[role="tablist"]'));
  $("close-window").addEventListener("click", () => void closeWindow());
  $("theme-select").addEventListener("change", async (event) => {
    const theme = normalizeTheme(event.target.value);
    applyTheme(theme);
    try {
      await saveSetting("appearance.theme", theme, { localKey: THEME_STORAGE_KEY, message: "已套用外觀。" });
    } catch (error) {
      setStatus(`無法儲存外觀：${String(error)}`, "warn");
    }
  });
  $("ui-scale").addEventListener("input", (event) => {
    $("ui-scale-value").textContent = `${clampScalePercent(event.target.value)}%`;
  });
  $("ui-scale").addEventListener("change", (event) => {
    void saveScale(event.target.value, false, "已套用介面大小。");
  });
  onToggle("ui-autoscale", (on) => saveScale($("ui-scale").value, on, on ? "已改為依螢幕大小自動縮放。" : "已改為手動調整介面大小。"));
  $("ui-scale-reset").addEventListener("click", () => void saveScale(100, false, "介面大小已改回 100%。"));
  onToggle("minimize-on-close", async (on) => {
    await invoke("set_ui_prefs", { minimizeOnClose: on });
    setStatus("已更新按 X 時的行為。", "ok");
  });
  onToggle("sfx-muted", (on) =>
    saveSetting("appearance.sfxMuted", on ? "1" : "0", { localKey: SFX_MUTED_STORAGE_KEY, message: "已更新提示音。" })
  );
  $("sfx-volume").addEventListener("input", (event) => {
    $("sfx-volume-value").textContent = `${Math.round(Number(event.target.value) * 100)}%`;
  });
  $("sfx-volume").addEventListener("change", (event) => {
    void saveSetting("appearance.sfxVolume", String(event.target.value), {
      localKey: SFX_VOLUME_STORAGE_KEY,
      message: "已更新提示音音量。",
    }).catch((error) => setStatus(`無法儲存音量：${String(error)}`, "warn"));
  });
  $("output-storage-mode").addEventListener("change", (event) => {
    void onOutputModeChange(event.target.value).catch((error) => {
      renderOutputStorage();
      setStatus(`無法更新存放位置：${String(error)}`, "warn");
    });
  });
  $("pick-output-custom-root").addEventListener("click", async () => {
    try {
      const picked = await pickOutputRoot();
      if (!picked) return;
      await saveSetting("translate.outputCustomRoot", picked, { localKey: OUTPUT_CUSTOM_ROOT_KEY });
      await saveSetting("translate.outputStorageMode", "custom", {
        localKey: OUTPUT_STORAGE_MODE_KEY,
        message: "已改用你指定的資料夾。",
      });
      renderOutputStorage();
    } catch (error) {
      setStatus(`無法選擇資料夾：${String(error)}`, "warn");
    }
  });
  $("clear-output-custom-root").addEventListener("click", async () => {
    try {
      await saveSetting("translate.outputCustomRoot", null, { localKey: OUTPUT_CUSTOM_ROOT_KEY });
      if ($("output-storage-mode").value === "custom") {
        await saveSetting("translate.outputStorageMode", "managed", { localKey: OUTPUT_STORAGE_MODE_KEY });
      }
      renderOutputStorage();
      showOutputNote(ROW_COPY.outputCustomCleared);
      setStatus("已清除指定的資料夾，改回交給工具管理。", "ok");
    } catch (error) {
      setStatus(`無法清除：${String(error)}`, "warn");
    }
  });
  onToggle("remember-api-key", async (on) => {
    await invoke("set_remember_api_key_cmd", { remember: on });
    await saveSetting("privacy.rememberApiKey", on ? "1" : "0", {
      localKey: REMEMBER_KEY,
      message: on ? "API 金鑰會記在這台電腦（未加密）。" : "之後輸入的 API 金鑰不會存檔，關掉工具就要重新貼上。",
    });
  });
  $("clear-api-key").addEventListener("click", async () => {
    // D-12：工具自己的確認框（危險，預設焦點在取消）
    if (!(await confirmDialog({ ...SETTINGS_DIALOGS.clearKey }))) return;
    try {
      await invoke("clear_api_key_cmd");
      // 主畫面立即更新（金鑰欄、AI 狀態），不用等玩家切回去
      await emit(SETTINGS_NOTICE_EVENTS.apiKeyCleared, {});
      $("clear-api-key-result").textContent = ROW_COPY.keyCleared;
      setStatus("已清除金鑰。", "ok");
      await renderOnlineAi();
    } catch (error) {
      setStatus(`無法清除金鑰：${String(error)}`, "warn");
    }
  });
  $("local-cloud-topup").addEventListener("change", async (event) => {
    const box = event.target;
    const on = !!box.checked;
    // 打開前一定先同意（D-14，G0.6）：會送出文字、會用到線上 AI 額度；預設焦點在「只用本地」
    if (on && !(await confirmDialog({ ...SETTINGS_DIALOGS.cloudTopUp }))) {
      box.checked = false;
      setStatus("沒有改動：本地翻不好時仍不會送到線上 AI。", "ok");
      return;
    }
    try {
      await saveSetting("translate.localCloudTopUp", on ? "1" : "0", {
        localKey: TOPUP_KEY,
        message: on ? `已選擇「${CLOUD_TOPUP_CONSENT.confirmLabel}」。` : "已改成只用本地，文字不會送出。",
      });
    } catch (error) {
      box.checked = !on;
      setStatus(`沒有儲存成功：${String(error)}`, "warn");
    }
    renderCloudTopUp();
  });
  wireSettingsActions({ invoke, emit, $, setStatus });
  wireDataPane({ invoke, emit, listen, $, setStatus, patchSettings, settingsSnapshot, confirmDialog });
}

async function boot() {
  const boot = bootInfo();
  applyTheme(boot.theme);
  switchPane(boot.pane);
  wireControls();
  applyScale(readLocal(SCALE_KEY) || "100", parseStoredAuto(readLocal(AUTOSCALE_KEY)));
  try {
    const report = await invoke("read_app_settings_report_cmd");
    // 讀到 null（沒有設定檔或讀不出來）時只用預設值顯示，不寫回任何東西
    if (report?.settings && typeof report.settings === "object") appSettings = report.settings;
    applyTheme(readSetting("appearance.theme", THEME_STORAGE_KEY, boot.theme));
    renderToggles();
    renderOutputStorage();
    $("settings-path").textContent = report?.path ? `設定檔：${report.path}` : "";
    const prefs = await invoke("get_ui_prefs");
    $("minimize-on-close").checked = prefs?.minimizeOnClose !== false;
    $("app-version").textContent = String(prefs?.appVersion || "—");
    await refreshSettingsActions(prefs);
    await refreshDataPane();
    await renderOnlineAi();
    setStatus("設定已載入。", "ok");
  } catch (error) {
    setStatus(`部分設定無法讀取：${String(error)}`, "warn");
  }
  // 主視窗用快捷鍵或自動縮放改了大小，這裡跟著更新顯示（只顯示，不再寫回去）
  await listen(SETTINGS_UPDATED_EVENT, (event) => {
    const payload = event?.payload || {};
    if (payload.source === "main" && payload.path === "appearance.uiScale") {
      applyScale(payload.value, !!payload.autoScale);
    }
  });
  await listen("settings-pane", (event) => {
    const payload = event?.payload;
    const pane = typeof payload === "object" ? payload?.pane : payload;
    const theme = typeof payload === "object" ? payload?.theme : undefined;
    if (theme) applyTheme(theme);
    switchPane(pane);
  });
}

void boot();
