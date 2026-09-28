/**
 * 審查 2：第一次按停止後，後端會「寫出已翻部分並裝進遊戲」；這時再按一次會放棄寫出與套用。
 * 按鈕文字必須講清楚，不能只寫「再次停止」。
 */
import test from "node:test";
import assert from "node:assert/strict";

import { STOP_LABELS } from "./stop-button.js";

test("第一次停止後的按鈕說明正在寫出，並警告再按會放棄套用", () => {
  assert.equal(STOP_LABELS.idle, "停止翻譯");
  assert.match(STOP_LABELS.afterFirstStop, /正在寫出已翻部分/);
  assert.match(STOP_LABELS.afterFirstStop, /再按會放棄套用/);
  assert.doesNotMatch(STOP_LABELS.afterFirstStop, /^再次停止$/);
  assert.match(STOP_LABELS.afterFirstStopTitle, /再按一次會放棄/);
});

test("F6：恢復停止按鈕時一併清掉「再按會放棄」的提示", async () => {
  const mod = await import("./stop-button.js");
  assert.equal(typeof mod.resetStopButton, "function", "要有統一的恢復函式");
  const btn = { disabled: true, textContent: STOP_LABELS.afterFirstStop, title: STOP_LABELS.afterFirstStopTitle };
  mod.resetStopButton(btn);
  assert.equal(btn.textContent, STOP_LABELS.idle);
  assert.equal(btn.title, "");
  assert.equal(btn.disabled, false);
});
