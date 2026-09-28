/**
 * B5a-1 首次流程：同意頁本版一次、Esc 不算同意、同意後接引導；引導依狀態、Esc＝跳過＋toast。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { CONSENT_COPY, CONSENT_CONTENT_VERSION, TOUR_SKIPPED_TOAST, afterConsent, isConsentAccepted } from "./first-run.js";
import { TOUR_STEPS, tourAdvance, tourMeta, tourNextLabel, tourPlan } from "../onboarding/tour-steps.js";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel) => readFileSync(join(here, rel), "utf8").replace(/\r\n/g, "\n");

test("同意頁本版一次：存的是本版內容版本才算同意；舊版的 \"1\" 不算", () => {
  assert.equal(isConsentAccepted(CONSENT_CONTENT_VERSION), true);
  assert.equal(isConsentAccepted("1"), false, "舊版勾「下次不再顯示」不等於看過新內容");
  assert.equal(isConsentAccepted(""), false);
  assert.equal(isConsentAccepted(null), false);
});

test("D-01：標題是問句、三句白話、不再說不修改原始檔", () => {
  assert.match(CONSENT_COPY.title, /？$/);
  assert.ok(Array.from(CONSENT_COPY.title).length <= 18);
  assert.equal(CONSENT_COPY.lines.length, 3);
  assert.ok(Array.from(CONSENT_COPY.lines.join("")).length <= 80);
  assert.match(CONSENT_COPY.lines[0], /放入翻好的模組檔/);
  const html = read("../index.html");
  const consent = html.slice(html.indexOf('id="consent-overlay"'), html.indexOf('id="onboard-root"'));
  for (const wrong of ["不修改原始 jar", "不會被直接改寫", "到診斷頁", "下次不再顯示"]) {
    assert.ok(!consent.includes(wrong), `同意頁還有「${wrong}」`);
  }
  assert.ok(consent.includes(CONSENT_COPY.title));
  for (const line of CONSENT_COPY.lines) assert.ok(consent.includes(line), `同意頁缺「${line}」`);
  assert.ok(consent.includes("詳細說明"));
});

test("同意後接引導；看過引導就不接", () => {
  assert.equal(afterConsent({ tourSeen: false }), "tour");
  assert.equal(afterConsent({ tourSeen: true }), "none");
});

test("同意頁 Esc 不作用、不算同意：app.js 不再有任何 Esc 關同意頁的路徑", () => {
  const app = read("../app.js");
  assert.ok(!app.includes("consent-dont-show"), "不再有「下次不再顯示」勾選");
  const escBlocks = app.split('ev.key === "Escape"').slice(1).map((s) => s.slice(0, 400));
  for (const block of escBlocks) assert.ok(!block.includes("hideConsentOverlay"), "Esc 不可關同意頁");
  // 浮層焦點登記在 flow/pack-actions.js（由 app.js 呼叫 packActions.wireOverlayFocus()）
  const actions = read("./pack-actions.js");
  assert.match(actions, /watchOverlay\(\$\("consent-overlay"\), \{\s*onEscape: null/);
  assert.ok(app.includes("packActions.wireOverlayFocus()"));
});

test("引導 4 步：第 1 步框「選擇遊戲資料夾」，第 4 步指問題回報", () => {
  assert.equal(TOUR_STEPS.length, 4);
  assert.equal(TOUR_STEPS[0].selector, "#btn-card-pick");
  assert.equal(TOUR_STEPS[3].selector, "#btn-issue-report");
  assert.equal(TOUR_STEPS[2].selector, "#btn-run", "第 3 步框狀態卡的主要按鈕");
  assert.equal(tourMeta(0), "1 / 4");
  for (const step of TOUR_STEPS) {
    assert.ok(!/下面的開始翻譯|錯誤分析|⋯ 裡/.test(step.body), `引導文案過時：${step.body}`);
  }
});

test("引導依狀態：沒資料夾時只出第 1 步，走完先暫停；選好資料夾後從第 2 步接著", () => {
  assert.deepEqual(tourPlan({ progress: 0, instanceReady: false }), { kind: "show", index: 0 });
  const next = tourAdvance({ index: 0 });
  assert.equal(next, 1);
  assert.deepEqual(tourPlan({ progress: next, instanceReady: false }), { kind: "pause" });
  assert.deepEqual(tourPlan({ progress: next, instanceReady: true }), { kind: "show", index: 1 });
  assert.deepEqual(tourPlan({ progress: 3, instanceReady: true }), { kind: "show", index: 3 });
  assert.deepEqual(tourPlan({ progress: 4, instanceReady: true }), { kind: "done" });
  assert.equal(tourNextLabel(0), "知道了");
  assert.equal(tourNextLabel(1), "下一步");
  assert.equal(tourNextLabel(3), "完成");
});

test("引導 Esc＝跳過並 toast 告知去哪找回（≤24 字）", () => {
  assert.match(TOUR_SKIPPED_TOAST, /設定→關於→重看引導/);
  assert.ok(Array.from(TOUR_SKIPPED_TOAST.replace(/\s/g, "")).length <= 24);
  const onboarding = read("../onboarding/onboarding.js");
  assert.ok(onboarding.includes("onSkipped"), "跳過時要通知主視窗顯示 toast");
  const app = read("../app.js");
  assert.ok(app.includes("TOUR_SKIPPED_TOAST"));
});
