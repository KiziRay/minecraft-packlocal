import test from "node:test";
import assert from "node:assert/strict";
import { isRunning, runExclusive } from "./once.js";

const tick = (ms = 5) => new Promise((r) => setTimeout(r, ms));

test("同一 key 執行中時，第二次呼叫被忽略", async () => {
  let started = 0;
  const slow = async () => {
    started += 1;
    await tick(20);
    return "done";
  };
  const first = runExclusive("dup", slow);
  let busyCalled = 0;
  const second = await runExclusive("dup", slow, { onBusy: () => (busyCalled += 1) });
  assert.equal(second, undefined, "第二次應該直接被忽略");
  assert.equal(busyCalled, 1, "應該通知呼叫端現在忙碌");
  assert.equal(await first, "done");
  assert.equal(started, 1, "昂貴動作只能啟動一次");
});

test("結束後可以再次執行", async () => {
  let runs = 0;
  const job = async () => {
    runs += 1;
  };
  await runExclusive("again", job);
  await runExclusive("again", job);
  assert.equal(runs, 2);
  assert.equal(isRunning("again"), false);
});

test("拋錯也要釋放，不可把按鈕永久卡死", async () => {
  await assert.rejects(
    runExclusive("boom", async () => {
      throw new Error("炸了");
    }),
    /炸了/
  );
  assert.equal(isRunning("boom"), false, "finally 必須釋放");
  let ran = false;
  await runExclusive("boom", async () => {
    ran = true;
  });
  assert.equal(ran, true, "錯誤之後仍要能重試");
});

test("不同 key 互不影響", async () => {
  const a = runExclusive("k1", async () => {
    await tick(20);
    return "a";
  });
  const b = await runExclusive("k2", async () => "b");
  assert.equal(b, "b");
  assert.equal(await a, "a");
});
