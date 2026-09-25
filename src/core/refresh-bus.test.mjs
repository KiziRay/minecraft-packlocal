/**
 * 區塊刷新的三條硬規則（站長要求「不影響使用、不傷眼」）：
 *  1. 值沒變就不重畫
 *  2. 使用者正在用的區塊不要動
 *  3. 翻譯進行中不做背景輪詢
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  _peek,
  _resetForTests,
  configureRefreshBus,
  refreshRegion,
  registerRegion,
  trigger,
} from "./refresh-bus.js";

/** 這個模組會碰 document／window；在 node 下給最小替身。 */
function fakeDom({ page = "translate", activeTag = "BODY", activeIn = null, selection = null } = {}) {
  const el = { contains: (n) => n === activeIn || n === selection };
  globalThis.document = {
    body: { dataset: { appPage: page } },
    activeElement: { tagName: activeTag },
    getElementById: (id) => (id === "watched" ? el : null),
    addEventListener() {},
  };
  if (activeIn) globalThis.document.activeElement = { tagName: activeTag, __el: true };
  globalThis.window = {
    addEventListener() {},
    getSelection: () =>
      selection ? { isCollapsed: false, anchorNode: selection } : { isCollapsed: true },
  };
  return el;
}

test("值沒變就不必重畫（指紋一樣時不做多餘的 DOM 寫入）", async () => {
  _resetForTests();
  fakeDom();
  let calls = 0;
  registerRegion({
    id: "watched",
    when: ["focus"],
    refresh: () => {
      calls += 1;
      return "same-fingerprint";
    },
  });
  trigger("focus");
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(_peek("watched").fingerprint, "same-fingerprint", "指紋要被記下來");
  const before = calls;
  trigger("focus");
  await new Promise((r) => setTimeout(r, 5));
  // refresh 本身仍會被呼叫（要去問後端才知道有沒有變），但呼叫端靠指紋決定要不要動 DOM
  assert.ok(calls > before, "focus 時仍要去確認一次");
  assert.equal(_peek("watched").fingerprint, "same-fingerprint");
});

test("使用者正在該區塊輸入時不要打擾他", async () => {
  _resetForTests();
  const el = fakeDom({ activeTag: "INPUT" });
  const active = {};
  globalThis.document.activeElement = { tagName: "INPUT" };
  el.contains = () => true; // 焦點就在這個區塊裡
  let calls = 0;
  registerRegion({ id: "watched", when: ["focus"], refresh: () => { calls += 1; } });
  trigger("focus");
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(calls, 0, "焦點在區塊內的輸入框時，這一塊不可以被重畫");
  void active;
});

test("使用者正在選取這一塊的文字時也不要動", async () => {
  _resetForTests();
  const node = {};
  const el = fakeDom({ selection: node });
  el.contains = (n) => n === node;
  globalThis.document.activeElement = { tagName: "BODY" };
  let calls = 0;
  registerRegion({ id: "watched", when: ["focus"], refresh: () => { calls += 1; } });
  trigger("focus");
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(calls, 0, "重畫會把使用者選到一半的文字清掉");
});

test("翻譯進行中不做背景輪詢", async () => {
  _resetForTests();
  fakeDom();
  configureRefreshBus({ busy: () => true });
  let calls = 0;
  registerRegion({ id: "watched", when: ["interval", "focus"], refresh: () => { calls += 1; } });
  trigger("interval");
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(calls, 0, "翻譯中畫面在跑進度，任何額外重畫都會讓人覺得在閃");
  // 但明確要求的刷新仍要能跑（例如作業剛結束）
  refreshRegion("watched", "after-run");
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(calls, 1, "明確要求的刷新不受輪詢限制");
});

test("只刷新目前這一頁的區塊", async () => {
  _resetForTests();
  fakeDom({ page: "font" });
  let calls = 0;
  registerRegion({ id: "watched", scope: "translate", when: ["focus"], refresh: () => { calls += 1; } });
  trigger("focus");
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(calls, 0, "看不到的分頁不必刷新，白花時間又可能干擾");
});

test("單一區塊刷新失敗不影響其他區塊", async () => {
  _resetForTests();
  fakeDom();
  let good = 0;
  registerRegion({
    id: "watched",
    when: ["focus"],
    refresh: () => {
      throw new Error("這一塊壞了");
    },
  });
  registerRegion({ id: "other", when: ["focus"], refresh: () => { good += 1; } });
  trigger("focus");
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(good, 1, "壞掉的區塊不可以拖累其他區塊");
});
