/**
 * B5c 完成與套用的流程（mock 實跑 result-actions＋apply-pending＋pack-state）：
 * 換資料夾後套用不會套到上一包；遊戲開著不跳對話框；複製資料夾被擋當成新的後直接套用；資源包清單自動修。
 */
import test from "node:test";
import assert from "node:assert/strict";

import { createPackResults } from "./pack-results.js";
import { createResultActions, feedbackNudgeDue } from "./result-actions.js";
import { createApplyPendingFlow } from "../ui/apply-pending.js";
import { computePackState } from "./pack-state.js";

const full = {
  coveragePercent: 97,
  pendingCount: 0,
  interruption: { cause: null },
  displaySafety: { fontMayNotSupportChinese: [] },
};

function harness({ running = false, invokeImpl = null, forkOk = true } = {}) {
  const calls = { invoke: [], banners: [], logs: [], dialogs: 0, forks: 0 };
  let current = "C:/games/A";
  let probe = null;
  const outputs = { "C:/games/A": "C:/out/A", "C:/games/B": "C:/out/B" };
  const store = createPackResults();
  const invoke = async (cmd, args) => {
    calls.invoke.push({ cmd, args });
    if (invokeImpl) {
      const out = await invokeImpl(cmd, args);
      if (out !== undefined) return out;
    }
    if (cmd === "is_game_running_cmd") return { running };
    if (cmd === "verify_resource_packs_cmd") return { listEmpty: false, presentButDisabled: [] };
    if (cmd === "apply_translation_to_game") return { status: "applied", playerSummary: "已套用", zipCopied: "x.zip" };
    return null;
  };
  let actions;
  const applyPending = createApplyPendingFlow({
    invoke,
    confirmDialog: async () => {
      calls.dialogs += 1;
      return true;
    },
    choiceDialog: async () => {
      calls.dialogs += 1;
      return "always";
    },
    appendLog: (t) => calls.logs.push(t),
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
    appendLog: (t) => calls.logs.push(t),
    sync: () => {},
    currentPath: () => current,
    probe: () => probe,
    selectedOutputDir: () => outputs[current] || "",
    packName: () => null,
    aiMode: () => "custom",
    onlineConfigured: () => false,
    disclosure: { isShown: () => true, retire: () => {} },
    canShare: () => true,
    canMergeTerms: () => false,
    backupChoice: () => "always",
    showBanner: (b) => calls.banners.push(b),
    offerFork: async () => {
      calls.forks += 1;
      return forkOk;
    },
    openShare: () => {},
    openManualFix: () => {},
    openResultFolder: () => {},
    mergeTerms: () => {},
    openIssueReport: () => {},
  });
  return {
    actions,
    store,
    calls,
    select: (p) => {
      actions.onFolderChanged(current);
      current = p;
    },
    setProbe: (p) => (probe = p),
    state: () =>
      computePackState({ consentAccepted: true, instancePath: current, validation: { ok: true }, ...actions.stateInput(current) }),
  };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

test("開著遊戲翻完 → S11；切到 B 包再按套用不會套到 A；切回 A 仍是還沒套用，套用只套到 A", async () => {
  const h = harness();
  await h.actions.finishRun({ ...full, applyStatus: "gameRunning", applyMessage: "Minecraft 正在使用" }, { instancePath: "C:/games/A", outputDir: "C:/out/A" });
  assert.equal(h.state().id, "S11");
  assert.equal(h.calls.dialogs, 0, "遊戲開著不跳任何對話框");
  h.select("C:/games/B");
  assert.notEqual(h.state().id, "S11", "B 包沒有 A 的待套用卡");
  await h.actions.apply();
  for (const c of h.calls.invoke.filter((x) => x.cmd === "apply_translation_to_game")) {
    assert.equal(c.args.instancePath, "C:/games/B");
    assert.notEqual(c.args.outputDir, "C:/out/A", "不會把 A 的結果套進 B");
  }
  h.select("C:/games/A");
  assert.equal(h.state().id, "S11", "回 A：S11 再現");
  h.calls.invoke.length = 0;
  await h.actions.apply();
  const again = h.calls.invoke.filter((c) => c.cmd === "apply_translation_to_game");
  assert.equal(again.length, 1);
  assert.deepEqual([again[0].args.instancePath, again[0].args.outputDir], ["C:/games/A", "C:/out/A"]);
  assert.equal(h.state().id, "S17");
});

test("按套用時遊戲仍開著：不呼叫套用、不跳對話框，按鈕旁寫「Minecraft 還開著」", async () => {
  const h = harness({ running: true });
  await h.actions.finishRun({ ...full, applyStatus: "gameRunning" }, { instancePath: "C:/games/A", outputDir: "C:/out/A" });
  await h.actions.apply();
  assert.equal(h.calls.invoke.filter((c) => c.cmd === "apply_translation_to_game").length, 0);
  assert.equal(h.calls.dialogs, 0);
  const st = h.state();
  assert.equal(st.id, "S11");
  assert.equal(st.disabledReason, "Minecraft 還開著，請先關閉遊戲");
});

test("複製副本翻完才被擋：S11「已翻完，還沒套用」＋當成新的；確認後直接接著套用", async () => {
  const h = harness();
  await h.actions.finishRun({ ...full, applyStatus: "forkNeeded", applyMessage: "請按「把這份當成新的整合包」" }, { instancePath: "C:/games/A", outputDir: "C:/out/A" });
  const st = h.state();
  assert.equal(st.id, "S11");
  assert.ok(!st.sentence.includes("失敗"));
  assert.equal(st.primary.label, "當成新的模組整合包");
  h.actions.onAction(st.primary.action);
  for (let i = 0; i < 5; i += 1) await tick();
  assert.equal(h.calls.forks, 1);
  assert.equal(h.calls.invoke.filter((c) => c.cmd === "apply_translation_to_game").length, 1);
  assert.equal(h.state().id, "S17");
});

test("套用成功後資源包清單壞了就自動修（不跳確認），列在已幫你做的事", async () => {
  const h = harness({
    invokeImpl: (cmd) => {
      if (cmd === "verify_resource_packs_cmd") return { listEmpty: false, presentButDisabled: ["a.zip"], summary: "少了 1 個" };
      if (cmd === "repair_resource_packs_cmd") return { summary: "已修好" };
      return undefined;
    },
  });
  await h.actions.finishRun({ ...full, applyStatus: "applied", applyResult: { status: "applied", zipCopied: "x.zip" } }, { instancePath: "C:/games/A", outputDir: "C:/out/A" });
  assert.ok(h.calls.invoke.some((c) => c.cmd === "repair_resource_packs_cmd"));
  assert.equal(h.calls.dialogs, 0);
  assert.ok(h.state().detailLines.includes("・已修好資源包清單"));
});

test("N-05：有會換字體的資源包時出橫幅（同包同組一次）；N-09 第一次完成不出", async () => {
  const h = harness();
  const r = { ...full, applyStatus: "applied", displaySafety: { fontMayNotSupportChinese: ["Faithful"] } };
  await h.actions.finishRun(r, { instancePath: "C:/games/A", outputDir: "C:/out/A" });
  await h.actions.finishRun(r, { instancePath: "C:/games/A", outputDir: "C:/out/A" });
  assert.equal(h.calls.banners.filter((b) => b.id === "N-05").length, 1);
  assert.equal(feedbackNudgeDue({ completions: 1, roll: 0 }), false, "第一次翻完不會跳問卷");
  assert.equal(feedbackNudgeDue({ completions: 3, roll: 0.1 }), true);
  assert.equal(feedbackNudgeDue({ completions: 3, roll: 0.1, lastNudge: Date.now() - 1000 }), false);
});

test("本機舊結果（沒套用紀錄）按套用：用探測到的結果、只套到目前這包", async () => {
  const h = harness();
  h.select("C:/games/B");
  h.setProbe({ status: "ready", ownerPath: "C:/games/B", outputDir: "C:/out/B", shareable: true, completionPercent: 90 });
  await h.actions.apply();
  const c = h.calls.invoke.find((x) => x.cmd === "apply_translation_to_game");
  assert.deepEqual([c.args.instancePath, c.args.outputDir], ["C:/games/B", "C:/out/B"]);
});

test("移除翻譯後這包保存的完成結果作廢：換包再回來不會又說已套用", async () => {
  const h = harness();
  await h.actions.finishRun({ ...full, applyStatus: "applied" }, { instancePath: "C:/games/A", outputDir: "C:/out/A" });
  assert.equal(h.state().id, "S17");
  h.actions.forget("C:/games/A");
  h.select("C:/games/B");
  h.select("C:/games/A");
  assert.notEqual(h.state().id, "S17");
});
