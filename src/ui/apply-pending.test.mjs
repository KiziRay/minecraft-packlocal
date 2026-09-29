import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  APPLY_STATUS,
  applyFailureReason,
  NO_BACKUP_ACK,
  applyStatusOf,
  createApplyPendingFlow,
  isApplyPending,
  previewList,
} from "./apply-pending.js";

function makeFlow({ choice = null, confirm = true, applyResults = [], applyError = null } = {}) {
  const calls = { invoke: [], saved: [], confirms: [], choices: [], logs: [], pending: [], applied: [], failed: [] };
  const flow = createApplyPendingFlow({
    invoke: async (cmd, args) => {
      calls.invoke.push({ cmd, args });
      if (applyError) throw applyError;
      return applyResults.shift() || { status: "applied", playerSummary: "已把翻譯套用到遊戲" };
    },
    choiceDialog: async (opts) => {
      calls.choices.push(opts);
      return choice;
    },
    confirmDialog: async (opts) => {
      calls.confirms.push(opts);
      return confirm;
    },
    appendLog: (text, level) => calls.logs.push({ text, level }),
    setBusy: () => {},
    saveBackupChoice: async (value) => calls.saved.push(value),
    onPending: (result, context) => calls.pending.push({ result, context }),
    onApplied: (result, context) => calls.applied.push({ result, context }),
    onFailed: (reason, context) => calls.failed.push({ reason, context }),
  });
  return { flow, calls };
}

const ctx = { instancePath: "C:/game", outputDir: "C:/out", packName: "繁體中文翻譯" };

test("沒有狀態欄位的舊結果視為已套用；兩種結果形狀都認得", () => {
  assert.equal(applyStatusOf({}), APPLY_STATUS.applied);
  assert.equal(applyStatusOf({ applyStatus: "gameRunning" }), "gameRunning");
  assert.equal(applyStatusOf({ status: "noOptionsTxt" }), "noOptionsTxt");
  assert.equal(isApplyPending({ applyStatus: "applied" }), false);
});

test("B5c 遊戲開著：不是失敗、不開對話框，交給狀態卡 S11（onPending）；按套用沿用既有命令", async () => {
  const { flow, calls } = makeFlow();
  const done = await flow.handle({ applyStatus: "gameRunning", applyMessage: "翻譯已完成，但遊戲開著" }, ctx);
  assert.equal(done, false);
  assert.equal(calls.pending.length, 1);
  assert.equal(calls.confirms.length + calls.choices.length, 0);
  assert.equal(calls.invoke.length, 0, "遊戲開著時不自動重試");
  assert.equal(await flow.applyNow(ctx), true);
  assert.equal(calls.invoke[0].cmd, "apply_translation_to_game");
  assert.equal(calls.invoke[0].args.instancePath, "C:/game");
  assert.equal(calls.applied.length, 1);
});

test("第一次套用（D-04）：選要備份後寫回設定並重新套用；對話框標題問句、按鈕「先不要套用」", async () => {
  const { flow, calls } = makeFlow({ choice: "always" });
  const done = await flow.handle({ applyStatus: "needsBackupChoice" }, ctx);
  assert.equal(done, true);
  assert.deepEqual(calls.saved, ["always"]);
  assert.equal(calls.invoke.length, 1);
  assert.equal(calls.invoke[0].args.overwriteConfirmed, false);
  assert.equal(calls.choices[0].title, "要先備份會被覆蓋的原檔嗎？");
  assert.equal(calls.choices[0].cancelLabel, "先不要套用");
});

test("B5c D-04：選不備份在同一個框勾選「我了解…」，不再另開紅色「確定不備份嗎？」", async () => {
  const { flow, calls } = makeFlow({ choice: "never", confirm: true });
  await flow.handle({ applyStatus: "needsBackupChoice" }, ctx);
  assert.deepEqual(calls.choices[0].ack, { label: NO_BACKUP_ACK, forValue: "never" });
  assert.equal(calls.confirms.length, 0, "沒有第二個對話框");
  assert.deepEqual(calls.saved, ["never"]);
  assert.equal(calls.invoke[0].args.overwriteConfirmed, true);
  const src = readFileSync(new URL("./apply-pending.js", import.meta.url), "utf8");
  assert.ok(!src.includes("確定不備份嗎"));
});

test("關掉備份選擇對話框：不寫設定、不套用，回狀態卡 S11", async () => {
  const { flow, calls } = makeFlow({ choice: null });
  const done = await flow.handle({ applyStatus: "needsBackupChoice" }, ctx);
  assert.equal(done, false);
  assert.deepEqual(calls.saved, []);
  assert.equal(calls.invoke.length, 0);
  assert.equal(calls.pending.length, 1);
});

test("不備份模式：每次覆蓋前都確認，取消就不套用", async () => {
  const pending = { applyStatus: "needsOverwriteConfirm", pendingOverwrites: ["mods/a.jar"] };
  const yes = makeFlow({ confirm: true });
  assert.equal(await yes.flow.handle(pending, ctx), true);
  assert.equal(yes.calls.invoke[0].args.overwriteConfirmed, true);
  assert.deepEqual(yes.calls.confirms[0].affected, ["mods/a.jar"]);
  const no = makeFlow({ confirm: false });
  assert.equal(await no.flow.handle(pending, ctx), false);
  assert.equal(no.calls.invoke.length, 0);
});

test("B5a-1 D-03：覆蓋無備份的確認標題是問句、危險（預設焦點在取消）", async () => {
  const pending = { applyStatus: "needsOverwriteConfirm", pendingOverwrites: ["mods/a.jar"] };
  const { flow, calls } = makeFlow({ confirm: false });
  await flow.handle(pending, ctx);
  assert.equal(calls.confirms[0].title, "要覆蓋遊戲裡原本的檔案嗎？");
  assert.equal(calls.confirms[0].danger, true);
});

test("B5c：套用時複製資料夾被擋 → 回「已翻完未套用」（forkNeeded），不是失敗、不直接跳 D-10", async () => {
  const { flow, calls } = makeFlow({ applyError: "這份是複製出來的…請按「把這份當成新的整合包」" });
  assert.equal(await flow.applyNow(ctx), false);
  assert.equal(calls.failed.length, 0);
  assert.equal(calls.pending[0].result.applyStatus, "forkNeeded");
  assert.equal(calls.confirms.length, 0);
});

test("B5c：其他套用失敗 → S11 失敗變體的白話原因（不再一律「請確認遊戲已關閉」）", async () => {
  const { flow, calls } = makeFlow({ applyError: "寫入失敗：os error 53 找不到網路路徑" });
  assert.equal(await flow.applyNow(ctx), false);
  assert.equal(calls.failed[0].reason, "網路磁碟或遊戲資料夾連不上");
  assert.equal(applyFailureReason("被另一個程序使用中 (os error 32)"), "有檔案被占用，請先關閉遊戲");
  assert.equal(applyFailureReason("奇怪的錯誤"), "發生錯誤，完整原因在紀錄");
  const src = readFileSync(new URL("./apply-pending.js", import.meta.url), "utf8");
  assert.ok(!src.includes("apply-pending-card"), "待套用卡已併入狀態卡");
});


test("覆蓋清單過長時只列前幾個", () => {
  const list = Array.from({ length: 12 }, (_, i) => `f${i}`);
  const shown = previewList(list, 8);
  assert.equal(shown.length, 9);
  assert.match(shown[8], /另有 4 個/);
});

import { isBrokenRecordError } from "./apply-pending.js";
import { ROW_COPY } from "../settings/settings-copy.js";

// B5a-2：開始前的「保留／不保留」三選一刪除，改成設定「翻完刪除翻譯結果」（唯一位置）。
test("「翻完刪除翻譯結果」只管翻譯結果，不代表不備份（G1.8）", () => {
  assert.match(ROW_COPY.deleteResultsHelp, /翻譯結果/);
  assert.match(ROW_COPY.deleteResultsHelp, /備份照「套用前要不要備份」，不受影響/);
});

test("套用紀錄損壞的錯誤要能被辨認出來", () => {
  assert.equal(isBrokenRecordError("套用紀錄損壞，無法判斷…可以按「重設套用紀錄」"), true);
  assert.equal(isBrokenRecordError(new Error("套用紀錄損壞")), true);
  assert.equal(isBrokenRecordError("網路錯誤"), false);
});

test("主頁按鈕與提示統一叫「移除翻譯」", () => {
  const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  assert.match(html, /id="btn-restore"[^>]*>移除翻譯</);
  assert.ok(!html.includes("還原上一次套用"), "index.html 還有舊名稱");
  assert.ok(!app.includes("還原上一次套用"), "app.js 還有舊名稱");
  const flow = readFileSync(new URL("./apply-pending.js", import.meta.url), "utf8");
  assert.ok(flow.includes("reset_apply_record_cmd"), "紀錄損壞時要提供重設按鈕");
  assert.ok(app.includes("offerRecordReset"), "主視窗要在紀錄損壞時給重設按鈕");
});

test("複製出來的資料夾被拒絕時：提供「把這份當成新的整合包」，確認後才呼叫後端", async () => {
  const mod = await import("./apply-pending.js");
  assert.equal(typeof mod.isForkableError, "function");
  assert.equal(mod.isForkableError("…請按「把這份當成新的整合包」…"), true);
  assert.equal(mod.isForkableError("網路錯誤"), false);
  const calls = [];
  const deps = {
    confirmDialog: async () => true,
    invoke: async (cmd, args) => {
      calls.push([cmd, args]);
      return "ok";
    },
    appendLog: () => {},
  };
  assert.equal(await mod.offerForkInstance(deps, "D:/mc-copy"), true);
  assert.deepEqual(calls, [["fork_apply_instance_cmd", { instancePath: "D:/mc-copy" }]]);
  const declined = { ...deps, confirmDialog: async () => false };
  calls.length = 0;
  assert.equal(await mod.offerForkInstance(declined, "D:/mc-copy"), false);
  assert.equal(calls.length, 0, "沒按確認不能動");
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  assert.ok(app.includes("offerForkInstance"), "主視窗的套用／移除失敗也要給這個按鈕");
});

test("字體工具有自己的「移除字體包」，不跟「移除翻譯」混在一起", () => {
  const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  assert.match(html, /id="btn-font-remove"[^>]*>移除字體包</);
  assert.ok(app.includes('"remove_font_pack_cmd"'), "要呼叫後端的移除字體包");
});
