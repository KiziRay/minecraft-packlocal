/**
 * B5b 第二輪修正：錯誤分類用後端真實產生的訊息字串（取自 deepseek.rs、lib.rs、server.rs、codex_chat.rs、timeouts.rs）；
 * 本地模型啟動期間按停止、啟動中的狀態句與紀錄、三選一轉接續補完前清「這次不用 AI」並記住備份。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { classifyFailure } from "./run-failure.js";
import { createRunFlow } from "./run-flow.js";
import { createAiGate, startLocalForRound, LOCAL_START_STOPPED } from "./run-start.js";

const here = dirname(fileURLToPath(import.meta.url));
const app = readFileSync(join(here, "../app.js"), "utf8");

// 後端原句（行號為取樣當下）
const SAMPLES = [
  // deepseek.rs:4890（strict 探測；{err} 是 reqwest 錯誤）
  ["本地模型連線失敗：error sending request for url (http://127.0.0.1:18765/health)", "local"],
  // server.rs:592
  ["本地模型已啟動，但健康檢查逾時。請稍後再試。", "local"],
  // deepseek.rs:282
  ["尚未安裝本地模型。請先同意並完成偵測／下載。", "local"],
  // server.rs:285
  ["找不到翻譯模型，請先完成安裝或改選已有模型的資料夾。", "local"],
  // deepseek.rs:3966（local_process_gone_message）
  ["本地模型程式已經結束（0xC0000409），通常是這台電腦的記憶體不足。已翻好的部分都會保留；", "local"],
  // timeouts.rs:89
  ["本地模型在這台電腦上一直等不到回應（這一輪累計等了約 31 分鐘），先停下本地翻譯。", "local"],
  // deepseek.rs:728
  ["AI 上游拒絕請求（401）。請稍後再試，或改用自訂 API。", "key"],
  // deepseek.rs:4909
  ["金鑰無效或無權限", "key"],
  // deepseek.rs:4912
  ["帳號餘額不足", "quota"],
  // deepseek.rs:640
  ["免費翻譯的當日額度已用完", "quota"],
  // deepseek.rs:464（額度訊息的 detail 可能帶 reqwest 字樣，不得判成網路）
  ["【自訂 API 額度或金鑰無法使用】\nerror sending request for url (https://api.deepseek.com/chat/completions)", "quota"],
  // deepseek.rs:449
  ["【ChatGPT 暫時不能翻譯】\nusage limit\n\n這次 ChatGPT 不接受翻譯請求，可能是翻譯用量已達上限", "quota"],
  // codex_chat.rs:359
  ["GPT 有回應，但沒有可用的 assistant text。", "gpt"],
  // deepseek.rs:4923
  ["連線失敗（無回應）：error sending request for url (https://api.deepseek.com/chat/completions)", "network"],
  // deepseek.rs:3668
  ["AI 連線中斷超過 300 秒仍未恢復（最後一次：timed out）。已翻好的部分都會保留；", "network"],
  // Windows 磁碟滿（std::io 錯誤原文）
  ["寫入 C:/out/翻譯結果/x.zip 失敗：There is not enough space on the disk. (os error 112)", "disk"],
  // app.js playerize（後端 login_required）
  ["翻譯前請先登入 Discord 並加入官方伺服器。", "discord"],
  // 狀態碼要有 HTTP／status 或（401）才算
  ["已完成 401 條後連線逾時", "network"],
  ["自訂 API 回應 HTTP 402 Payment Required", "quota"],
];

test("6：錯誤分類用後端真實訊息——本地模型、額度在網路之前；（401）判金鑰；Windows 磁碟滿", () => {
  assert.ok(SAMPLES.length >= 10);
  for (const [text, kind] of SAMPLES) {
    const got = classifyFailure(text).kind;
    const group = got === "local-start" ? "local" : got;
    assert.equal(group, kind, text);
  }
});

test("6：狀態碼判斷真的會命中（不是寫成字母 D 與退格）", () => {
  assert.equal(classifyFailure("status: 401 unauthorized").kind, "key");
  assert.equal(classifyFailure("HTTP 402").kind, "quota");
  assert.equal(classifyFailure("第 402 批失敗").kind, "other");
});

test("1c-1：本地模型啟動期間按了停止——啟動完成後照停止收尾（丟停止，不開跑、不算失敗）", async () => {
  let stopped = false;
  await assert.rejects(
    startLocalForRound({
      useAi: true,
      localMode: true,
      start: async () => {
        stopped = true; // 使用者在等待啟動時按了停止
        return true;
      },
      isStopping: () => stopped,
    }),
    (e) => e.message === LOCAL_START_STOPPED
  );
  // 沒按停止照常
  await startLocalForRound({ useAi: true, localMode: true, start: async () => true, isStopping: () => false });
  // app.js：停止訊息要被當成使用者停止（isCancellation），不當失敗
  assert.ok(LOCAL_START_STOPPED.includes("已依你的要求停止"));
});

test("1c-2／1c-3：啟動前狀態卡先說「正在啟動本地模型」；啟動失敗把原因寫進紀錄", async () => {
  const notes = [];
  const logs = [];
  await assert.rejects(
    startLocalForRound({
      useAi: true,
      localMode: true,
      start: async () => {
        throw new Error("找不到執行程式，請先完成本地模型安裝。");
      },
      onStarting: (m) => notes.push(m),
      log: (m) => logs.push(m),
    })
  );
  assert.deepEqual(notes, ["正在啟動本地模型"]);
  assert.ok(logs.some((l) => l.includes("找不到執行程式")), "原因寫進紀錄");
  const flow = createRunFlow({
    instancePath: () => "C:/A",
    aiStatus: () => null,
    useAi: () => true,
    backupChoice: () => "always",
    gate: { ensure: async () => ({ ready: true, useAi: true }) },
    saveBackupChoice: async () => {},
    saveCloudChoice: async () => {},
    disclosure: { isShown: () => true, retire: () => {} },
  });
  flow.beginRun({ localMode: true });
  flow.noteMessage("正在啟動本地模型");
  assert.equal(flow.progressInput("A").sentence, "正在啟動本地模型（第一次較久）");
});

test("1b／1d：三選一轉接續補完前清「這次不用 AI」並記住 §3.1 的備份（開始後不會跳 D-04）", async () => {
  const discordOk = { discordReady: true, loggedIn: true, inGuild: true, serviceAvailable: true };
  const st = { backupChoice: "", saved: [] };
  const flow = createRunFlow({
    instancePath: () => "C:/A",
    aiStatus: () => ({ aiMode: "gpt", ...discordOk, providerReady: false }),
    useAi: () => true,
    backupChoice: () => st.backupChoice,
    gate: createAiGate({ useAi: () => true, refreshAiStatus: async () => ({ aiMode: "gpt", ...discordOk, providerReady: false }), gptUsable: async () => false }),
    saveBackupChoice: async (v) => {
      st.saved.push(v);
      st.backupChoice = v;
    },
    saveCloudChoice: async () => {},
  });
  flow.onAction("ai-skip-once");
  const start = await flow.beforeStart("run");
  assert.equal(start.ok, true);
  // B5c 審查 2b：handOffToSupplement 已刪（三選一刪除後無呼叫端）；記住 §3.1 的選擇改由 commitRunChoices
  await flow.commitRunChoices();
  assert.deepEqual(st.saved, ["always"], "備份列在開跑前記住");
  const sup = await flow.beforeStart("supplement", { skipAi: !start.useAi });
  assert.equal(sup.useAi, false);
  // 之後從卡片按開始翻譯：「這次不用 AI」已清掉，AI 列照實變紅
  assert.equal((await flow.beforeStart("run")).ok, false);
  // B5c：三選一刪除後，開始翻譯不再轉接續補完（接續補完是 S14 的主要按鈕，直接開跑，R-8）
  const body = app.slice(app.indexOf("async function onRunInner("), app.indexOf("async function onRepair("));
  assert.ok(!body.includes("onSupplementInner("), "開始翻譯不再轉成接續補完");
});
