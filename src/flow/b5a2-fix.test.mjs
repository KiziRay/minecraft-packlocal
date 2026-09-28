// B5a-2 審查修正 F3（移除翻譯期間算忙碌、設定視窗重送狀態請求）、F10（搬移工具資料期間主視窗停用開始翻譯）、
// 補充（「建議先啟動一次遊戲再翻譯」放回開始前畫面唯一位置）。
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { createRemovalFlow } from "./removal-flow.js";
import { DATA_MIGRATING_EVENT, createStateRequester } from "./main-state.js";
import { ACTION, STATE, computePackState } from "./pack-state.js";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel) => readFileSync(join(here, rel), "utf8").replace(/\r\n/g, "\n");

test("F3：移除翻譯只在真的呼叫後端時算忙碌（套用中），結束一定解除；取消不動忙碌狀態", async () => {
  const seen = [];
  const flow = createRemovalFlow({
    invoke: async () => {
      seen.push("invoke");
      return {};
    },
    confirmDialog: async () => true,
    onBusy: (on) => seen.push(on ? "busy" : "idle"),
  });
  await flow.removeTranslation({ instancePath: "D:/pack" });
  assert.deepEqual(seen, ["busy", "invoke", "idle"]);

  const failing = [];
  const bad = createRemovalFlow({
    invoke: async () => {
      throw new Error("遊戲還開著");
    },
    confirmDialog: async () => true,
    onBusy: (on) => failing.push(on),
  });
  const out = await bad.removeTranslation({ instancePath: "D:/pack" });
  assert.equal(out.status, "failed");
  assert.deepEqual(failing, [true, false], "失敗也要解除忙碌");

  const cancelled = [];
  const no = createRemovalFlow({ invoke: async () => ({}), confirmDialog: async () => false, onBusy: (on) => cancelled.push(on) });
  await no.removeTranslation({ instancePath: "D:/pack" });
  assert.deepEqual(cancelled, []);
  assert.ok(read("../app.js").includes('onBusy: (on) => setBusy(on, "apply")'), "移除翻譯期間主視窗 setBusy(apply)，設定視窗跟著停用");
});

test("F3：設定視窗收不到主視窗狀態時每 2 秒重送請求，最多 5 次；收到就停", () => {
  const timers = [];
  const sent = [];
  let has = false;
  const requester = createStateRequester({
    emit: (event) => sent.push(event),
    hasState: () => has,
    setTimer: (fn, ms) => timers.push({ fn, ms }),
  });
  requester.start();
  assert.equal(sent.length, 1);
  assert.equal(timers[0].ms, 2000);
  for (let i = 0; i < 10 && timers.length; i++) timers.shift().fn();
  assert.equal(sent.length, 6, "第一次＋最多重送 5 次");
  const sent2 = [];
  has = false;
  const timers2 = [];
  const r2 = createStateRequester({ emit: (e) => sent2.push(e), hasState: () => has, setTimer: (fn) => timers2.push(fn) });
  r2.start();
  has = true;
  timers2.shift()();
  assert.equal(sent2.length, 1, "收到狀態後不再重送");
  assert.ok(read("../settings/data-pane.js").includes("createStateRequester("));
});

test("F10：搬移工具資料期間，主視窗「開始翻譯」停用並就地說明", () => {
  const base = {
    consentAccepted: true,
    instancePath: "D:/Instances/ATM10",
    validation: { ok: true },
    busy: false,
    busyKind: "",
  };
  const s = computePackState({ ...base, busy: true, busyKind: "migrate" });
  assert.equal(s.id, STATE.busy);
  assert.equal(s.primary.action, ACTION.run);
  assert.equal(s.primary.disabled, true);
  assert.equal(s.disabledReason, "正在搬移工具資料，搬完才能開始");
  const pane = read("../settings/data-pane.js");
  assert.ok(pane.includes("DATA_MIGRATING_EVENT, { active: true }") && pane.includes("DATA_MIGRATING_EVENT, { active: false }"));
  assert.match(read("./settings-bridge.js"), /listen\(\s*DATA_MIGRATING_EVENT/);
  assert.equal(DATA_MIGRATING_EVENT, "mcpl:data-migrating");
  const actions = read("./pack-actions.js");
  assert.ok(actions.includes('s.dataMigrating ? "migrate"'), "主視窗狀態把搬移中當成忙碌");
});

test("補充：「建議先啟動一次遊戲再翻譯」回到開始前畫面，只在狀態卡「可開始」一處", () => {
  const s = computePackState({ consentAccepted: true, instancePath: "D:/Instances/ATM10", validation: { ok: true } });
  assert.equal(s.id, STATE.ready);
  assert.ok(s.detailLines.some((line) => line.includes("建議先啟動一次遊戲再翻譯")));
  assert.ok(s.detailLines.every((line) => [...line.replace(/\s/g, "")].length <= 40));
  const done = computePackState({ consentAccepted: true, instancePath: "D:/x", validation: { ok: true }, translationComplete: true });
  assert.ok(!done.detailLines.some((line) => line.includes("建議先啟動")), "已翻過就不再提");
  assert.ok(!read("../index.html").includes("建議先啟動一次遊戲再翻譯"), "只在一處");
});
