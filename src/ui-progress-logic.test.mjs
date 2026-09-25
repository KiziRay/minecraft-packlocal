import test from "node:test";
import assert from "node:assert/strict";
import {
  clampStepIndexForward,
  formatStepMeta,
  hiddenLogCount,
  progressLogDedupeKey,
  shortenProgressMessage,
  visibleLogLines,
} from "./ui-progress-logic.js";

// 1.0.9 起改為只呈現最新 10 則；完整紀錄仍寫進結果資料夾，由「報告」查看。
// 這條原本釘的是「顯示全部行」，那個契約已經被取代。
test("visibleLogLines 只留最新 10 則，且不改動原陣列", () => {
  const lines = Array.from({ length: 20 }, (_, i) => `line-${i + 1}`);
  const visible = visibleLogLines(lines);
  assert.equal(visible.length, 10);
  assert.equal(visible[0], "line-11");
  assert.equal(visible[9], "line-20");
  assert.equal(lines.length, 20, "不可就地截斷來源陣列");
  assert.deepEqual(visibleLogLines(null), []);
  // 明確給上限時要照給的走（0／負數＝不限制）
  assert.equal(visibleLogLines(lines, 3).length, 3);
  assert.equal(visibleLogLines(lines, 0).length, 20);
});

test("formatStepMeta 為短計數", () => {
  assert.equal(formatStepMeta(1200, 5000, "條"), "1200／5000 條");
  assert.equal(formatStepMeta(null, 10, ""), "");
});

test("shortenProgressMessage 專業白話", () => {
  assert.equal(shortenProgressMessage("AI 限流：請稍候再送"), "AI 忙碌中，已自動放慢");
  assert.equal(shortenProgressMessage("AI 翻譯中…等待本輪回應（12s）"), "AI 等待回應");
  assert.equal(shortenProgressMessage("等待 Discord 重新登入"), "等待 Discord 驗證");
});

test("shortenProgressMessage 階段名稱用人話，但保留檔名與進度細節", () => {
  // 階段名稱要看得懂，但不能把「在翻哪個檔案／第幾個」這種有用的細節一起吃掉——
  // 使用者實測回報日誌連續三行都是「補上沒翻到的句子」，完全看不出在做什麼。
  const cases = [
    ["檢查資料夾…", "檢查資料夾與模組"],
    ["掃描模組文字：第 120/446 個 JAR", "掃描模組文字：第 120/446 個 JAR"],
    ["補充：語言表仍缺 433 條，只補缺…", "補上沒翻到的句子：語言表仍缺 433 條，只補缺…"],
    ["覆寫文字：候補 191 個檔，擷取字串…", "翻譯模組內的說明文字：候補 191 個檔，擷取字串…"],
    ["套用：備份後複製資源包／任務…", "套用到遊戲"],
    ["AI 有 3 批失敗，將重送", "有幾批沒成功，正在重送"],
  ];
  for (const [input, expected] of cases) {
    assert.equal(shortenProgressMessage(input), expected, input);
  }
  // 純狀態重述的尾巴不保留（沒有檔名也沒有數字）
  assert.equal(shortenProgressMessage("套用：備份後複製資源包／任務…"), "套用到遊戲");
  // 沒有對應階段的長訊息才截斷，而且要切在標點處
  const long = "某個沒有對應階段的很長訊息：" + "這裡是細節內容，".repeat(6);
  assert.ok(long.length > 48, "測試字串本身要夠長才驗得到截斷");
  const odd = shortenProgressMessage(long);
  assert.ok(odd.endsWith("…"), "沒對應的長訊息仍該截斷");
  assert.ok(odd.length <= 48);
});

test("日誌只呈現最新 10 則，並回報收合數", () => {
  const lines = Array.from({ length: 25 }, (_, i) => `line-${i}`);
  const visible = visibleLogLines(lines);
  assert.equal(visible.length, 10);
  assert.equal(visible[0], "line-15", "留下的必須是最新的 10 則");
  assert.equal(visible.at(-1), "line-24");
  assert.equal(hiddenLogCount(lines), 15);
});

test("progressLogDedupeKey 同一件事同一個十位數區間只留一行", () => {
  // 這是第十一輪要修的症狀本身：批次數／已得句數每次都不同，舊鍵（原始訊息）
  // 永遠判定成「不同行」——縮寫後其實長得一模一樣，只有百分比在跳。
  const a = progressLogDedupeKey(43, "AI 翻譯中… 5／20 批 · 已得 80/400 句 · 已進行 12 秒");
  const b = progressLogDedupeKey(44, "AI 翻譯中… 6／20 批 · 已得 96/400 句 · 已進行 18 秒");
  assert.equal(a, b, "同一個十位數區間、縮寫後文字相同，應視為同一行");
});

test("progressLogDedupeKey 跨十位數區間才重新記一行", () => {
  const a = progressLogDedupeKey(48, "AI 翻譯中… 9／20 批");
  const b = progressLogDedupeKey(51, "AI 翻譯中… 10／20 批");
  assert.notEqual(a, b, "跨進下一個十位數區間要能再寫一行，不能整個階段只有一行");
});

test("progressLogDedupeKey 翻譯中／等待回應交替視為同一行（實測會來回上百次）", () => {
  // 讀真實跑一次本地模型翻譯的執行日誌.txt 才發現的：這兩個狀態每次批次送出／
  // 收到回應就交替一次，實測在同一個十位數區間內來回了超過 60 次、跨了 17
  // 分鐘——因為每次交替文字都跟「上一行」不同，舊版（各自獨立不去重）完全沒
  // 擋下來，日誌被灌爆。這兩個狀態不是有意義的狀態轉換，是同一件事
  // （AI 忙著算這批）的正常交替，必須視為同一行。
  const a = progressLogDedupeKey(63, "AI 翻譯中… 5／20 批");
  const b = progressLogDedupeKey(63, "AI 翻譯中…等待本輪回應 · 已完成 5／20 批");
  assert.equal(a, b, "AI 翻譯中 ↔ AI 等待回應是同一階段的正常交替，不該各自佔一行");
  // 但真正有意義的狀態變化（例如整輪都沒拿到新譯文）仍要能寫出新的一行。
  const c = progressLogDedupeKey(63, "AI 這一輪沒有新譯文（連續 2 次）…");
  assert.notEqual(a, c, "真正的狀態變化不能被這個正規化吃掉");
});

test("clampStepIndexForward 擋下翻譯中途誤判回較早步驟（使用者回報的症狀）", () => {
  // 使用者截圖：翻譯明明已經到「翻譯」這一步（idx=2），文案卻在中途被
  // resolveStepKey 誤判回「檢查」（idx=0，例如文案又出現「準備中」字樣）。
  // 這裡必須夾回 lastStepIdx，不能讓時間被灌回已經定格的「檢查」。
  assert.equal(
    clampStepIndexForward(0, 2, 63, "prep", false),
    2,
    "翻譯中途（0<percent<100）誤判回較早步驟要夾回 lastStepIdx"
  );
});

test("clampStepIndexForward 正常前進不受影響", () => {
  assert.equal(clampStepIndexForward(3, 2, 82, "supplement", false), 3, "往前走不夾");
  assert.equal(clampStepIndexForward(2, 2, 63, "translate", false), 2, "停在原地不夾");
});

test("clampStepIndexForward 三個例外情境都不夾", () => {
  // done：允許直接跳到套用完成，不管 lastStepIdx 在哪。
  assert.equal(clampStepIndexForward(0, 2, 100, "done", false), 0);
  // failed：失敗是一次性事件，不套用這個 clamp。
  assert.equal(clampStepIndexForward(0, 2, 63, "prep", true), 0);
  // 新一輪剛開始（呼叫端已經把 lastStepIdx 重置成 -1）：合理地再跑一次不夾。
  assert.equal(clampStepIndexForward(0, -1, 5, "prep", false), 0);
});

test("不足上限時不收合", () => {
  const lines = ["a", "b", "c"];
  assert.deepEqual(visibleLogLines(lines), lines);
  assert.equal(hiddenLogCount(lines), 0);
  assert.deepEqual(visibleLogLines([]), []);
  assert.equal(hiddenLogCount([]), 0);
});
