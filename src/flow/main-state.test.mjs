import test from "node:test";
import assert from "node:assert/strict";

import { MAIN_STATE_EVENT, createMainStateBroadcaster, mainStateSnapshot } from "./main-state.js";

test("主視窗狀態回報：只送設定視窗要的欄位", () => {
  const snap = mainStateSnapshot({ busy: 1, instancePath: " D:/pack ", outputDir: null, packName: "ATM10", apiKey: "sk-x" });
  assert.deepEqual(snap, { busy: true, instancePath: "D:/pack", outputDir: "", packName: "ATM10" });
});

test("狀態沒變不重送；設定視窗要求時強制送一次", () => {
  const sent = [];
  let state = { busy: false, instancePath: "D:/a" };
  const broadcast = createMainStateBroadcaster({ emit: (e, p) => sent.push([e, p]), read: () => state });
  assert.equal(broadcast(), true);
  assert.equal(broadcast(), false);
  assert.equal(broadcast({ force: true }), true);
  state = { busy: true, instancePath: "D:/a" };
  assert.equal(broadcast(), true);
  assert.equal(sent.length, 3);
  assert.equal(sent[2][0], MAIN_STATE_EVENT);
  assert.equal(sent[2][1].busy, true);
});

test("emit 失敗不拋錯（設定視窗沒開）", () => {
  const broadcast = createMainStateBroadcaster({
    emit: () => Promise.reject(new Error("no window")),
    read: () => ({}),
  });
  assert.doesNotThrow(() => broadcast());
});
