/**
 * B5b 接線（原始碼掃描）：按一次開始翻譯最多一個確認畫面；三個入口共用 AI 閘門；
 * 本地模型輪次、備份、停止的既有配對不變；刪掉的彈窗與舊控制項不再出現。
 *
 * 「按一次開始翻譯」可能出現的確認畫面（全部列明；流程層 mock 實跑見 flow/b5b-fix.test.mjs「4a／4c」）：
 *  - 一般情況（S13、AI 就緒、第一次在 §3.1 選備份或不備份並勾確認）：0 個（含套用階段）。
 *  - 例外 1：已有可用舊結果 → 「已有舊結果三選一」（計畫暫留，B5c 刪除）。
 *  - 例外 2：之前就選了不備份（設定裡）、這一輪要覆蓋遊戲原檔 → D-03（不可逆，每次問）。
 *    這一輪在 §3.1 已勾「我了解」時不跳（套用帶 overwriteConfirmed，後端 apply_translation_to_game 已接受，審查 4a）。
 *  - 例外 3：套用後資源包清單壞掉 → 修復確認（規格 §3.5 改自動屬完成段，B5c）。
 *  D-04 不會在開始翻譯後出現：§3.1 備份列在開跑前已記住選擇。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const app = readFileSync(join(here, "app.js"), "utf8");
const html = readFileSync(join(here, "index.html"), "utf8");
const localLlm = readFileSync(join(here, "ai/local-llm.js"), "utf8");

function fnBody(src, header) {
  const start = src.indexOf(header);
  assert.ok(start >= 0, `找不到 ${header}`);
  const open = src.indexOf("{", start + header.length - 1);
  let depth = 0;
  for (let i = open; i < src.length; i++) {
    if (src[i] === "{") depth++;
    else if (src[i] === "}") {
      depth--;
      if (depth === 0) return src.slice(start, i + 1);
    }
  }
  throw new Error("unbalanced");
}

const dialogCalls = (text) => (text.match(/\b(confirmDialog|choiceDialog)\s*\(/g) || []).length;

test("按一次開始翻譯：開跑前最多一個確認畫面（只有暫留的三選一），AI 問題不跳視窗", () => {
  const body = fnBody(app, "async function onRunInner() {");
  const before = body.slice(0, body.indexOf('setBusy(true, "translate")'));
  assert.equal(dialogCalls(before), 0, "B5c：三選一已刪，開跑前沒有任何對話框");
  assert.ok(!/AI 現在無法使用|被移動或刪除|補充漏翻/.test(body), "舊的 AI 無法使用對話框與說法已刪");
  assert.match(before, /runFlow\.beforeStart\("run"\)/);
  // 審查 1d／4a：AI 閘門之後、版本檢查之後才記住 §3.1 的選擇，並把本輪已確認帶進套用（B5c：經 finishRun）
  const commitAt = before.indexOf("runFlow.commitRunChoices()");
  assert.ok(commitAt > before.indexOf('runFlow.beforeStart("run")'), "閘門之後才記住");
  assert.match(body, /resultActions\.finishRun\(finished\.result, \{[\s\S]*?overwriteConfirmed,/);
});

test("三個入口（翻譯、修復、接續補完）都改用同一個 AI 閘門，沒通過就交給狀態卡，不開對話框", () => {
  for (const [header, origin] of [
    ["async function onRunInner() {", "run"],
    ["async function onRepairInner() {", "repair"],
    ["async function onSupplementInner({ skipAi = false, overwriteConfirmed = false } = {}) {", "supplement"],
  ]) {
    const body = fnBody(app, header);
    assert.match(body, new RegExp(`runFlow\\.beforeStart\\("${origin}"`), header);
    assert.doesNotMatch(body, /ensureAiReadyForAction|ensureCloudTopUpConsent/, header);
  }
  assert.doesNotMatch(app, /async function ensureAiReadyForAction|async function ensureCloudTopUpConsent/);
  assert.doesNotMatch(app, /CLOUD_TOPUP_CONSENT/, "線上補完同意框改成 §3.1 的列");
});

test("本地模型輪次、停止語意不變：三個入口各一組 begin／release（G0.5、G4.17、G4.18）", () => {
  for (const header of ["async function onRunInner() {", "async function onRepairInner() {", "async function onSupplementInner({ skipAi = false, overwriteConfirmed = false } = {}) {"]) {
    const body = fnBody(app, header);
    assert.equal((body.match(/await localModelRounds\.begin\(\)/g) || []).length, 1, header);
    assert.equal((body.match(/void releaseLocalModelAfterRun\(modelRound\)/g) || []).length, 1, header);
    assert.ok(body.indexOf("finally") < body.indexOf("releaseLocalModelAfterRun(modelRound)"), "關閉在 finally");
  }
  const stop = fnBody(app, "async function onStop() {");
  assert.match(stop, /STOP_LABELS\.sending/, "停止鈕文字仍由 stop-button 管（G4.25）");
});

test("備份決策不變：開始翻譯仍照 translate.backupChoice（後端讀），不保留結果不影響備份（G1.8）", () => {
  const body = fnBody(app, "async function onRunInner() {");
  assert.match(body, /keepResults: !skipResultFolder/);
  assert.match(body, /currentBackupChoice\(\)/);
  assert.ok(!/backup-before-apply|BACKUP_STORAGE_KEY|saveBackupPreference|loadBackupPreference/.test(app), "app.js 不再引用 #backup-before-apply");
  assert.ok(!/backup-before-apply/.test(html), "index.html 不再有 #backup-before-apply");
});

test("本地模型浮層：「完成，回到翻譯」；閘門不自動開浮層（不再疊「AI 無法使用」）", () => {
  assert.match(html, /id="btn-local-llm-start-translate"[^>]*>完成，回到翻譯</);
  assert.doesNotMatch(html + app + localLlm, /關閉並開始翻譯/);
  assert.match(localLlm, /openOverlay/);
  assert.match(app, /ensureLocalLlmReady\(\{[^}]*openOverlay: false/);
});

test("修復翻譯檔只在 S12 主要按鈕（#btn-repair 刪除，一顆按鈕一個位置）", () => {
  assert.doesNotMatch(html, /id="btn-repair"/);
  assert.doesNotMatch(app, /"btn-repair"/);
});

test("預設 AI 前端是本地模型（與後端 secrets.rs 一致，Rust 測試鎖住）；記住「不使用 AI」", () => {
  assert.match(html, /id="ai-source-local"[^>]*checked/);
  assert.doesNotMatch(html, /id="ai-source-(custom|gpt|none)"[^>]*checked/);
  assert.match(app, /USE_AI_KEY = "mcpl-use-ai"/);
});

test("狀態卡有 §3.1 列與進度區；右欄進度訊息翻譯中收起；徽章不寫「限流」", () => {
  for (const id of ["status-card-rows", "status-card-progress", "status-card-count", "status-card-eta", "status-card-badge", "status-card-reassure", "status-card-fill"]) {
    assert.match(html, new RegExp(`id="${id}"`), id);
  }
  assert.doesNotMatch(fnBody(app, "function stateBadgeLabel(state) {"), /限流/);
});
