import test from "node:test";
import assert from "node:assert/strict";

import { describeUpdateCheck, TEST_BUILD_MESSAGE } from "./update.js";

test("測試版：顯示白話訊息，不跳更新視窗", () => {
  const out = describeUpdateCheck({
    ok: true,
    testBuild: true,
    updateAvailable: false,
    message: TEST_BUILD_MESSAGE,
  });
  assert.equal(out.kind, "test-build");
  assert.equal(out.showModal, false);
  assert.ok(out.message.includes("測試版不自動更新"));
});

test("測試版就算誤帶 updateAvailable 也不可跳更新視窗", () => {
  const out = describeUpdateCheck({ ok: true, testBuild: true, updateAvailable: true, message: "" });
  assert.equal(out.showModal, false);
  assert.equal(out.message, TEST_BUILD_MESSAGE);
});

test("正式版：有新版才跳視窗；連不上與已是最新版都不跳", () => {
  assert.equal(describeUpdateCheck({ ok: true, updateAvailable: true, message: "有新版本" }).showModal, true);
  assert.equal(describeUpdateCheck({ ok: true, updateAvailable: false }).kind, "current");
  assert.equal(describeUpdateCheck({ ok: false, message: "暫時無法檢查更新（可能沒有網路）。" }).kind, "failed");
  assert.equal(describeUpdateCheck(null).showModal, false);
});
