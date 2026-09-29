/**
 * B5c 完成模式（規格 §2.2 S11／S14／S17、§3.3）：結論依實際結果；中途停下絕不寫「完成」；
 * 次要 ≤3；已幫你做的事；還是英文的部分；依包保存與套用前比對。
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  RESULT_STATE,
  RESULT_ACTION,
  summarizeRun,
  summarizeProbe,
  resultCardState,
  englishLines,
  doneLines,
  reasonFor,
  fontBanner,
  runPlanNote,
} from "./result-card.js";
import { applyTargetCheck, createPackResults } from "./pack-results.js";
import { STATE, ZERO_PRIMARY_ALLOWED, computePackState } from "./pack-state.js";

const len = (t) => Array.from(String(t)).length;

const stopped = {
  coveragePercent: 62,
  pendingCount: 1340,
  applyStatus: "applied",
  interruption: { aiStopped: "已依你的要求停止翻譯", kind: "user_stop", stoppedByUser: true, noAnswer: 0, qualityDeferred: 4, cause: "user_stop" },
  displaySafety: { rejectedCount: 2, rejected: [], unverifiedCount: 0, needsOriginal: [], fontMayNotSupportChinese: [] },
  applyResult: { status: "applied", zipCopied: "MCPL.zip", langSet: true, originalLang: "en_us", backupCreated: true, backupDir: "C:/bak", jarsCopied: 3, quarantinedFiles: [], outdatedMods: [], outdatedTexts: [], staleOutputs: [], retiredFiles: [], unconfirmedFiles: [], retireSkipped: [], sourceRemovedTexts: [], unverifiableTexts: [], skippedChanged: [], unknownFiles: [] },
  playerSummary: "可以直接開遊戲了，主要遊戲文字都已是繁體中文。",
};

const full = {
  coveragePercent: 97,
  pendingCount: 0,
  applyStatus: "applied",
  interruption: { aiStopped: null, kind: null, stoppedByUser: false, noAnswer: 0, qualityDeferred: 0, cause: null },
  displaySafety: { rejectedCount: 0, rejected: [], unverifiedCount: 0, needsOriginal: [], fontMayNotSupportChinese: [] },
  applyResult: { ...stopped.applyResult },
};

test("中途停下的完成卡不含「完成」字樣、寫比例與英文條數，並顯示原因", () => {
  const s = summarizeRun(stopped, { origin: "run" });
  const card = resultCardState(s, { packName: "ATM10" });
  assert.equal(card.id, RESULT_STATE.partial);
  assert.equal(card.sentence, "「ATM10」翻了約 62%，還有 1340 條是英文");
  const all = [card.sentence, ...card.detailLines].join("\n");
  assert.ok(!all.includes("完成"), all);
  assert.ok(!all.includes("都已是繁體中文"), all);
  assert.ok(card.detailLines.includes("你按了停止，翻好的部分已套用"), all);
  assert.equal(card.primary.action, RESULT_ACTION.supplement);
  assert.equal(card.primary.label, "接續補完");
});

test("AI 停下的原因依後端分類碼（額度／金鑰／本地模型卡住／關閉），不看中文句", () => {
  const cases = [
    ["quota", "custom", "服務商帳戶沒餘額或到上限；儲值後按接續補完"],
    ["quota", "gpt", "ChatGPT 暫時不接受翻譯；額度重設後按接續補完"],
    ["auth", "custom", "金鑰被服務商拒絕；重新填金鑰後接續補完"],
    ["local_gone", "local", "本地模型意外關閉，通常是記憶體不夠"],
    ["no_output", "custom", "AI 一直給不出可用的翻譯，先停下"],
    ["network", "custom", "網路斷太久，先停下"],
  ];
  for (const [cause, aiMode, text] of cases) {
    const r = reasonFor({ cause, coverage: 38 }, { aiMode });
    assert.equal(r.text, text, cause);
    assert.ok(len(r.text) <= 40);
  }
  assert.equal(reasonFor({ cause: "local_stuck", coverage: 38 }, { aiMode: "local" }).text, "本地模型在這台電腦跑不動，停在 38%");
  assert.equal(reasonFor({ cause: "local_stuck", coverage: 38 }, { onlineConfigured: true }).primary.label, "改用線上 AI 接續");
  assert.equal(reasonFor({ cause: "auth" }, {}).primary.label, "重新填金鑰");
  // 額度用完只在原因句說（沒有另外的橫幅或視窗）
  const card = resultCardState(summarizeRun({ ...stopped, interruption: { ...stopped.interruption, cause: "quota", stoppedByUser: false } }), { aiMode: "custom" });
  assert.ok(card.detailLines.includes("服務商帳戶沒餘額或到上限；儲值後按接續補完"));
});

test("完整跑完且已套用：S17 無主要按鈕、中文約 N%、已幫你做的事第一次展開", () => {
  const s = summarizeRun(full, { localModelClosed: true });
  const card = resultCardState(s, { packName: "ATM10", firstTime: true });
  assert.equal(card.id, RESULT_STATE.done);
  assert.equal(card.primary, null);
  assert.ok(ZERO_PRIMARY_ALLOWED.includes("S17"));
  assert.equal(card.sentence, "「ATM10」已套用到遊戲（中文約 97%），開遊戲就好");
  const done = doneLines(s);
  assert.deepEqual(done.slice(0, 3), [
    "啟用翻譯資源包並排最上面",
    "語言設為繁體中文（原本是英文，移除翻譯會改回）",
    "備份了會被換掉的原檔（路徑：C:/bak）",
  ]);
  assert.ok(done.includes("覆蓋了 3 個模組檔") && done.includes("已關閉本地模型"));
  assert.ok(card.detailLines.includes("已幫你做的事："));
  assert.equal(card.secondary[0].label, "重新翻譯");
});

test("次要按鈕最多 3 顆，其餘進「更多」；移除翻譯不在完成卡", () => {
  const s = summarizeRun(stopped);
  const card = resultCardState(s, { canShare: true, canMergeTerms: true, aiMode: "custom" });
  assert.ok(card.secondary.length <= 3);
  const labels = [...card.secondary, ...card.more].map((b) => b.label);
  for (const want of ["人工補翻", "分享給朋友", "開啟結果資料夾", "併入用詞建議"]) assert.ok(labels.includes(want), want);
  assert.ok(!labels.includes("移除翻譯"));
});

test("還是英文的部分：0 條不顯示；第一次附原因、之後只條數；帶出 displaySafety 與 ApplyResult 欄位", () => {
  const s = summarizeRun({
    ...stopped,
    displaySafety: { rejectedCount: 2, unverifiedCount: 5, needsOriginal: ["kubejs/a.js"], fontMayNotSupportChinese: [] },
    applyResult: { ...stopped.applyResult, outdatedMods: ["mods/a.jar"], outdatedTexts: ["config/q.snbt", "x"], staleOutputs: ["old.json"] },
  });
  const first = englishLines(s, { firstTime: true });
  assert.ok(first.includes("還沒翻 1340 條：接續補完會再翻"));
  assert.ok(first.includes("安全檢查退回 2 條：避免遊戲出現方框或亂碼，明細在紀錄"));
  assert.ok(first.includes("缺原文未完整檢查 5 條：之後重新翻譯一次可補完整檢查"));
  assert.ok(first.includes("需要原檔 1 個檔：先移除翻譯或重裝模組整合包再接續補完"));
  assert.ok(first.includes("模組已更新：1 個模組、2 處文字要重翻"));
  assert.ok(first.includes("舊版結果 1 個檔沒放進遊戲：要重新翻譯才能套用"));
  const later = englishLines(s, { firstTime: false });
  assert.ok(later.includes("還沒翻 1340 條") && !later.some((l) => l.includes("接續補完會再翻")));
  assert.deepEqual(englishLines(summarizeRun(full)), []);
});

test("S11：已翻完未套用依原因一句＋套用到遊戲；部分完成變體；失敗變體；複製來的主要按鈕是當成新的", () => {
  const running = summarizeRun({ ...full, applyStatus: "gameRunning", applyMessage: "翻譯已完成，但 Minecraft 正在使用…" });
  let card = resultCardState(running, { packName: "A" });
  assert.equal(card.id, "S11");
  assert.equal(card.sentence, "已翻完，還沒套用：Minecraft 還開著");
  assert.equal(card.primary.label, "套用到遊戲");
  card = resultCardState(summarizeRun({ ...stopped, applyStatus: "gameRunning" }), {});
  assert.equal(card.sentence, "翻了約 62%（1340 條英文），還沒套用：Minecraft 還開著");
  card = resultCardState(running, { applyFailure: "網路磁碟連不上" });
  assert.equal(card.sentence, "套用沒有完成：網路磁碟連不上");
  assert.equal(card.primary.label, "再試一次");
  assert.ok(card.secondary.some((b) => b.label === "問題回報"));
  card = resultCardState(summarizeRun({ ...full, applyStatus: "forkNeeded" }), {});
  assert.equal(card.primary.action, RESULT_ACTION.fork);
  assert.equal(card.primary.label, "當成新的模組整合包");
  assert.ok(!card.sentence.includes("失敗"));
  card = resultCardState(summarizeRun({ ...full, applyStatus: "noOptionsTxt" }), {});
  assert.equal(card.extraLine, "先用啟動器開一次遊戲再回來按");
  card = resultCardState(running, { gameStillRunning: true });
  assert.equal(card.disabledReason, "Minecraft 還開著，請先關閉遊戲");
});

test("本機已有結果（探測）也走 S14／S17，不再有「本機已有翻譯」卡", () => {
  const partial = resultCardState(summarizeProbe({ status: "partial", pendingCount: 20, completionPercent: 88, shareable: true }), { packName: "B" });
  assert.equal(partial.id, "S14");
  assert.equal(partial.sentence, "「B」翻了約 88%，還有 20 條是英文");
  const done = resultCardState(summarizeProbe({ status: "ready", pendingCount: 0, completionPercent: 100, shareable: true }), { packName: "B", firstTime: false });
  assert.equal(done.id, "S17");
  assert.equal(done.sentence, "已翻完並套用（中文約 100%）");
  assert.ok([...done.secondary, ...done.more].some((b) => b.label === "分享給朋友"), "本機舊結果也能分享");
});

test("N-05 字體橫幅只在有換字體的資源包時出現；runPlan 覆寫寫成一行", () => {
  assert.equal(fontBanner(summarizeRun(full)), null);
  const b = fontBanner(summarizeRun({ ...full, displaySafety: { fontMayNotSupportChinese: ["Faithful 32x"] } }));
  assert.equal(b.id, "N-05");
  assert.equal(b.text, "已啟用的「Faithful 32x」會換掉字體，中文可能變方框");
  assert.equal(b.actionLabel, "開啟字體工具");
  const note = runPlanNote(summarizeRun({ ...full, runPlanHasOverrides: true, runPlan: { decisions: [{ overridden: true, reason: "這台電腦記憶體較少，改成一次翻一批" }] } }));
  assert.equal(note, "上次的特別設定：這台電腦記憶體較少，改成一次翻一批");
});

test("依包保存：換到 B 包再回 A，A 的待套用仍在；在 B 套用會被擋，不會套到 A", () => {
  const store = createPackResults();
  store.record("C:/games/A", summarizeRun({ ...full, applyStatus: "gameRunning" }), { instancePath: "C:/games/A", outputDir: "C:/out/A" });
  assert.ok(store.pendingContext("C:/games/A"));
  assert.equal(store.pendingContext("C:/games/B"), null, "B 包沒有待套用");
  const ctx = store.pendingContext("c:\\games\\A\\");
  assert.equal(ctx.outputDir, "C:/out/A", "同一包不同寫法也認得");
  const check = applyTargetCheck(ctx, "C:/games/B");
  assert.equal(check.ok, false);
  assert.equal(check.reason, "這份翻譯結果屬於「A」，不是「B」");
  assert.ok(applyTargetCheck(ctx, "C:/games/A").ok);
  store.collapse("C:/games/A");
  assert.equal(store.summary("C:/games/A").fresh, false);
  store.markApplied("C:/games/A", null);
  assert.equal(store.pendingContext("C:/games/A"), null);
});

test("pack-state：S11-card 暫行例外已刪；結果卡依目前資料夾計算", () => {
  assert.ok(!Object.values(STATE).includes("S11-card"));
  assert.ok(!ZERO_PRIMARY_ALLOWED.includes("S11-card"));
  const base = { consentAccepted: true, instancePath: "C:/games/A", validation: { ok: true } };
  const pending = summarizeRun({ ...full, applyStatus: "gameRunning" });
  let st = computePackState({ ...base, result: pending, resultCtx: { packName: "A" } });
  assert.equal(st.id, "S11");
  assert.equal(st.primary.label, "套用到遊戲");
  st = computePackState({ ...base, result: summarizeRun(stopped), resultCtx: {} });
  assert.equal(st.id, "S14");
  st = computePackState({ ...base, result: summarizeRun(full), resultCtx: {} });
  assert.equal(st.id, "S17");
  assert.equal(st.primary, null);
  // 沒有這次的結果、但本機有探測到且已套用過 → S17／S14（取代本機已有翻譯卡）
  st = computePackState({ ...base, hasResult: true, hasTranslationRecord: true, probeSummary: summarizeProbe({ status: "ready", completionPercent: 95 }), translationComplete: true });
  assert.equal(st.id, "S17");
  // 模組整合包有變動（S15）優先於舊結果
  st = computePackState({ ...base, packChanged: true, result: summarizeRun(full), resultCtx: {} });
  assert.equal(st.id, "S15");
  // S17 按重新翻譯進入 §3.1（reTranslate）
  st = computePackState({ ...base, result: summarizeRun(full), resultCtx: {} });
  assert.equal(st.reTranslate, true);
});

test("§3.1「更多」：已有結果時有「另存一份新的結果」與上次特別設定一行（取代三選一的另存）", () => {
  const base = { consentAccepted: true, instancePath: "C:/games/A", validation: { ok: true } };
  const st = computePackState({
    ...base,
    hasResult: true,
    translationComplete: true,
    prestart: { open: true, rows: [] },
    lastRunPlanNote: "上次的特別設定：改成一次翻一批",
  });
  assert.equal(st.id, "S13");
  assert.ok(st.more.some((m) => m.action === "run-new-copy" && m.label === "另存一份新的結果"));
  assert.ok(st.more.some((m) => m.note === "上次的特別設定：改成一次翻一批"));
});

test("R-1／§5.2：S11／S14／S17 各變體主要按鈕 0 或 1（0 只在 S17）、現況句與原因句 ≤40 字（包名不計）", () => {
  const variants = [
    summarizeRun(stopped),
    summarizeRun(full),
    ...["gameRunning", "noOptionsTxt", "needsBackupChoice", "needsOverwriteConfirm", "forkNeeded"].map((applyStatus) =>
      summarizeRun({ ...stopped, applyStatus })
    ),
    ...["quota", "auth", "relogin", "local_stuck", "local_gone", "no_output", "network", "other"].map((cause) =>
      summarizeRun({ ...stopped, interruption: { ...stopped.interruption, cause, stoppedByUser: false } })
    ),
  ];
  for (const s of variants) {
    for (const ctx of [{ packName: "P" }, { packName: "P", applyFailure: "網路磁碟或遊戲資料夾連不上" }, { packName: "P", firstTime: false }]) {
      const card = resultCardState(s, ctx);
      assert.ok(card.primary === null ? card.id === "S17" : !!card.primary.label, card.id);
      assert.ok(card.secondary.length <= 3);
      assert.ok(len(card.sentence.replace("「P」", "")) <= 40, card.sentence);
      if (card.id === "S14") assert.ok(len(card.detailLines[0] || "") <= 40, card.detailLines[0]);
      for (const b of [card.primary, ...card.secondary].filter(Boolean)) assert.ok(len(b.label) >= 2 && len(b.label) <= 10, b.label);
    }
  }
});
