/**
 * B4 #8：本地模型翻完自動關閉——以輪次編號避免關閉競態。
 *
 * 實際情境：第 1 輪翻完，關閉本地模型的動作還在路上（要先查狀態），使用者立刻按
 * 「接續補完」開始第 2 輪。舊版那個遲到的關閉會把第 2 輪正在用的模型關掉，補完中途失敗。
 */
import test from "node:test";
import assert from "node:assert/strict";

import { createLocalModelRounds } from "./local-model-round.js";

function harness({ mode = "local" } = {}) {
  const calls = [];
  const logs = [];
  let backendRound = 0;
  const invoke = async (cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === "local_llm_begin_round_cmd") {
      backendRound += 1;
      return backendRound;
    }
    if (cmd === "local_llm_release_after_run_cmd") {
      const stopped = args.round === backendRound;
      return {
        stopped,
        message: stopped ? `第 ${args.round} 輪結束，已關閉本地模型` : `第 ${args.round} 輪不關（第 ${backendRound} 輪在用）`,
      };
    }
    throw new Error(`unexpected ${cmd}`);
  };
  const rounds = createLocalModelRounds({
    invoke,
    isLocalMode: () => mode === "local",
    log: (m) => logs.push(m),
    onStopped: () => logs.push("refreshed"),
  });
  return { rounds, calls, logs };
}

test("每一輪開始都拿到新的輪次編號", async () => {
  const { rounds } = harness();
  assert.equal(await rounds.begin(), 1);
  assert.equal(await rounds.begin(), 2);
  assert.equal(rounds.current(), 2);
});

test("同一輪結束：照常關閉本地模型並寫日誌", async () => {
  const { rounds, calls, logs } = harness();
  const r = await rounds.begin();
  await rounds.release(r);
  assert.ok(calls.some(([cmd, a]) => cmd === "local_llm_release_after_run_cmd" && a.round === r));
  assert.ok(logs.some((m) => m.includes("已關閉本地模型")), logs.join("\n"));
  assert.ok(logs.includes("refreshed"));
});

test("翻完立刻按接續補完：舊的關閉不得關掉新一輪要用的模型", async () => {
  const { rounds, calls, logs } = harness();
  const first = await rounds.begin();
  const second = await rounds.begin(); // 使用者立刻開始第 2 輪
  await rounds.release(first); // 第 1 輪的關閉才遲遲到達
  assert.ok(
    !calls.some(([cmd, a]) => cmd === "local_llm_release_after_run_cmd" && a.round === first),
    "舊輪次不可以送出關閉"
  );
  assert.ok(logs.some((m) => m.includes(`第 ${first} 輪`) && m.includes(`第 ${second} 輪`)), logs.join("\n"));
});

test("沒用本地模型時什麼都不做", async () => {
  const { rounds, calls } = harness({ mode: "gpt" });
  const r = await rounds.begin();
  await rounds.release(r);
  assert.equal(calls.length, 0);
});

test("後端說不關（別輪在用）時照實寫日誌、不刷新", async () => {
  const { rounds, logs } = harness();
  const r = await rounds.begin();
  // 模擬後端的輪次已被別處推進
  await rounds.release(r + 0);
  const before = logs.length;
  rounds._setCurrentForTest(r + 1);
  await rounds.release(r);
  assert.ok(logs.length > before);
});

test("審查 8：先告訴後端翻譯已結束，再送關閉請求（後端才不會以為還在翻而不關）", async () => {
  const order = [];
  const rounds = createLocalModelRounds({
    invoke: async (cmd) => {
      order.push(cmd);
      if (cmd === "local_llm_begin_round_cmd") return 1;
      return { stopped: true, message: "ok" };
    },
    isLocalMode: () => true,
    log: () => {},
    markIdle: async () => {
      order.push("mark-idle");
    },
  });
  const r = await rounds.begin();
  await rounds.release(r);
  assert.deepEqual(order, ["local_llm_begin_round_cmd", "mark-idle", "local_llm_release_after_run_cmd"]);
});
