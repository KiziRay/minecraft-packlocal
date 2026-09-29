// B5a-2 #2：詞表禁用詞在「開始前畫面」與設定視窗的可見文字中 0 命中（規格 §5.1）。
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { FORBIDDEN, TERMS, findForbidden } from "./terms.js";
import { sliceElement, visibleText, withoutElement } from "./visible-text.js";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel) => readFileSync(join(here, rel), "utf8").replace(/\r\n/g, "\n");
const indexHtml = read("../index.html");

/**
 * 開始前畫面＝標題列、分頁、翻譯頁（整合包區、狀態卡、AI 區）與本包選項。
 * 排除別批負責的舊卡（B5c：待套用卡、本機已有翻譯卡、整合包資訊卡），之後會併入狀態卡、文案由 B5c 依詞表改。
 * （B5d 的接續卡、寫入權限卡已刪，改由 D 區「上次」與狀態卡 S04 說，文案掃描見 flow/folder-state.test.mjs「詞表」。）
 */
function startScreenHtml() {
  // B5c：待套用卡、本機已有翻譯卡、整合包資訊卡已併入狀態卡（不再排除）；分享與人工補翻浮層一起掃
  const page = sliceElement(indexHtml, '<div id="page-translate"');
  void withoutElement;
  return [
    sliceElement(indexHtml, '<div id="share-overlay"'),
    sliceElement(indexHtml, '<div id="manual-fix-overlay"'),
    sliceElement(indexHtml, "<title>"),
    sliceElement(indexHtml, '<header class="winbar"'),
    sliceElement(indexHtml, '<nav id="workbench-tabs"'),
    page,
    sliceElement(indexHtml, '<div id="pack-options-modal"'),
  ].join("\n");
}

/** 原始碼裡的字串常值（雙引號、單引號、反引號；去掉註解），只留含中文的。 */
function literalsIn(text) {
  const source = String(text)
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/(^|[^:"'`\\])\/\/.*$/gm, "$1");
  return [...source.matchAll(/"((?:[^"\\\n]|\\.)*)"|'((?:[^'\\\n]|\\.)*)'|`((?:[^`\\]|\\.)*)`/g)]
    .map((m) => m[1] ?? m[2] ?? m[3])
    .filter((s) => /[一-鿿]/.test(s))
    .join("\n");
}

function stringLiterals(rel) {
  return literalsIn(read(rel));
}

/** app.js 裡把字寫進開始前畫面（整合包區、狀態卡、AI 區、本包選項）的函式（審查 F7）。 */
const APP_START_SCREEN_FUNCTIONS = [
  "function setAutoOutputDir(",
  "function outputStorageHint(",
  "function aiModeLabel(",
  "async function refreshAiStatus(",
  "function syncCustomProviderUi(",
  "function renderGptStatus(",
  "async function beginGptLogin(",
  "async function cancelGptLoginFlow(",
  "async function logoutGpt(",
  "function markGptLoginOverlayDone(",
  "async function adoptInstancePath(",
  "async function onPickInstance(",
  "async function refreshPackTranslationName(",
  "async function refreshReferencePack(",
  // B5c：完成與套用段寫給玩家看的字
  "async function copyShareUrl(",
  "function packageShare(",
  "async function onCopyFailedItems(",
  "async function onImportTranslations(",
  "async function handleCloseWhileBusy(",
  "async function usageFeedbackMaybeNudge(",
  "function onRunNewCopy(",
  "async function openCurrentResultFolder(",
];

function appStartScreenSource() {
  const app = read("../app.js");
  const parts = APP_START_SCREEN_FUNCTIONS.map((head) => {
    const at = app.indexOf(head);
    assert.ok(at >= 0, `app.js 找不到 ${head}`);
    return app.slice(at, app.indexOf("\n}\n", at));
  });
  // onRunInner 只掃「開始時寫給玩家看的那一段」（其餘屬 B5b／B5c）
  const from = app.indexOf("  const backupNote = {");
  const to = app.indexOf('setProgress(1, "準備中…");', from);
  assert.ok(from > 0 && to > from, "找不到開始翻譯時的說明段");
  parts.push(app.slice(from, to));
  return parts.join("\n");
}

const COPY_MODULES = [
  "../settings-window.js",
  "../settings/data-pane.js",
  "../flow/pack-state.js",
  "../onboarding/tour-steps.js",
  "../ai/copy.js",
  "../ui/help-copy.js",
  "../settings/settings-copy.js",
  "../onboarding/onboarding.js",
  "../flow/first-run.js",
  "../core/cloud-topup-consent.js",
  // B5b：開始前確認、E1 AI 列、翻譯中、S12 的文案
  "../flow/ai-readiness.js",
  "../flow/prestart.js",
  "../flow/run-progress.js",
  "../flow/run-failure.js",
  "../flow/run-flow.js",
  // B5c：完成卡、套用、人工補翻、分享的文案
  "../flow/result-card.js",
  "../flow/result-actions.js",
  "../flow/pack-results.js",
  "../ui/apply-pending.js",
];

function report(hits) {
  return hits.map((h) => `「${h.word}」→「${h.use}」：${h.line}`).join("\n");
}

test("掃描器本身：抓得到禁用詞、放過合法用法", () => {
  assert.equal(findForbidden("選擇模組整合包").length, 0);
  assert.equal(findForbidden("用 ChatGPT 翻").length, 0);
  assert.equal(findForbidden("人工補翻").length, 0);
  assert.equal(findForbidden("遊戲設定的「強制使用 Unicode 字型」").length, 0);
  for (const bad of ["選擇實例", "模組包翻譯工具", "整合包", "登入 GPT", "修復工作階段", "API Key", "補譯完成", "裝進遊戲", "字型"]) {
    assert.ok(findForbidden(bad).length > 0, `沒抓到：${bad}`);
  }
  assert.equal(TERMS.pack, "模組整合包");
  assert.ok(FORBIDDEN.length >= 20);
});

test("開始前畫面（整合包區、狀態卡、AI 區、本包選項）的可見文字沒有禁用詞", () => {
  const hits = findForbidden(visibleText(startScreenHtml()));
  assert.equal(hits.length, 0, report(hits));
});

test("設定視窗的可見文字沒有禁用詞", () => {
  const hits = findForbidden(visibleText(read("../settings.html")));
  assert.equal(hits.length, 0, report(hits));
});

test("開始前畫面的文案模組（狀態卡、引導、AI、「？」、設定）沒有禁用詞", () => {
  for (const rel of COPY_MODULES) {
    const hits = findForbidden(stringLiterals(rel));
    assert.equal(hits.length, 0, `${rel}\n${report(hits)}`);
  }
});

test("審查 F7：app.js 寫進開始前畫面與 AI 區的動態字串沒有禁用詞（含單引號字串）", () => {
  const hits = findForbidden(literalsIn(appStartScreenSource()));
  assert.equal(hits.length, 0, report(hits));
  assert.ok(findForbidden(literalsIn("x = '登出 GPT';")).length > 0, "單引號字串也要掃到");
});

test("使用者的測試清單：找不到「實例」「模組包」「GPT」「工作階段」", () => {
  const all = [
    visibleText(startScreenHtml()),
    visibleText(read("../settings.html")),
    ...COPY_MODULES.map(stringLiterals),
    literalsIn(appStartScreenSource()),
  ].join("\n");
  for (const word of ["實例", "模組包", "工作階段"]) assert.ok(!all.includes(word), word);
  assert.ok(!/(?<!Chat)GPT/.test(all), "GPT");
});
