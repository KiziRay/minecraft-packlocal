/**
 * 設定視窗的文案與純判斷（B5a-2，規格 §1.3、§3.5、§5.2）。
 *
 * 不碰 DOM：對話框規格交給 ui/confirm.js（與主視窗同一套焦點規範 G5a1.5／G5a1.6），
 * 停用原因與目前值由 data-pane.js 顯示。字數上限照規格 §5.2，由測試檢查。
 */

/** 規格 §5.2 的「字」：不算空白。 */
export function textLength(text) {
  return [...String(text || "").replace(/\s+/g, "")].length;
}

/** 各列就地說明（≤40 字）。 */
export const ROW_COPY = Object.freeze({
  aiSource: "AI 來源在主畫面選好遊戲資料夾後，於 AI 區更換。",
  rememberKey: "金鑰以一般檔案存在這台電腦（未加密）；不勾則關掉工具就要重貼。",
  keyCleared: "已清除金鑰，主畫面也已更新。",
  outputChanged: "已翻過的結果留在原處，工具仍找得到。",
  outputCustomCleared: "已改回交給工具管理；放在那個資料夾的舊結果，工具不再去找。",
  deleteResultsHelp: "套用後刪掉這次的翻譯結果；備份照「套用前要不要備份」，不受影響。",
  deleteResultsAck: "我了解之後模組整合包更新只能整包重翻",
  deleteBackupsHelp: "只刪目前遊戲資料夾的備份；翻譯結果與其他模組整合包不受影響。",
  migrateHelp: "按下就搬（原本的資料會留著當備份），搬完請重新開啟工具。",
  modelNone: "這台電腦沒有本地模型檔案。",
});

export const SETTINGS_DIALOGS = Object.freeze({
  /** D-12（清除金鑰）：不可逆，要重新貼金鑰。 */
  clearKey: Object.freeze({
    title: "清除已記住的金鑰？",
    body: "清除後，要用自訂 API 翻譯時得重新貼上金鑰。",
    danger: true,
    confirmLabel: "清除金鑰",
    cancelLabel: "取消",
  }),
  /** D-14（花錢）：打開「本地翻不好改用線上 AI 補完」要先同意（G0.6）。 */
  cloudTopUp: Object.freeze({
    title: "要改用線上 AI 補完嗎？",
    body: "本地模型翻不好的那幾句，會送到你設定的線上 AI，用到你自己的 AI 額度。可隨時在這裡關掉。",
    danger: false,
    initialFocus: "cancel",
    confirmLabel: "打開補完",
    cancelLabel: "只用本地",
  }),
  /** 設定改成「不備份」：覆蓋無備份（D-04 同款勾選）。 */
  noBackup: Object.freeze({
    title: "套用前都不備份嗎？",
    body: "之後套用會直接覆蓋遊戲裡原本的檔案，移除翻譯時放不回來。可隨時在這裡改回。",
    danger: true,
    ackLabel: "我了解之後無法還原被覆蓋的檔案",
    confirmLabel: "改成不備份",
    cancelLabel: "取消",
  }),
});

export function formatGb(bytes) {
  const n = Number(bytes);
  if (!Number.isFinite(n) || n <= 0) return "";
  return `${(n / 1024 ** 3).toFixed(1)} GB`;
}

/** D-12（刪除本地模型檔案）。 */
export function deleteModelDialog({ sizeBytes, dir } = {}) {
  const size = formatGb(sizeBytes);
  return {
    title: "刪除本地模型檔案？",
    body: `${size ? `可釋放 ${size}。` : ""}之後要用本地模型翻譯，得重新下載。已翻好的結果不受影響。`,
    affected: dir ? [String(dir)] : [],
    danger: true,
    confirmLabel: "刪除模型",
    cancelLabel: "取消",
  };
}

/** D-07（刪除這個遊戲資料夾的全部備份）：列實際備份位置，不寫「所有備份」。 */
export function deleteBackupsDialog({ location } = {}) {
  return {
    title: "刪除這個遊戲資料夾的全部備份？",
    body: "之後移除翻譯時，被翻譯覆蓋的原檔就放不回來了。翻譯結果與其他模組整合包的備份不受影響。",
    affected: location ? [String(location)] : [],
    danger: true,
    ackLabel: "我知道刪除後無法還原被覆蓋的檔案",
    confirmLabel: "刪除備份",
    cancelLabel: "取消",
  };
}

/** 套用前要不要備份（translate.backupChoice；全部模組整合包共用）。沒選過＝每次詢問（刪除設定）。 */
export const BACKUP_CHOICES = Object.freeze([
  Object.freeze({ value: "ask", label: "每次詢問" }),
  Object.freeze({ value: "always", label: "先備份" }),
  Object.freeze({ value: "never", label: "不備份" }),
]);

export function backupChoiceValue(settings) {
  const raw = settings && settings.translate ? settings.translate.backupChoice : "";
  return raw === "always" || raw === "never" ? raw : "ask";
}

export function backupCurrentLine(value) {
  const found = BACKUP_CHOICES.find((c) => c.value === value) || BACKUP_CHOICES[0];
  return `目前：${found.label}（全部模組整合包共用）`;
}

const BUSY_REASON = {
  deleteBackups: "正在翻譯，翻完才能刪除備份。",
  migrate: "正在翻譯，翻完才能搬移工具資料。",
  deleteModel: "正在翻譯，翻完才能刪除本地模型。",
};

/**
 * 資料與備份的三個動作何時停用（規格 §1.3、§7「翻譯中從設定搬移／刪備份」）。
 * mainState 由主視窗回報（mcpl:main-state）；還沒收到時當成忙碌中（失效安全：不讓玩家在翻譯中刪東西）。
 */
export function dataActionLocks(mainState, { modelInstalled = false } = {}) {
  const known = !!mainState && typeof mainState === "object";
  const busy = !known || !!mainState.busy;
  const waiting = "正在確認主畫面的狀態…";
  const lock = (key, extra) => {
    if (busy) return { locked: true, reason: known ? BUSY_REASON[key] : waiting };
    if (extra) return { locked: true, reason: extra };
    return { locked: false, reason: "" };
  };
  const hasPack = known && !!String(mainState.instancePath || "").trim();
  return {
    deleteBackups: lock("deleteBackups", hasPack ? "" : "請先在主畫面選好遊戲資料夾。"),
    migrate: lock("migrate", ""),
    deleteModel: lock("deleteModel", modelInstalled ? "" : ROW_COPY.modelNone),
  };
}

/** 翻完刪除翻譯結果：第一次打開要同列勾「我了解」才生效。save＝要寫入的值（null＝先不寫）。 */
export function deleteResultsStep({ wantOn, acked }) {
  if (!wantOn) return { save: "0", showAck: false };
  if (acked) return { save: "1", showAck: false };
  return { save: null, showAck: true };
}

const PROVIDER_NAMES = { deepseek: "DeepSeek", glm: "智譜 GLM", openai: "OpenAI", qwen: "通義千問", other: "OpenAI 相容服務" };

/** 補完會用哪個線上 AI（後端 resolve_cloud_fallback_config：自訂金鑰優先，其次已登入的 ChatGPT）。 */
export function describeOnlineAi(info) {
  const value = info && typeof info === "object" ? info : {};
  if (value.hasKey) {
    const name = PROVIDER_NAMES[String(value.provider || "")] || "自訂服務";
    return `補完會用：自訂 API（${name}），已填入金鑰。`;
  }
  if (value.gptUsable) return "補完會用：ChatGPT（已登入）。";
  return "目前還沒有可用的線上 AI，打開了也會先保留原文。";
}

export function migrateResultLine(result) {
  const files = Number(result && result.files);
  const count = Number.isFinite(files) && files > 0 ? `（${files} 個檔）` : "";
  return `已搬到工具旁邊${count}，請重新開啟工具。原本的資料留著當備份。`;
}
