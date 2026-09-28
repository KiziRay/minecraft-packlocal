/**
 * 設定視窗 → 主視窗的即時同步。
 *
 * 設定視窗改完值會自己寫進設定檔（patch_app_settings_cmd）與 localStorage，
 * 再送 `mcpl:settings-updated` 通知主視窗「立刻套用」。這裡只負責把事件分派到
 * 主視窗對應的處理函式，不碰 DOM，方便測試。
 */

import { isAllowedSettingPath } from "./settings-paths.js";

export const SETTINGS_UPDATED_EVENT = "mcpl:settings-updated";
/** 設定視窗請主視窗執行動作（重看引導與說明、顯示更新）。 */
export const SETTINGS_ACTION_EVENT = "mcpl:settings-action";

const truthy = (value) => value === true || value === "1" || value === "true";

/**
 * @param {{ path?: string, value?: unknown }} update
 * @param {Record<string, Function>} handlers
 * @returns {string} 處理的類別；不認得的路徑回 "ignored"
 */
export function routeSettingsUpdate(update, handlers = {}) {
  const path = String(update?.path || "");
  const value = update?.value;
  const call = (name, ...args) => {
    const fn = handlers[name];
    if (typeof fn === "function") fn(...args);
  };
  if (!path) return "ignored";
  // 白名單上的設定一律先同步快取，避免主視窗之後讀到舊值；其他路徑不碰快取
  if (isAllowedSettingPath(path)) call("store", path, value === undefined ? null : value);
  switch (path) {
    case "appearance.theme":
      call("theme", value === "light" ? "light" : "dark");
      return "theme";
    case "appearance.uiScale":
      call("uiScale", value);
      return "uiScale";
    case "appearance.uiAutoScale":
      call("uiAutoScale", truthy(value));
      return "uiAutoScale";
    case "appearance.sfxMuted":
      call("sfx", { muted: truthy(value) });
      return "sfx";
    case "appearance.sfxVolume":
      call("sfx", { volume: Number(value) });
      return "sfx";
    case "translate.outputStorageMode":
    case "translate.outputCustomRoot":
      call("outputStorage");
      return "outputStorage";
    case "privacy.rememberApiKey":
      call("rememberApiKey", truthy(value));
      return "rememberApiKey";
    default:
      return "ignored";
  }
}

/** 設定視窗可以請主視窗做的事。其他名稱一律忽略。（刪除全部備份 D-07 已改在設定視窗內做，B5a-2） */
export const SETTINGS_ACTIONS = ["replay-onboarding", "show-update"];

export function routeSettingsAction(payload, handlers = {}) {
  const action = String(payload?.action || "");
  if (!SETTINGS_ACTIONS.includes(action)) return "ignored";
  const fn = handlers[action];
  if (typeof fn === "function") fn(payload);
  return action;
}
