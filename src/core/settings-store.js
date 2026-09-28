/**
 * 工具設定的單一存取點：檔案優先，localStorage 保底。
 *
 * 為什麼要有這一層：改版前所有偏好都只存在 WebView2 的 localStorage——那是
 * 瀏覽器快取，清快取／換機器／重裝就全部消失，使用者也看不到、改不了。
 * 現在改成寫進資料根目錄的「工具設定.json」（跟著工具走）。
 *
 * 硬不變式（順序不可調換）：
 *  1. 啟動時先讀檔案；檔案缺的欄位才從 localStorage（含 1.0.6／1.0.7 舊鍵）補進去。
 *  2. 每次寫入**兩邊都寫**——檔案寫失敗不影響 localStorage，反之亦然。
 *     任何一邊還在，使用者的設定就還在。
 *  3. 檔案讀寫的任何失敗都必須靜默降級，絕不能讓工具開不起來。
 *  4. 寫檔一律走 patch_app_settings_cmd（只送改動的路徑），不整份覆寫——
 *     設定視窗與主視窗同時開著時，整份覆寫會把對方剛改的值蓋回去。
 *  5. 設定檔讀不出來（unreadable）時絕不寫入，以免拿預設值蓋掉使用者原本的設定。
 */

import { invoke } from "./dom.js";
import { applyOps, getPath, opDelete, opSet, planLocalStorageMigration } from "./settings-patch.js";

/** 記憶體快取：整份設定物件。啟動載入後就地更新，不必每次讀檔。 */
let cache = null;
let loaded = false;
let settingsPath = "";
/**
 * 設定檔健康狀態，供啟動後顯示提醒。
 * `missing` 是全新安裝的正常狀況，不打擾；`corrupt`／`unreadable` 一定要講，
 * 因為使用者原本的偏好在這一刻全部回到預設值，不講他只會覺得工具在亂改設定。
 */
let health = { status: "missing", backup: "", detail: "" };

/** localStorage 鍵 → 設定檔內的路徑（點號分隔）。遷移與雙寫都靠這張表。 */
const KEY_MAP = {
  "modpack-i18n-theme": "appearance.theme",
  "modpack-i18n-consent-hide-v1.0.9": "consent.hideVersion",
  "modpack-i18n-backup-before-apply": "translate.backupBeforeApply",
  "modpack-i18n-font-prefs": "appearance.fontPrefs",
  "modpack-i18n-sfx-volume-v1": "appearance.sfxVolume",
  "modpack-i18n-sfx-muted-v1": "appearance.sfxMuted",
  "modpack-i18n-output-storage-mode-v1": "translate.outputStorageMode",
  "modpack-i18n-output-custom-root-v1": "translate.outputCustomRoot",
  "modpack-i18n-cache-remind-v1": "translate.cacheRemind",
  "modpack-i18n-local-cloud-topup-v1": "translate.localCloudTopUp",
  "modpack-i18n-last-instance-path-v1": "translate.lastInstancePath",
  "modpack-i18n-local-llm-consent-v1": "localModel.consented",
  "modpack-i18n-local-llm-dir-v1": "localModel.installDir",
  "modpack-i18n-onboarding-seen-v1.0.9": "onboarding.seenVersion",
  "modpack-i18n-coverage-ack-hard": "translate.coverageAck",
  "modpack-i18n-usage-feedback-client-id-v1": "usage.clientId",
  "modpack-i18n-usage-feedback-last-submit-at-v1": "usage.lastSubmitAt",
  "modpack-i18n-usage-feedback-last-nudge-at-v1": "usage.lastNudgeAt",
  "modpack-i18n-remember-api-key-v1": "privacy.rememberApiKey",
  "mcpl-webview-scale": "appearance.uiScale",
  "mcpl-webview-autoscale": "appearance.uiAutoScale",
  "mcpl-disclosure-pick-folder": "ui.disclosure.pickFolder",
  "mcpl-banner-update-dismissed": "ui.banner.updateDismissedVersion",
};

/** 舊版鍵名 → 目前鍵名。目前的鍵沒有值時，才用舊鍵的值搬進設定檔。 */
const LEGACY_ALIASES = {
  "modpack-i18n-consent-hide-v1.0.6": "modpack-i18n-consent-hide-v1.0.9",
  "modpack-i18n-onboarding-seen-v1.0.7": "modpack-i18n-onboarding-seen-v1.0.9",
};

/** 已移除的設定：不再有任何程式讀它，啟動時順手清掉。 */
const REMOVED_LOCAL_KEYS = ["modpack-i18n-keep-local-model-v1"];

export { KEY_MAP, LEGACY_ALIASES };

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
    /* localStorage 壞掉不影響檔案那一邊 */
  }
}

/**
 * 把補丁送到後端合併寫入，成功後用後端回傳的整份設定更新快取
 * （順便拿到另一個視窗剛寫的值）。失敗靜默——localStorage 那邊已經寫過了。
 */
async function patchFile(ops) {
  if (!ops.length) return;
  try {
    const result = await invoke("patch_app_settings_cmd", { ops });
    if (result && result.settings && typeof result.settings === "object") cache = result.settings;
    if (result && result.path) settingsPath = String(result.path);
  } catch (_) {
    /* 檔案寫不進去（唯讀目錄、權限、設定檔讀不出來）時仍有 localStorage 保底 */
  }
}

/**
 * 啟動時呼叫一次。檔案有內容就用檔案，並把檔案缺的欄位從 localStorage 補進去；
 * 沒有檔案就把 localStorage 現況寫成檔案。全程不拋錯。
 */
export async function loadSettings() {
  if (loaded) return cache;
  loaded = true;
  try {
    settingsPath = String((await invoke("app_settings_path_cmd")) || "");
  } catch (_) {
    settingsPath = "";
  }
  let fromFile = null;
  try {
    const report = await invoke("read_app_settings_report_cmd");
    if (report && typeof report === "object") {
      fromFile = report.settings && typeof report.settings === "object" ? report.settings : null;
      health = {
        status: String(report.status || "missing"),
        backup: String(report.backup || ""),
        detail: String(report.detail || ""),
      };
      if (report.path) settingsPath = String(report.path);
    }
  } catch (_) {
    // 舊版後端沒有這個命令：退回只讀設定、不報健康狀態
    try {
      fromFile = await invoke("read_app_settings_cmd");
    } catch (_) {
      fromFile = null;
    }
  }
  for (const key of REMOVED_LOCAL_KEYS) writeLocal(key, null);
  const base = fromFile && typeof fromFile === "object" ? fromFile : { version: 1 };
  const ops = planLocalStorageMigration(base, KEY_MAP, LEGACY_ALIASES, readLocal);
  cache = base;
  applyOps(cache, ops);
  // 設定檔在但讀不出來：只在記憶體裡用 localStorage 的值，絕不寫檔蓋掉它
  if (health.status !== "unreadable") await patchFile(ops);
  // 檔案是主來源：把值同步回 localStorage，讓還沒改寫的舊程式碼也讀得到
  for (const [key, dotted] of Object.entries(KEY_MAP)) {
    const value = getPath(cache, dotted);
    if (value !== undefined && value !== null) writeLocal(key, value);
  }
  return cache;
}

/** 讀一個設定值；設定檔沒有就回退 localStorage，再沒有就回 fallback。 */
export function getSetting(localStorageKey, fallback = null) {
  const dotted = KEY_MAP[localStorageKey];
  if (cache && dotted) {
    const value = getPath(cache, dotted);
    if (value !== undefined && value !== null) return value;
  }
  const raw = readLocal(localStorageKey);
  return raw === null ? fallback : raw;
}

/**
 * 寫一個設定值：localStorage 與設定檔兩邊都寫。
 * value 是 null／undefined＝呼叫端明確要清除這個設定，送刪除操作。
 */
export function setSetting(localStorageKey, value) {
  writeLocal(localStorageKey, value);
  const dotted = KEY_MAP[localStorageKey];
  if (!dotted) return;
  if (!cache) cache = { version: 1 };
  const op = value === null || value === undefined ? opDelete(dotted) : opSet(dotted, String(value));
  applyOps(cache, [op]);
  void patchFile([op]);
}

/** 讀一個設定檔路徑（沒有對應 localStorage 鍵的新設定）；沒有值回 fallback。 */
export function getSettingPath(dotted, fallback = null) {
  const value = cache ? getPath(cache, dotted) : undefined;
  return value === undefined || value === null ? fallback : value;
}

/**
 * 直接寫一個設定檔路徑（沒有對應 localStorage 鍵的新設定，例如 translate.backupChoice）。
 * 跟 setSetting 不同：這裡要等寫入完成，失敗就拋錯——呼叫端接著要依這個值做事
 * （後端套用時會讀設定檔的備份選擇），不能靜默當作成功。
 */
export async function setSettingPath(dotted, value) {
  const op = value === null || value === undefined ? opDelete(dotted) : opSet(dotted, value);
  const result = await invoke("patch_app_settings_cmd", { ops: [op] });
  if (!cache) cache = { version: 1 };
  if (result && result.settings && typeof result.settings === "object") cache = result.settings;
  else applyOps(cache, [op]);
  return cache;
}

/**
 * 另一個視窗（設定視窗）已經把值寫進設定檔了，這裡只同步本視窗的快取與
 * localStorage，不再寫檔。回傳對應的 localStorage 鍵（沒有對應就回空字串）。
 */
export function applyExternalSetting(dotted, value) {
  if (!cache) cache = { version: 1 };
  const op = value === null || value === undefined ? opDelete(dotted) : opSet(dotted, String(value));
  applyOps(cache, [op]);
  const key = Object.keys(KEY_MAP).find((k) => KEY_MAP[k] === dotted) || "";
  if (key) writeLocal(key, value === null || value === undefined ? null : String(value));
  return key;
}

/** 設定檔實際位置，供設定頁顯示。 */
export function getSettingsPath() {
  return settingsPath;
}

/**
 * 設定檔壞掉時要對使用者說的話；一切正常回 `null`。
 *
 * 只有 `corrupt`／`unreadable` 才出聲。`missing` 是全新安裝的正常狀況，
 * 講了只會嚇到人。
 */
export function getSettingsHealthNotice() {
  if (health.status === "corrupt") {
    const where = health.backup
      ? `原本那份已改名保存在：${health.backup}`
      : "原本那份無法備份（可能被其他程式鎖住）";
    return {
      level: "warn",
      title: "你的設定讀不出來，這次先用預設值",
      body: `設定檔的內容壞掉了（${health.detail || "格式不正確"}）。${where}。翻譯功能不受影響，只是主題、音量這些偏好回到預設；重新設定一次就好。`,
      // 橫幅 N-02 用：原檔有沒有保留、要開哪個資料夾
      backupKept: !!health.backup,
      folder: health.backup || settingsPath,
    };
  }
  if (health.status === "unreadable") {
    return {
      level: "warn",
      title: "設定檔打不開，這次先用預設值",
      body: `${health.detail || "讀取失敗"}。多半是被防毒軟體或另一個視窗佔住了。翻譯功能不受影響。設定檔位置：${settingsPath}`,
      // 打不開時工具不寫入，原檔一定還在
      backupKept: true,
      folder: settingsPath,
    };
  }
  return null;
}
