/**
 * B5c 審查修正：探測結果有歸屬（換包不沿用上一包的探測）、套用後查遊戲時換包就中止、
 * 重開後探測不可信不寫數字、貼回後遊戲開著回 S11、S11 不重複「Minecraft 還開著」、另存只在真的開跑才用掉。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { createPackResults } from "./pack-results.js";
import { createResultActions } from "./result-actions.js";
import { createApplyPendingFlow } from "../ui/apply-pending.js";
import { computePackState } from "./pack-state.js";
import { resultCardState, summarizeProbe, summarizeRun } from "./result-card.js";

function harness({ running = false, onRunning = null } = {}) {
  const calls = { invoke: [], dialogs: 0 };
  const env = { current: "C:/games/A", probe: null };
  const store = createPackResults();
  const invoke = async (cmd, args) => {
    calls.invoke.push({ cmd, args });
    if (cmd === "is_game_running_cmd") {
      if (onRunning) onRunning(env);
      return { running };
    }
    if (cmd === "verify_resource_packs_cmd") return { listEmpty: false, presentButDisabled: [] };
    if (cmd === "apply_translation_to_game") return { status: "applied", zipCopied: "x.zip" };
    return null;
  };
  let actions;
  const applyPending = createApplyPendingFlow({
    invoke,
    confirmDialog: async () => ((calls.dialogs += 1), true),
    choiceDialog: async () => ((calls.dialogs += 1), "always"),
    appendLog: () => {},
    setBusy: () => {},
    saveBackupChoice: async () => {},
    onApplied: (r, c) => actions.onApplied(r, c),
    onPending: (r, c) => actions.onPending(r, c),
    onFailed: (reason, c) => actions.onFailed(reason, c),
  });
  actions = createResultActions({
    store,
    applyPending,
    invoke,
    appendLog: () => {},
    sync: () => {},
    currentPath: () => env.current,
    probe: () => env.probe,
    selectedOutputDir: () => "C:/out/A",
    packName: () => null,
    aiMode: () => "custom",
    onlineConfigured: () => false,
    disclosure: { isShown: () => true, retire: () => {} },
    canShare: () => true,
    canMergeTerms: () => false,
    backupChoice: () => "always",
    showBanner: () => {},
    offerFork: async () => true,
    openShare: () => {},
    openManualFix: () => {},
    openResultFolder: () => {},
    mergeTerms: () => {},
    openIssueReport: () => {},
  });
  const state = () =>
    computePackState({ consentAccepted: true, instancePath: env.current, validation: { ok: true }, hasResult: !!actions.stateInput(env.current).probeSummary, ...actions.stateInput(env.current) });
  return { actions, store, calls, env, state };
}

test("審查 1a：A 的探測結果在選到 B 時不沿用（狀態卡不畫 A 的 S11、按套用不會把 A 的結果寫進 B）", async () => {
  const h = harness();
  h.env.probe = { status: "ready", ownerPath: "C:/games/A", outputDir: "C:/out/A", shareable: true, completionPercent: 90 };
  h.env.current = "C:/games/B";
  assert.equal(h.actions.stateInput("C:/games/B").probeSummary, null);
  assert.notEqual(h.state().id, "S11");
  assert.equal(h.actions.contextFor("C:/games/B"), null, "沒有 B 自己的結果就沒有套用來源");
  await h.actions.apply();
  assert.equal(h.calls.invoke.filter((c) => c.cmd === "apply_translation_to_game").length, 0);
  // 回到 A：A 的探測照用
  h.env.current = "C:/games/A";
  assert.ok(h.actions.stateInput("C:/games/A").probeSummary);
  assert.equal(h.actions.contextFor("C:/games/A").outputDir, "C:/out/A");
});

test("審查 1b：按套用查遊戲期間換了資料夾就中止，不套用", async () => {
  const h = harness({ onRunning: (env) => (env.current = "C:/games/B") });
  await h.actions.finishRun({ applyStatus: "gameRunning", coveragePercent: 90 }, { instancePath: "C:/games/A", outputDir: "C:/out/A" });
  await h.actions.apply();
  assert.equal(h.calls.invoke.filter((c) => c.cmd === "apply_translation_to_game").length, 0);
});

test("審查 3a：重開後探測到的最新一輪沒套用 → S11；計數不可信時不寫數字也不寫「已套用」", () => {
  const notApplied = summarizeProbe({ status: "ready", lastApplied: false, countsTrusted: true, completionPercent: 80, pendingCount: 5 });
  assert.equal(notApplied.applyStatus, "notApplied");
  const card = computePackState({ consentAccepted: true, instancePath: "C:/games/A", validation: { ok: true }, hasResult: true, hasTranslationRecord: true, probeSummary: notApplied });
  assert.equal(card.id, "S11");
  const untrusted = resultCardState(summarizeProbe({ status: "partial", lastApplied: true, countsTrusted: false, completionPercent: 40, pendingCount: 99 }), { packName: "A" });
  assert.ok(!/\d+%|\d+ 條/.test(untrusted.sentence), untrusted.sentence);
  assert.ok(!untrusted.sentence.includes("已套用"), untrusted.sentence);
  // 舊版探測沒有旗標：照舊（已套用紀錄存在時當已套用、計數可信）
  assert.equal(summarizeProbe({ status: "ready", completionPercent: 90 }).applyStatus, "applied");
});

test("審查 3b：貼回後自動套用遇遊戲開著 → 顯示 S11 並就地說原因（不是停在 S17 無提示）", async () => {
  const h = harness({ running: true });
  await h.actions.finishRun({ applyStatus: "applied", coveragePercent: 97, pendingCount: 0 }, { instancePath: "C:/games/A", outputDir: "C:/out/A" });
  assert.equal(h.state().id, "S17");
  await h.actions.apply();
  const st = h.state();
  assert.equal(st.id, "S11");
  assert.ok(st.disabledReason.includes("Minecraft 還開著"));
});

test("審查 3c：S11 的「Minecraft 還開著」只在一處（狀態句或就地原因擇一）", () => {
  const s = summarizeRun({ applyStatus: "gameRunning", coveragePercent: 90 });
  const card = resultCardState(s, { gameStillRunning: true });
  const where = [card.sentence, card.disabledReason].filter((t) => t.includes("Minecraft 還開著"));
  assert.equal(where.length, 1, JSON.stringify(where));
});

test("審查 2／1a 接線：另存只在所有早退之後才用掉；手動輸入與瀏覽共用換資料夾收尾", () => {
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8").replace(/\r\n/g, "\n");
  const body = (h) => app.slice(app.indexOf(h), app.indexOf("\n}\n", app.indexOf(h)));
  const run = body("async function onRunInner(");
  assert.ok(run.indexOf("runNewCopyOnce = false") > run.indexOf("isSupportedMinecraftVersion(targetVersion)"), "版本擋下後才消耗");
  assert.ok(run.indexOf("runNewCopyOnce = false") < run.indexOf('setBusy(true, "translate")'));
  for (const fn of ["async function adoptInstancePath(", "async function onInstanceTypedPath("]) {
    assert.ok(body(fn).includes("beginFolderChange("), fn);
  }
  const change = body("function beginFolderChange(");
  for (const want of ["hideLocalCacheCard()", "resultActions.onFolderChanged(", "runNewCopyOnce = false", 'dataset.autoPath = ""']) {
    assert.ok(change.includes(want), want);
  }
  const flow = readFileSync(new URL("./run-flow.js", import.meta.url), "utf8");
  assert.ok(!flow.includes("handOffToSupplement"), "2b 死碼刪除");
});

test("審查 6：N-09 只在完整跑完後、完成卡收合（換資料夾）時才判斷；中途停下的輪次不算", async () => {
  const seen = { clean: 0, collapsed: 0 };
  const h = harness();
  const actions = createResultActions({
    ...{ store: createPackResults(), applyPending: { handle: async () => true } },
    invoke: async (cmd) => (cmd === "verify_resource_packs_cmd" ? { presentButDisabled: [] } : null),
    appendLog: () => {},
    sync: () => {},
    currentPath: () => "C:/games/A",
    probe: () => null,
    disclosure: { isShown: () => true, retire: () => {} },
    onCleanCompletion: () => (seen.clean += 1),
    onCardCollapsed: () => (seen.collapsed += 1),
  });
  void h;
  await actions.finishRun({ applyStatus: "applied", pendingCount: 10, interruption: { stoppedByUser: true, cause: "user_stop" } }, { instancePath: "C:/games/A", outputDir: "o" });
  assert.equal(seen.clean, 0, "中途停下不算一次完成");
  await actions.finishRun({ applyStatus: "applied", pendingCount: 0, coveragePercent: 99 }, { instancePath: "C:/games/A", outputDir: "o" });
  assert.equal(seen.clean, 1);
  assert.equal(seen.collapsed, 0, "完成卡還開著時不出");
  actions.onFolderChanged("C:/games/A");
  assert.equal(seen.collapsed, 1);
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  assert.ok(!app.includes("scheduleUsageFeedbackNudge"), "不再在翻完當下排程");
  assert.ok(app.includes("title: `${jobName}還在進行，要結束工具嗎？`"), "審查 7：D-15 標題照實寫");
});

test("審查 1：後端以歸屬拒絕時，S11 失敗原因白話寫屬於哪一包", async () => {
  const { applyFailureReason } = await import("../ui/apply-pending.js");
  assert.equal(
    applyFailureReason("這份翻譯結果屬於「ATM10」，不是「ATM10-copy」，沒有套用（一個檔都沒動）。"),
    "這份結果屬於「ATM10」"
  );
  const s = summarizeRun({ applyStatus: "gameRunning" });
  const card = resultCardState(s, { applyFailure: applyFailureReason("這份翻譯結果屬於「ATM10」，不是「B」") });
  assert.equal(card.sentence, "套用沒有完成：這份結果屬於「ATM10」");
});
