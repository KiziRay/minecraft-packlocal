/**
 * 統一詞表（規格 §5.1）：同一功能只有一個名字。
 *
 * B5a-2 只套用到「開始前畫面」（整合包區、狀態卡 S01、AI 區、本包選項）與設定視窗；
 * 完成段、後端回傳文案留給 B5c，遊戲資料夾驗證字串留給 B5d。
 * FORBIDDEN 由 terms.test.mjs 掃這些畫面的可見文字，命中就是紅燈。
 */

export const TERMS = Object.freeze({
  pack: "模組整合包",
  gameFolder: "遊戲資料夾",
  traditionalChinese: "繁體中文",
  resume: "接續補完",
  updatedPart: "翻譯更新的部分",
  retranslate: "重新翻譯",
  deleteAndRetranslate: "刪除結果並重翻",
  apply: "套用到遊戲",
  applied: "已套用到遊戲",
  removeTranslation: "移除翻譯",
  removeFontPack: "移除字體包",
  openResults: "開啟結果資料夾",
  openFullLog: "開啟完整紀錄",
  share: "分享給朋友",
  issueReport: "問題回報",
  fontTool: "字體工具",
  chatgpt: "ChatGPT",
  apiKey: "API 金鑰",
  translationMemory: "翻譯記憶",
  sharedLibrary: "共享庫",
  resultLocation: "翻譯結果放哪裡",
  repairFile: "修復翻譯檔",
  modFile: "模組檔",
});

/**
 * 畫面上不該出現的字 → 應該用的字。pattern 已排除合法用法
 * （例如「模組整合包」裡的「整合包」、「ChatGPT」裡的「GPT」、「人工補翻」）。
 */
export const FORBIDDEN = Object.freeze([
  { pattern: /實例/, use: TERMS.gameFolder },
  { pattern: /模組包/, use: TERMS.pack },
  { pattern: /(?<!模組)整合包/, use: TERMS.pack },
  { pattern: /(?<!Chat)GPT/, use: TERMS.chatgpt },
  { pattern: /工作階段/, use: TERMS.repairFile },
  { pattern: /繁中/, use: TERMS.traditionalChinese },
  { pattern: /zh_tw/i, use: TERMS.traditionalChinese },
  { pattern: /補譯|補充漏翻|接續補翻|再補一些|只補缺漏/, use: TERMS.resume },
  { pattern: /(?<!人工)補翻/, use: TERMS.resume },
  { pattern: /覆蓋重翻/, use: TERMS.retranslate },
  { pattern: /裝進遊戲|直接套用|再次套用|重新套用/, use: TERMS.apply },
  { pattern: /開啟輸出|打開結果/, use: TERMS.openResults },
  { pattern: /打包分享|分享給其他玩家/, use: TERMS.share },
  { pattern: /送出回報|送出診斷回報|回報給管理員/, use: TERMS.issueReport },
  { pattern: /字體資源包工具|(?<!Unicode )字型/, use: "字體" },
  { pattern: /API Key/i, use: TERMS.apiKey },
  { pattern: /本機記憶/, use: TERMS.translationMemory },
  { pattern: /灌庫/, use: TERMS.sharedLibrary },
  { pattern: /預設存放位置/, use: TERMS.resultLocation },
  { pattern: /修復工作階段/, use: TERMS.repairFile },
  { pattern: /\bjar\b/i, use: TERMS.modFile },
  { pattern: /pack_format/, use: "（不寫）" },
  { pattern: /Luna 模型/, use: "（不寫）" },
  { pattern: /\bTTC\b/, use: ".ttc 字體檔" },
  { pattern: /Force Unicode|強制 Unicode(?! 字型)/i, use: "遊戲設定的「強制使用 Unicode 字型」" },
  { pattern: /CFPA/, use: "社群簡中翻譯" },
  { pattern: /AppData/i, use: "這台電腦的使用者資料夾" },
  { pattern: /港繁|待譯欄|健康檢查|並行|限流/, use: "（白話說明）" },
]);

/** 找出文字裡所有禁用詞；回傳 [{ word, use, line }]。 */
export function findForbidden(text) {
  const hits = [];
  const lines = String(text || "").split("\n");
  for (const line of lines) {
    for (const rule of FORBIDDEN) {
      const match = rule.pattern.exec(line);
      if (match) hits.push({ word: match[0], use: rule.use, line: line.trim() });
    }
  }
  return hits;
}
