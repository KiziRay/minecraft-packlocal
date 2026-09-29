/**
 * 工具設定檔（工具設定.json）允許從前端寫入的路徑——**唯一一份清單**。
 *
 * 後端 app_settings.rs 以 include_str! 讀這個檔案的 SETTING_PATHS 區塊，
 * 前端 settings-patch.js／settings-sync.js 直接 import，兩邊不會各寫一份而慢慢分歧。
 * 格式固定：一行一個雙引號字串，後端靠這個格式解析，請勿改寫成其他形式。
 *
 * 不在清單上的路徑（version、migration.*、__proto__ 等）一律拒絕寫入。
 */
export const SETTING_PATHS = [
  "appearance.theme",
  "appearance.fontPrefs",
  "appearance.sfxVolume",
  "appearance.sfxMuted",
  "appearance.uiScale",
  "appearance.uiAutoScale",
  "consent.hideVersion",
  // 已停用（B5b：#backup-before-apply 由開始前確認的備份列取代）：留在清單讓舊設定檔照常讀寫，值一律忽略
  "translate.backupBeforeApply",
  "translate.backupChoice",
  "translate.outputStorageMode",
  "translate.outputCustomRoot",
  // 已停用（B5a-2 刪除「發現已翻過時提醒我」）：留在清單讓舊設定檔照常讀寫，值一律忽略
  "translate.cacheRemind",
  "translate.localCloudTopUp",
  "translate.lastInstancePath",
  "translate.coverageAck",
  "translate.deleteResultsAfterApply",
  "translate.deleteResultsAck",
  // B5b：記住「不使用 AI」（"0"＝不使用 AI；其他來源存在後端 ai_mode）
  "translate.useAi",
  "localModel.consented",
  "localModel.installDir",
  "onboarding.seenVersion",
  "privacy.rememberApiKey",
  "usage.clientId",
  "usage.lastSubmitAt",
  "usage.lastNudgeAt",
  "developer.testMode",
  "ui.disclosure.pickFolder",
  "ui.disclosure.server",
  "ui.disclosure.brokenRecord",
  "ui.disclosure.copied",
  "ui.disclosure.packChanged",
  "ui.disclosure.prestart",
  "ui.disclosure.aiChoice",
  "ui.disclosure.runTips",
  "ui.disclosure.localStart",
  "ui.disclosure.resultReasons",
  "ui.disclosure.manualFix",
  "ui.banner.updateDismissedVersion",
];

const FORBIDDEN_SEGMENTS = new Set(["__proto__", "constructor", "prototype"]);

/** 路徑是否可以寫入：必須在清單上，且任何一段都不能是原型鏈相關的名稱。 */
export function isAllowedSettingPath(dotted) {
  const text = String(dotted || "");
  if (!text || text.split(".").some((part) => !part.trim() || FORBIDDEN_SEGMENTS.has(part))) return false;
  return SETTING_PATHS.includes(text);
}
