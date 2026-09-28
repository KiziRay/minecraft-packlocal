/**
 * B5a-1 移除翻譯（D-05）與刪除結果並重翻（D-08）：危險、預設取消、不再寫「所有備份」、
 * 失敗原因就地帶回；移除翻譯與按鈕位置只在 D 區（全工具唯一入口）。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { D05, D08, createRemovalFlow } from "./removal-flow.js";
import { initialFocusRole } from "../ui/modal-scope.js";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel) => readFileSync(join(here, rel), "utf8").replace(/\r\n/g, "\n");

function makeFlow({ confirm = true, gameClosed = true, invokeImpl } = {}) {
  const calls = { invoke: [], confirms: [] };
  const flow = createRemovalFlow({
    invoke: async (cmd, args) => {
      calls.invoke.push({ cmd, args });
      return invokeImpl ? invokeImpl(cmd, args) : { removed: 1, restored: 1 };
    },
    confirmDialog: async (opts) => {
      calls.confirms.push(opts);
      return confirm;
    },
    ensureGameClosed: async () => gameClosed,
  });
  return { flow, calls };
}

test("D-05／D-08：標題是問句、危險、預設焦點取消、內文 ≤80 字且不寫「所有備份」", () => {
  for (const d of [D05, D08]) {
    assert.match(d.title, /？$/);
    assert.ok(Array.from(d.title).length <= 18, d.title);
    assert.equal(d.danger, true);
    assert.equal(initialFocusRole(d), "cancel", "危險對話框預設焦點＝取消");
    assert.ok(Array.from(d.body).length <= 80, d.body);
    assert.ok(!d.body.includes("所有備份"));
  }
});

test("按「移除翻譯」後直接 Enter（＝焦點上的取消）不會移除", async () => {
  const { flow, calls } = makeFlow({ confirm: false });
  const out = await flow.removeTranslation({ instancePath: "C:/Games/ATM10" });
  assert.equal(out.status, "cancelled");
  assert.equal(calls.invoke.length, 0);
  assert.equal(calls.confirms[0].title, D05.title);
});

test("確認後才移除；結果帶回給狀態卡 S19", async () => {
  const { flow, calls } = makeFlow();
  const out = await flow.removeTranslation({ instancePath: "C:/Games/ATM10", outputDir: "D:/out" });
  assert.equal(out.status, "removed");
  assert.deepEqual(calls.invoke[0], { cmd: "restore_last_apply_cmd", args: { instancePath: "C:/Games/ATM10", outputDir: "D:/out" } });
});

test("G1.30：遊戲開著時後端拒絕，白話原因就地帶回，不當成移除成功", async () => {
  const { flow } = makeFlow({
    invokeImpl: () => {
      throw "Minecraft 正在使用這個模組整合包，現在移除翻譯可能讓檔案被鎖住，所以一個檔都沒有動。";
    },
  });
  const out = await flow.removeTranslation({ instancePath: "C:/Games/ATM10" });
  assert.equal(out.status, "failed");
  assert.match(out.message, /一個檔都沒有動/);
});

test("刪除結果並重翻：確認後才刪，沒刪到就照實回報", async () => {
  const cancelled = makeFlow({ confirm: false });
  assert.equal((await cancelled.flow.deleteResultAndRestart({ outputDir: "D:/out" })).status, "cancelled");
  assert.equal(cancelled.calls.invoke.length, 0);
  const done = makeFlow({ invokeImpl: () => ({ deleted: true }) });
  assert.equal((await done.flow.deleteResultAndRestart({ outputDir: "D:/out" })).status, "deleted");
  assert.equal(done.calls.confirms[0].ackLabel, D08.ackLabel);
  const nothing = makeFlow({ invokeImpl: () => ({ deleted: false }) });
  assert.equal((await nothing.flow.deleteResultAndRestart({ outputDir: "D:/out" })).status, "nothing");
});

test("移除翻譯只在 D 區：index.html 只有一顆 #btn-restore，放在遊戲資料夾區；不再有「重新翻譯缺漏」「所有備份」", () => {
  const html = read("../index.html");
  assert.equal(html.split('id="btn-restore"').length - 1, 1);
  const pathBlock = html.slice(html.indexOf('<div class="path-block">'), html.indexOf('id="status-card"'));
  assert.ok(pathBlock.includes('id="btn-restore"'), "移除翻譯要在 D 區（遊戲資料夾區）");
  const app = read("../app.js");
  assert.ok(!app.includes("補翻時勾選「重新翻譯缺漏」"), "刪「重新翻譯缺漏」句");
  assert.ok(!app.includes("覆蓋範圍說明與所有備份"), "刪除結果不再寫「所有備份」");
  assert.ok(!app.includes("包含裡面的翻譯結果、覆蓋範圍說明與所有備份"));
  assert.ok(read("./pack-actions.js").includes("removalFlow.removeTranslation("));
});

test("中3：刪除結果全工具只剩一處（狀態卡「更多」→ D-08），本包選項不再有「刪除結果」", () => {
  const html = read("../index.html");
  const app = read("../app.js");
  assert.ok(!html.includes('id="btn-delete-output"'), "本包選項的重複入口要拿掉");
  assert.ok(!app.includes("btn-delete-output"));
  // 唯一剩下的直接呼叫是 onRunInner 的「翻完刪除翻譯結果」設定清理（不是玩家入口，本批不動）
  const calls = app.split("delete_result_folder_cmd").length - 1;
  assert.equal(calls, 1, "玩家入口只經 removalFlow 呼叫");
  const inner = app.slice(app.indexOf("async function onRunInner()"), app.indexOf("function isLegacySharedWorkPath"));
  assert.ok(inner.includes("delete_result_folder_cmd"));
  assert.ok(!app.includes('title: "刪除翻譯結果並重翻？"'), "D-08 文案只在 removal-flow.js 一處");
});
