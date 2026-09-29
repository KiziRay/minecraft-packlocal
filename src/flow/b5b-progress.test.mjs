/**
 * B5b 翻譯中進度（規格 §3.2）：一句現況、已完成幾條、大約還要多久；看不到批次、同時處理數、HTTP。
 */
import test from "node:test";
import assert from "node:assert/strict";

import { FORBIDDEN, findForbidden } from "../copy/terms.js";
import {
  N07_TEXT,
  REASSURE,
  STOPPING_SENTENCE,
  countLine,
  etaText,
  needsMemoryBanner,
  plainBadge,
  progressNotes,
  progressView,
  runSentence,
} from "./run-progress.js";

test("一般：「正在翻「<包名>」：第 N／M 段」＋已完成 X／Y 條＋約還要 T", () => {
  const v = progressView({
    packName: "ATM10",
    stepIndex: 2,
    stepTotal: 5,
    percent: 50,
    elapsedMs: 10 * 60000,
    payload: { state: "running", done: 1200, total: 3000, unit: "條" },
    message: "AI 翻譯：第 12/40 批（並行 4）",
  });
  assert.equal(v.sentence, "正在翻「ATM10」：第 3／5 段");
  assert.equal(v.count, "已完成 1,200／3,000 條");
  assert.equal(v.eta, "約還要 10 分鐘");
  assert.equal(v.badge, "");
  assert.equal(v.reassure, "");
  const all = [v.sentence, v.count, v.eta].join("\n");
  assert.doesNotMatch(all, /批|並行|HTTP|拆半|上下文|層數/);
});

test("剩餘時間：未滿 5% 寫「正在估算…」；超過一小時寫小時與分", () => {
  assert.equal(etaText({ percent: 3, elapsedMs: 60000 }), "正在估算…");
  assert.equal(etaText({ percent: 20, elapsedMs: 30 * 60000 }), "約還要 2 小時");
  assert.equal(etaText({ percent: 25, elapsedMs: 25 * 60000 }), "約還要 1 小時 15 分");
  assert.equal(etaText({ percent: 99, elapsedMs: 60000 }), "約還要不到 1 分鐘");
});

test("條數只在單位是條數時顯示（模組、檔案不算）", () => {
  assert.equal(countLine({ done: 3, total: 10, unit: "個模組" }), "");
  assert.equal(countLine({ done: 3, total: 10, unit: "" }), "", "審查 5a：單位不明不當條數");
  assert.equal(countLine({}), "");
});

test("連線中斷：「連線中斷，自動重試中（最多再等約 X 分鐘）」＋「已翻好的不會遺失」；不顯示第幾次", () => {
  const v = progressView({
    packName: "A",
    payload: { state: "retrying" },
    message: "AI 連線中斷或忙碌，等待恢復並自動重試（已等 60 秒，最多等 300 秒；已翻好的不會遺失）…",
  });
  assert.equal(v.sentence, "連線中斷，自動重試中（最多再等約 4 分鐘）");
  assert.equal(v.reassure, REASSURE);
  const short = progressView({ payload: { state: "retrying" }, message: "AI 連線中斷，自動重試中（第 2 次，等 8 秒）…" });
  assert.equal(short.sentence, "連線中斷，自動重試中");
  assert.doesNotMatch(short.sentence, /第 \d+ 次/);
});

test("放慢與較慢只用白話徽章，並附保證語；不寫「限流」「並行」", () => {
  assert.equal(plainBadge("throttled"), "服務商要求放慢，已自動放慢");
  const slow = progressView({ message: "本地模型這批 12 條等了 240 秒還沒回應，改拆小重送；之後每批的等待上限拉長為原本的 1.5 倍" });
  assert.equal(slow.badge, "較慢，已自動放寬等待時間");
  assert.equal(slow.reassure, REASSURE);
  assert.deepEqual(findForbidden([plainBadge("throttled"), slow.badge].join("\n")), []);
});

test("本地模型字句的白話：啟動（第一次較久只在第一次）、顯示卡記憶體、沒回應重送、線上補完", () => {
  assert.equal(runSentence({ message: "本地模型啟動設定：上下文 8192 個字詞…", localStartFirst: true }), "正在啟動本地模型（第一次較久）");
  assert.equal(runSentence({ message: "本地模型啟動中，正在確認狀態…" }), "正在啟動本地模型");
  assert.equal(runSentence({ message: "顯示記憶體不足以載入整個模型，改用較保守的設定重試" }), "顯示卡記憶體不夠，改用較穩的方式啟動");
  assert.equal(runSentence({ message: "AI 沒回應的 17 句：同一輪縮小批次重送（第 1 次）" }), "再翻一次剛才沒回應的 17 句");
  assert.equal(progressView({ message: "AI 沒回應的 17 句：同一輪縮小批次重送（第 1 次）" }).reassure, REASSURE);
  assert.equal(runSentence({ message: "本地模型翻不好的 5 句，改用線上 AI 補完（讓品質一致）" }), "用線上 AI 補完 5 句（照你的設定）");
  assert.deepEqual(progressNotes("…顯示卡層數 0（純 CPU，會比較慢）"), ["沒有可用的顯示卡，會比較慢"]);
  assert.deepEqual(progressNotes("記憶體估計只放得下約 4096 個字詞，已把同時處理數從 4 降為 1"), ["記憶體較少，改成一次翻一批"]);
});

test("N-07：本地模型記憶體不太夠才出橫幅", () => {
  assert.equal(needsMemoryBanner("注意：記憶體仍不夠放下一批需要的量"), true);
  assert.equal(needsMemoryBanner("本地模型啟動設定"), false);
  assert.ok(Array.from(N07_TEXT).length <= 40);
});

test("停止中（S10）：「正在停止，寫出已翻好的部分…」，不顯示剩餘時間與條數", () => {
  const v = progressView({ stopping: true, percent: 60, elapsedMs: 600000, payload: { done: 1, total: 2 } });
  assert.equal(v.sentence, STOPPING_SENTENCE);
  assert.equal(v.eta, "");
  assert.equal(v.count, "");
});

test("翻譯中文案都在 40 字內且不含禁用詞", () => {
  const samples = [
    runSentence({ packName: "一個非常非常長的模組整合包名稱", stepIndex: 0 }),
    STOPPING_SENTENCE,
    N07_TEXT,
    "連線中斷，自動重試中（最多再等約 5 分鐘）",
    ...progressNotes("純 CPU 同時處理數從 4 降為 1"),
  ];
  for (const s of samples) assert.ok(Array.from(s).length <= 40, s);
  assert.ok(FORBIDDEN.length > 0);
  assert.deepEqual(findForbidden(samples.join("\n")), []);
});
