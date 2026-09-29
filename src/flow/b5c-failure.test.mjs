/**
 * B5c：S12 錯誤分類先用後端分類碼（interruption.cause／FailureClass 代碼），字串判斷只當退路（B5b 暫行）。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { classifyFailure, failureCodeOf } from "./run-failure.js";

test("有後端分類碼時照碼分類，不看中文句", () => {
  assert.equal(classifyFailure("看不懂的句子", "run", { code: "quota" }).kind, "quota");
  assert.equal(classifyFailure("看不懂的句子", "run", { code: "auth" }).kind, "key");
  assert.equal(classifyFailure("看不懂的句子", "run", { code: "auth_invalid" }).kind, "key");
  assert.equal(classifyFailure("看不懂的句子", "run", { code: "local_gone" }).kind, "local");
  assert.equal(classifyFailure("看不懂的句子", "run", { code: "local_stuck" }).kind, "local");
  assert.equal(classifyFailure("看不懂的句子", "run", { code: "relogin" }).kind, "discord");
  assert.equal(classifyFailure("看不懂的句子", "supplement", { code: "network" }).primary.action, "supplement");
  assert.equal(classifyFailure("空間不足", "run", { code: "disk_full" }).kind, "disk");
});

test("沒有碼（或碼看不懂）時退回字串判斷", () => {
  assert.equal(classifyFailure("HTTP 402 insufficient balance", "run").kind, "quota");
  assert.equal(classifyFailure("HTTP 402 insufficient balance", "run", { code: "???" }).kind, "quota");
  assert.equal(failureCodeOf({ code: "quota" }), "quota");
  assert.equal(failureCodeOf({ interruption: { cause: "auth" } }), "auth");
  assert.equal(failureCodeOf("字串錯誤"), "");
});
