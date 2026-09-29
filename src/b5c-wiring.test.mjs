/**
 * B5c 接線（原始碼檢查）：三選一、前端 headline、maybeHintAiQuota、三個不合規 danger 對話框已刪；
 * 待套用卡／本機已有翻譯卡／整合包資訊卡併入狀態卡；人工補翻與分享改浮層並接上既有 command。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const app = readFileSync(new URL("./app.js", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = readFileSync(new URL("./index.html", import.meta.url), "utf8").replace(/\r\n/g, "\n");

function body(name) {
  const at = app.indexOf(name);
  assert.ok(at >= 0, `找不到 ${name}`);
  const rest = app.slice(at);
  const end = rest.indexOf("\n}\n");
  return rest.slice(0, end < 0 ? rest.length : end);
}

test("三選一彈窗不再出現（由狀態卡主要按鈕與 §3.1「更多」取代）", () => {
  assert.ok(!app.includes("這個整合包已經翻譯過了，這次想怎麼做"));
  assert.ok(!body("async function onRunInner(").includes("choiceDialog("), "開始翻譯不再跳任何選項對話框");
  const packState = readFileSync(new URL("./flow/pack-state.js", import.meta.url), "utf8");
  const packActions = readFileSync(new URL("./flow/pack-actions.js", import.meta.url), "utf8");
  assert.ok(packState.includes('"run-new-copy"') && packActions.includes('"run-new-copy"'), "另存一份新的結果改在 §3.1 更多");
  assert.ok(app.includes("function onRunNewCopy("));
});

test("完成段：刪前端 headline 與 maybeHintAiQuota；結論交給完成卡（resultActions.finishRun）", () => {
  assert.ok(!app.includes("可以直接開遊戲了"));
  assert.ok(!app.includes("maybeHintAiQuota"));
  assert.ok(!app.includes("本次共記錄 "), "錯誤行數只在紀錄（整段刪）");
  for (const fn of ["async function onRunInner(", "async function onSupplementInner(", "async function onRepairInner("]) {
    assert.ok(body(fn).includes("resultActions.finishRun("), `${fn} 結束走完成模式`);
    assert.ok(!body(fn).includes("applyPending.handle("), `${fn} 套用流程交給 finishRun`);
  }
  assert.ok(!app.includes("補譯完成！"));
});

test("不合規的 danger 對話框：遊戲好像還開著刪、確定不備份嗎刪、翻譯中關閉改 D-15（不是紅色、焦點在繼續執行）", () => {
  assert.ok(!app.includes("遊戲好像還開著"));
  assert.ok(!app.includes("async function ensureGameClosed("));
  assert.ok(!app.includes("確定不備份嗎"));
  const close = body("async function handleCloseWhileBusy(");
  assert.ok(close.includes("要結束工具嗎？"), "D-15 標題問句");
  assert.ok(!close.includes("danger: true"));
  assert.ok(close.includes('initialFocus: "cancel"'));
  assert.ok(!app.includes('title: report.listEmpty ? "資源包清單是空的"'), "資源包清單修復改自動（§3.5）");
});

test("舊卡併入狀態卡：待套用卡、本機已有翻譯卡、整合包資訊卡 DOM 已刪", () => {
  for (const id of ["apply-pending-card", "local-cache-card", "pack-meta-card", "btn-cache-share", "btn-package"]) {
    assert.ok(!html.includes(`id="${id}"`), `${id} 還在`);
  }
  assert.ok(!app.includes("applyPending.hideCard"));
  assert.ok(!app.includes("function wirePackMetaCard("));
});

test("人工補翻浮層：複製沒翻到的、貼回翻譯有接線；貼回後自動重新套用", () => {
  const overlay = html.slice(html.indexOf('id="manual-fix-overlay"'));
  assert.ok(html.includes('id="manual-fix-overlay"'));
  assert.ok(overlay.indexOf('id="btn-copy-failed"') > 0 && overlay.indexOf('id="btn-import-translations"') > 0);
  assert.ok(/bind\("btn-copy-failed"/.test(app) && /bind\("btn-import-translations"/.test(app), "兩顆按鈕要接上");
  assert.ok(body("async function onImportTranslations(").includes("resultActions.apply("), "貼回後自動重新套用");
});

test("分享給朋友改浮層，本機舊結果也能分享；安裝步驟密碼只寫一處、不寫實例根目錄", () => {
  assert.ok(html.includes('id="share-overlay"'));
  const shareHtml = html.slice(html.indexOf('id="share-overlay"'));
  assert.ok(shareHtml.indexOf('id="btn-share-confirm"') > 0);
  const steps = body("async function copyShareUrl(");
  assert.ok(!steps.includes("實例"));
  assert.equal((steps.match(/cloud\.zeitfrei\.uk/g) || []).length, 1, "密碼只寫一處");
  assert.ok(!app.includes('complete && resultSource === "run"'), "分享不再只限這次跑完");
});

test("新浮層都登記焦點規範（規格 §6）", () => {
  const actions = readFileSync(new URL("./flow/pack-actions.js", import.meta.url), "utf8");
  assert.ok(actions.includes('$("share-overlay")') && actions.includes('$("manual-fix-overlay")'));
});

test("中途停下時右欄進度也不寫「完成」", () => {
  const run = body("async function onRunInner(");
  assert.ok(!run.includes("本輪流程完成"));
  assert.ok(run.includes("這一輪中途停下，已翻好的部分已保留"));
});
