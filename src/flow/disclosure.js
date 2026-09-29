/**
 * 說明漸進退場（規格 §4）：每則說明有獨立 key；第一次完整顯示，之後退場只留「？」。
 *
 * 熟手＝該說明對應的動作成功完成一次（或玩家按了「不再顯示」）。
 * 存在設定檔 `ui.disclosure.<key>`（白名單在 src/core/settings-paths.js，前後端共用，G0.1、G0.2）。
 * 這個模組不碰 DOM 也不直接讀檔：讀寫由呼叫端注入（主視窗用 getSetting／setSetting）。
 */

/** key → 設定檔路徑與 localStorage 鍵。新增一則說明要同步 settings-paths.js 與 settings-store.js 的 KEY_MAP。 */
export const DISCLOSURES = Object.freeze({
  pickFolder: Object.freeze({
    storageKey: "mcpl-disclosure-pick-folder",
    settingPath: "ui.disclosure.pickFolder",
    topic: "怎麼找到遊戲資料夾",
  }),
  // B5d：選資料夾就判定的附加說明（第一次完整，之後只留「？」）
  server: Object.freeze({
    storageKey: "mcpl-disclosure-server",
    settingPath: "ui.disclosure.server",
    topic: "為什麼不能翻伺服器資料夾",
  }),
  brokenRecord: Object.freeze({
    storageKey: "mcpl-disclosure-broken-record",
    settingPath: "ui.disclosure.brokenRecord",
    topic: "為什麼要先停下",
  }),
  copied: Object.freeze({
    storageKey: "mcpl-disclosure-copied",
    settingPath: "ui.disclosure.copied",
    topic: "分開記錄是什麼意思",
  }),
  packChanged: Object.freeze({
    storageKey: "mcpl-disclosure-pack-changed",
    settingPath: "ui.disclosure.packChanged",
    topic: "重新翻譯會不會很久",
  }),
  // B6a-1：S16 Minecraft 版本變了
  mcChanged: Object.freeze({
    storageKey: "mcpl-disclosure-mc-changed",
    settingPath: "ui.disclosure.mcChanged",
    topic: "版本變了為什麼要重新翻譯",
  }),
  // B5b：開始前確認（第一次多一句）、E1 AI 列「適合誰」、翻譯中附加行、本地模型第一次較久
  prestart: Object.freeze({
    storageKey: "mcpl-disclosure-prestart",
    settingPath: "ui.disclosure.prestart",
    topic: "開始前要確認什麼",
  }),
  aiChoice: Object.freeze({
    storageKey: "mcpl-disclosure-ai-choice",
    settingPath: "ui.disclosure.aiChoice",
    topic: "四種 AI 各適合誰",
  }),
  runTips: Object.freeze({
    storageKey: "mcpl-disclosure-run-tips",
    settingPath: "ui.disclosure.runTips",
    topic: "翻譯中可以做什麼",
  }),
  localStart: Object.freeze({
    storageKey: "mcpl-disclosure-local-start",
    settingPath: "ui.disclosure.localStart",
    topic: "本地模型啟動",
  }),
  // B5c：完成卡每類原因與「已幫你做的事」第一次展開（之後只條數）；人工補翻浮層怎麼請線上 AI 翻
  resultReasons: Object.freeze({
    storageKey: "mcpl-disclosure-result-reasons",
    settingPath: "ui.disclosure.resultReasons",
    topic: "還是英文的原因",
  }),
  manualFix: Object.freeze({
    storageKey: "mcpl-disclosure-manual-fix",
    settingPath: "ui.disclosure.manualFix",
    topic: "怎麼人工補翻",
  }),
});

const RETIRED = "retired";

export function createDisclosure({ read = () => null, write = () => {} } = {}) {
  const known = (key) => Object.prototype.hasOwnProperty.call(DISCLOSURES, key);
  /** 叫回一次（按「？」）：只影響這次畫面，不改退場紀錄。 */
  const recalled = new Set();

  function isFresh(key) {
    if (!known(key)) return false;
    try {
      return read(DISCLOSURES[key].storageKey) !== RETIRED;
    } catch (_) {
      // 讀不到＝當第一次（失效安全：多說一次比少說好）
      return true;
    }
  }

  function retire(key) {
    if (!known(key)) return;
    recalled.delete(key);
    try {
      write(DISCLOSURES[key].storageKey, RETIRED);
    } catch (_) {
      /* 寫不進去只是下次再說一次 */
    }
  }

  function recall(key) {
    if (known(key)) recalled.add(key);
  }

  function isShown(key) {
    return isFresh(key) || recalled.has(key);
  }

  function resetAll() {
    recalled.clear();
    for (const key of Object.keys(DISCLOSURES)) {
      try {
        write(DISCLOSURES[key].storageKey, "");
      } catch (_) {
        /* ignore */
      }
    }
  }

  return { isFresh, isShown, retire, recall, resetAll };
}
