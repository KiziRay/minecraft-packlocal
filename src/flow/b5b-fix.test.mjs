/**
 * B5b 審查修正：以真的流程模組（run-flow、AI 閘門、套用流程）加 mock 後端實跑，不只掃字串。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { STATE, computePackState } from "./pack-state.js";
import { createRunFlow } from "./run-flow.js";
import { createAiGate, startLocalForRound } from "./run-start.js";
import { aiReadiness } from "./ai-readiness.js";
import { classifyFailure } from "./run-failure.js";
import { countLine } from "./run-progress.js";
import { APPLY_STATUS, createApplyPendingFlow } from "../ui/apply-pending.js";

const here = dirname(fileURLToPath(import.meta.url));
const base = { consentAccepted: true, instancePath: "C:/Games/ATM10", validation: { ok: true } };
const discordOk = { discordReady: true, loggedIn: true, inGuild: true, serviceAvailable: true };

function harness(over = {}) {
  const st = {
    status: { aiMode: "local", ...discordOk, localInstalled: true },
    useAi: true,
    backupChoice: "",
    path: base.instancePath,
    dialogs: [],
    invokes: [],
    localStarts: 0,
    saved: [],
    ...over,
  };
  const gate = createAiGate({
    useAi: () => st.useAi,
    refreshAiStatus: async () => st.status,
    gptUsable: async () => true,
    // 閘門不得啟動本地模型（G0.5：啟動要在輪次開始之後）
    startLocal: async () => {
      st.localStarts += 1;
      return true;
    },
  });
  const flow = createRunFlow({
    instancePath: () => st.path,
    aiStatus: () => st.status,
    useAi: () => st.useAi,
    backupChoice: () => st.backupChoice,
    gate,
    saveBackupChoice: async (v) => {
      st.saved.push(v);
      st.backupChoice = v;
    },
    saveCloudChoice: async () => {},
  });
  const dialog = (kind) => async (opts) => {
    st.dialogs.push([kind, opts && opts.title]);
    return st.answer ?? true;
  };
  const apply = createApplyPendingFlow({
    $: () => null,
    invoke: async (cmd, args) => {
      st.invokes.push([cmd, args]);
      return { applyStatus: APPLY_STATUS.applied };
    },
    confirmDialog: dialog("confirm"),
    choiceDialog: dialog("choice"),
    appendLog: () => {},
    setBusy: () => {},
    saveBackupChoice: async (v) => {
      st.backupChoice = v;
    },
  });
  const card = () => computePackState({ ...base, prestart: flow.prestartInput(), failure: flow.failureInput() });
  return { st, flow, apply, card };
}

/** 模擬 app.js onRunInner 的順序：閘門 → （三選一）→ 記住 §3.1 → 開跑 → 套用。回傳這次點擊出現的對話框。 */
async function clickStart(h, { backend, existingChoice = null } = {}) {
  const start = await h.flow.beforeStart("run");
  if (!start.ok) return { started: false };
  if (existingChoice !== null) {
    h.st.dialogs.push(["choice", "三選一"]);
    if (!existingChoice) return { started: false, cancelled: true };
  }
  await h.flow.commitRunChoices();
  const result = backend(h.st);
  await h.apply.handle(result, { instancePath: h.st.path, outputDir: "C:/out" }, 0, {
    overwriteConfirmed: h.flow.takeOverwriteConfirmed(),
  });
  return { started: true, useAi: start.useAi };
}

const neverBackend = (st) =>
  st.backupChoice === "never"
    ? { applyStatus: APPLY_STATUS.needsOverwriteConfirm, pendingOverwrites: ["options.txt"] }
    : { applyStatus: APPLY_STATUS.applied };

test("1a：接續補完／修復的 AI-BLOCKED 按「這次不用 AI」後，主要按鈕可按，而且真的以不用 AI 開跑（不迴圈）", async () => {
  for (const origin of ["supplement", "repair"]) {
    const h = harness({ status: { aiMode: "custom", ...discordOk, providerReady: false }, backupChoice: "always" });
    assert.equal((await h.flow.beforeStart(origin)).ok, false);
    assert.equal(h.card().id, STATE.aiBlocked);
    h.flow.onAction("ai-skip-once");
    const s = h.card();
    assert.equal(s.id, STATE.aiBlocked, "仍是原動作的卡，不跳回別的狀態");
    assert.equal(s.primary.action, origin);
    assert.equal(s.primary.disabled, undefined, "紅列處理了（這次不用 AI），可以按");
    const out = await h.flow.beforeStart(origin);
    assert.equal(out.ok, true);
    assert.equal(out.useAi, false);
  }
});

test("1b：開始翻譯選了「這次不用 AI」但三選一取消後，之後從卡片按接續補完照 AI 設定（不悄悄不用 AI）", async () => {
  const h = harness({ status: { aiMode: "gpt", ...discordOk, providerReady: false }, backupChoice: "always" });
  h.flow.onAction("ai-skip-once");
  const first = await clickStart(h, { existingChoice: false, backend: neverBackend });
  assert.equal(first.cancelled, true);
  const later = await h.flow.beforeStart("supplement");
  assert.equal(later.ok, false, "AI 沒就緒就擋下，不是悄悄不用 AI");
  // 三選一改成接續補完（同一次點擊）要明確帶入
  const h2 = harness({ status: { aiMode: "gpt", ...discordOk, providerReady: false }, backupChoice: "always" });
  h2.flow.onAction("ai-skip-once");
  const s = await h2.flow.beforeStart("run");
  const carried = await h2.flow.beforeStart("supplement", { skipAi: !s.useAi });
  assert.equal(carried.ok, true);
  assert.equal(carried.useAi, false);
  // 換資料夾清掉
  const h3 = harness({ status: { aiMode: "gpt", ...discordOk, providerReady: false }, backupChoice: "always" });
  h3.flow.onAction("ai-skip-once");
  h3.flow.onFolderChanged();
  assert.equal((await h3.flow.beforeStart("run")).ok, false);
});

test("1c：閘門不啟動本地模型；三選一取消後模型沒被啟動；啟動只在輪次開始之後（startLocalForRound）", async () => {
  const h = harness({ backupChoice: "always" });
  const out = await clickStart(h, { existingChoice: false, backend: neverBackend });
  assert.equal(out.cancelled, true);
  assert.equal(h.st.localStarts, 0);
  const order = [];
  await startLocalForRound({ useAi: true, localMode: true, start: async () => (order.push("start"), true) });
  assert.deepEqual(order, ["start"]);
  await startLocalForRound({ useAi: false, localMode: true, start: async () => (order.push("x"), true) });
  await startLocalForRound({ useAi: true, localMode: false, start: async () => (order.push("x"), true) });
  assert.deepEqual(order, ["start"], "不用 AI 或不是本地模型不啟動");
  await assert.rejects(startLocalForRound({ useAi: true, localMode: true, start: async () => false }), /本地模型啟動不起來/);
  const app = readFileSync(join(here, "../app.js"), "utf8");
  for (const header of ["async function onRunInner(", "async function onRepairInner(", "async function onSupplementInner("]) {
    const body = app.slice(app.indexOf(header), app.indexOf("\n}\n", app.indexOf(header)));
    const begin = body.indexOf("await localModelRounds.begin()");
    const startAt = body.indexOf("startLocalForRound(");
    assert.ok(begin > 0 && startAt > begin, `${header} 模型在輪次開始之後才啟動`);
    assert.ok(body.indexOf("try {", begin) < startAt, `${header} 啟動在 try 內（失敗也會 release）`);
  }
});

test("1d：§3.1 的選擇在三選一之後才記住（取消就不記）", async () => {
  const h = harness({ backupChoice: "" });
  await clickStart(h, { existingChoice: false, backend: neverBackend });
  assert.deepEqual(h.st.saved, []);
  await clickStart(h, { existingChoice: "overwrite", backend: neverBackend });
  assert.deepEqual(h.st.saved, ["always"]);
});

test("4a／4c：第一次在 §3.1 選不備份並勾確認 → 翻完不再跳 D-03（套用帶 overwriteConfirmed）；下一輪沒勾照跳", async () => {
  const h = harness({ backupChoice: "" });
  h.flow.onRowChange("backup", "never", true);
  const r1 = await clickStart(h, { backend: neverBackend });
  assert.equal(r1.started, true);
  assert.deepEqual(h.st.dialogs, [], "整次點擊（含套用）0 個確認畫面");
  const applyCall = h.st.invokes.find(([cmd]) => cmd === "apply_translation_to_game");
  assert.equal(applyCall[1].overwriteConfirmed, true);
  h.st.invokes = [];
  const r2 = await clickStart(h, { backend: neverBackend });
  assert.equal(r2.started, true);
  assert.equal(h.st.dialogs.length, 1, "下一輪沒在 §3.1 勾：D-03 照跳（每次）");
  assert.equal(h.st.dialogs[0][1], "要覆蓋遊戲裡原本的檔案嗎？");
});

test("4c：一般情況（AI 就緒、第一次選備份）整次點擊 0 個確認畫面；已有舊結果只多三選一這一個", async () => {
  const h = harness({ backupChoice: "" });
  assert.equal((await clickStart(h, { backend: neverBackend })).started, true);
  assert.deepEqual(h.st.dialogs, []);
  await clickStart(h, { existingChoice: "overwrite", backend: neverBackend });
  assert.equal(h.st.dialogs.length, 1);
});

test("4b：接續補完／修復的 AI-BLOCKED 只有 AI 列（不畫備份列與線上補完列，不因沒勾不備份而停用）", async () => {
  const h = harness({ status: { aiMode: "custom", ...discordOk, providerReady: false }, backupChoice: "" });
  h.flow.onRowChange("backup", "never", false);
  await h.flow.beforeStart("supplement");
  const s = h.card();
  assert.equal(s.id, STATE.aiBlocked);
  assert.deepEqual(s.rows.map((r) => r.id), ["ai"]);
  h.flow.onAction("ai-skip-once");
  assert.equal(h.card().primary.disabled, undefined);
});

test("6：分類順序——磁碟、網路在前；不再因 .zip／session 判成可修；401／402 要有 HTTP 字樣", () => {
  assert.equal(classifyFailure("寫入 C:/out/x.zip 失敗：磁碟空間不足").kind, "disk");
  assert.equal(classifyFailure("error sending request for url (https://x/x.zip)").kind, "network");
  assert.equal(classifyFailure("已完成 401 條後連線逾時").kind, "network");
  assert.equal(classifyFailure("讀取 session 失敗").kind, "other");
  assert.equal(classifyFailure("自訂 API 回 HTTP 401 unauthorized").kind, "key");
  assert.equal(classifyFailure("status 402 insufficient balance").kind, "quota");
  assert.equal(classifyFailure("找不到上次的工作階段").primary.label, "修復翻譯檔");
});

test("3a／3b：自訂 API 沒存金鑰寫「還沒填 API 金鑰」；記住的本地模型位置找不到時照實說並給重新設定", () => {
  assert.equal(aiReadiness({ status: { aiMode: "custom", ...discordOk, providerReady: false } }).sentence, "還沒填 API 金鑰");
  const moved = aiReadiness({ status: { aiMode: "local", ...discordOk, localInstalled: false }, localDirRemembered: true });
  assert.equal(moved.missing, "local-dir");
  assert.equal(moved.sentence, "找不到本地模型的安裝位置，請重新設定");
  assert.equal(moved.fix.label, "重新設定位置");
  assert.doesNotMatch(moved.sentence, /記憶體/);
});

test("5a：單位不明（空字串，可能是批次）不當成條數", () => {
  assert.equal(countLine({ done: 3, total: 10, unit: "" }), "");
  assert.equal(countLine({ done: 3, total: 10, unit: "條" }), "已完成 3／10 條");
});

test("5b／6b／7：右欄進度數字翻譯中收起；更換＝展開；關閉確認用詞「接續補完」", () => {
  const app = readFileSync(join(here, "../app.js"), "utf8");
  const setBusy = app.slice(app.indexOf("function setBusy("), app.indexOf("\n}\n", app.indexOf("function setBusy(")));
  for (const id of ["prog-pct", "prog-count", "prog-fill"]) assert.ok(setBusy.includes(`"${id}"`), id);
  assert.ok(/expandAi: \(\) => \{\s*aiRowExpanded = true;/.test(app), "展開，不是切換");
  assert.ok(!app.includes('"補充漏翻"') && !app.includes("「補充漏翻」"), "畫面字串不再用「補充漏翻」");
});
