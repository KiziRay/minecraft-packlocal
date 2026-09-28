import test from "node:test";
import assert from "node:assert/strict";
import {
  APPLY_STATUS,
  NO_BACKUP_ACK,
  applyStatusOf,
  createApplyPendingFlow,
  describeApplyPending,
  isApplyPending,
  previewList,
} from "./apply-pending.js";

function fakeDom() {
  const nodes = {
    "apply-pending-card": { hidden: true },
    "apply-pending-title": { textContent: "" },
    "apply-pending-message": { textContent: "" },
    "btn-apply-pending": {},
    "btn-apply-pending-dismiss": {},
  };
  return { $: (id) => nodes[id] || null, nodes };
}

function makeFlow({ choice = null, confirm = true, applyResults = [] } = {}) {
  const dom = fakeDom();
  const calls = { invoke: [], saved: [], confirms: [], logs: [] };
  const flow = createApplyPendingFlow({
    $: dom.$,
    invoke: async (cmd, args) => {
      calls.invoke.push({ cmd, args });
      return applyResults.shift() || { status: "applied", playerSummary: "已把翻譯裝進遊戲" };
    },
    choiceDialog: async () => choice,
    confirmDialog: async (opts) => {
      calls.confirms.push(opts);
      return confirm;
    },
    appendLog: (text, level) => calls.logs.push({ text, level }),
    setBusy: () => {},
    saveBackupChoice: async (value) => calls.saved.push(value),
  });
  return { flow, dom, calls };
}

const ctx = { instancePath: "C:/game", outputDir: "C:/out", packName: "繁體中文翻譯" };

test("沒有狀態欄位的舊結果視為已套用；兩種結果形狀都認得", () => {
  assert.equal(applyStatusOf({}), APPLY_STATUS.applied);
  assert.equal(applyStatusOf({ applyStatus: "gameRunning" }), "gameRunning");
  assert.equal(applyStatusOf({ status: "noOptionsTxt" }), "noOptionsTxt");
  assert.equal(isApplyPending({ applyStatus: "applied" }), false);
});

test("遊戲開著：不是失敗，顯示「已翻完，關掉遊戲後按套用到遊戲」並提供按鈕", async () => {
  const { flow, dom, calls } = makeFlow();
  const done = await flow.handle({ applyStatus: "gameRunning", applyMessage: "翻譯已完成，但遊戲開著" }, ctx);
  assert.equal(done, false);
  assert.equal(dom.nodes["apply-pending-card"].hidden, false);
  assert.match(dom.nodes["apply-pending-title"].textContent, /已翻完，關掉遊戲後按「套用到遊戲」/);
  assert.equal(calls.invoke.length, 0, "遊戲開著時不自動重試");
  // 關遊戲後按按鈕：沿用既有套用命令
  flow.wire();
  dom.nodes["btn-apply-pending"].onclick();
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(calls.invoke[0].cmd, "apply_translation_to_game");
  assert.equal(calls.invoke[0].args.instancePath, "C:/game");
});

test("第一次套用：選要備份後寫回設定並重新套用", async () => {
  const { flow, calls } = makeFlow({ choice: "always" });
  const done = await flow.handle({ applyStatus: "needsBackupChoice" }, ctx);
  assert.equal(done, true);
  assert.deepEqual(calls.saved, ["always"]);
  assert.equal(calls.invoke.length, 1);
  assert.equal(calls.invoke[0].args.overwriteConfirmed, false);
});

test("選不備份必須勾選「我了解之後無法還原被覆蓋的檔案」", async () => {
  const { flow, calls } = makeFlow({ choice: "never", confirm: true });
  await flow.handle({ applyStatus: "needsBackupChoice" }, ctx);
  assert.equal(calls.confirms[0].ackLabel, NO_BACKUP_ACK);
  assert.deepEqual(calls.saved, ["never"]);
  assert.equal(calls.invoke[0].args.overwriteConfirmed, true);
});

test("關掉備份選擇對話框：不寫設定、不套用，留說明卡", async () => {
  const { flow, dom, calls } = makeFlow({ choice: null });
  const done = await flow.handle({ applyStatus: "needsBackupChoice" }, ctx);
  assert.equal(done, false);
  assert.deepEqual(calls.saved, []);
  assert.equal(calls.invoke.length, 0);
  assert.equal(dom.nodes["apply-pending-card"].hidden, false);
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

test("沒有 options.txt：請先啟動一次遊戲", () => {
  assert.equal(describeApplyPending({ status: "noOptionsTxt" }).title, "請先啟動一次遊戲");
});

test("覆蓋清單過長時只列前幾個", () => {
  const list = Array.from({ length: 12 }, (_, i) => `f${i}`);
  const shown = previewList(list, 8);
  assert.equal(shown.length, 9);
  assert.match(shown[8], /另有 4 個/);
});

import { readFileSync } from "node:fs";
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
