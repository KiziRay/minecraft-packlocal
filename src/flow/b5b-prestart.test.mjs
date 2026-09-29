/**
 * B5b 開始前確認（規格 §3.1）、AI 列 E1、AI 閘門（三個入口）、S12。
 */
import test from "node:test";
import assert from "node:assert/strict";

import { AI_FIT, AI_FIX, aiReadiness } from "./ai-readiness.js";
import {
  BLOCKED_REASON,
  PRESTART_ACTION,
  aiBlockedState,
  aiChoiceLines,
  applyPrestart,
  prestartBlocked,
  prestartCommits,
  prestartRows,
  reTranslatePrestart,
} from "./prestart.js";
import { commitPrestartChoices, createAiGate } from "./run-start.js";
import { classifyFailure, failureState } from "./run-failure.js";

const discordOk = { discordReady: true, loggedIn: true, inGuild: true, serviceAvailable: true };

test("E1：依實際缺的第一項阻擋——沒登入 Discord 先說 Discord（選本地模型也一樣），不說金鑰或資料夾被移動", () => {
  for (const aiMode of ["local", "custom", "gpt"]) {
    const r = aiReadiness({ status: { aiMode, discordReady: false, loggedIn: false, serviceAvailable: true } });
    assert.equal(r.ready, false);
    assert.equal(r.missing, "discord");
    assert.equal(r.sentence, "用 AI 翻要先登入 Discord 並加入官方伺服器");
    assert.deepEqual(r.fix, { action: AI_FIX.discordLogin, label: "登入 Discord" });
    assert.doesNotMatch(r.sentence, /金鑰|移動|刪除/);
  }
  const joined = aiReadiness({ status: { aiMode: "local", discordReady: false, loggedIn: true, serviceAvailable: true } });
  assert.equal(joined.fix.action, AI_FIX.discordJoin);
});

test("E1：本地模型還沒下載寫「還沒下載（約 X GB）」，不是「被移動或刪除」；大小未知不捏造數字", () => {
  const known = aiReadiness({ status: { aiMode: "local", ...discordOk, localInstalled: false }, localNeedBytes: 5.2 * 1024 ** 3 });
  assert.equal(known.sentence, "本地模型還沒下載（約 5.2 GB）");
  assert.deepEqual(known.fix, { action: AI_FIX.localDownload, label: "下載本地模型" });
  const unknown = aiReadiness({ status: { aiMode: "local", ...discordOk, localInstalled: false } });
  assert.equal(unknown.sentence, "本地模型還沒下載（要下載數 GB）");
  assert.doesNotMatch(unknown.sentence, /移動|刪除/);
  assert.equal(aiReadiness({ status: { aiMode: "local", ...discordOk, localInstalled: true } }).ready, true);
});

test("E1：自訂 API 沒存金鑰→「還沒填 API 金鑰」［填入金鑰］；ChatGPT 沒登入→［登入 ChatGPT］；付費來源同列提示額度", () => {
  const c = aiReadiness({ status: { aiMode: "custom", ...discordOk, providerReady: false }, provider: "deepseek" });
  assert.equal(c.sentence, "還沒填 API 金鑰");
  assert.equal(c.fix.label, "填入金鑰");
  assert.equal(c.paidNote, "會用到你的 DeepSeek 額度");
  const g = aiReadiness({ status: { aiMode: "gpt", ...discordOk, providerReady: false } });
  assert.equal(g.sentence, "ChatGPT 還沒登入或已過期");
  assert.equal(g.paidNote, "會用到你的 ChatGPT 額度");
  const ok = aiReadiness({ status: { aiMode: "custom", ...discordOk, providerReady: true } });
  assert.equal(ok.ready, true);
  assert.equal(ok.summary, "用誰翻：自訂 API（就緒）");
  assert.equal(aiReadiness({ status: { aiMode: "local", ...discordOk, localInstalled: true } }).paidNote, "");
});

test("E1：AI 狀態讀不到當成需要處理（重新檢查），不當成就緒；不使用 AI 永遠就緒", () => {
  const r = aiReadiness({ status: null, mode: "local" });
  assert.equal(r.ready, false);
  assert.equal(r.fix.action, AI_FIX.recheck);
  assert.equal(aiReadiness({ useAi: false }).ready, true);
  assert.equal(aiReadiness({ useAi: false }).summary, "用誰翻：不使用 AI");
});

test("E1：四個選項各有「適合誰」，選中者附準備清單；都在 40 字內", () => {
  const lines = aiChoiceLines("custom");
  assert.equal(lines.length, 4);
  for (const l of lines) {
    assert.ok(l.fit && Array.from(l.fit).length <= 40, l.fit);
    assert.equal(!!l.prep, l.mode === "custom");
  }
  assert.ok(AI_FIT.local.includes("好幾小時"));
});

test("§3.1：紅色 AI 列附次要「這次不用 AI」；有紅列時開始翻譯停用並寫「先處理標紅的那一列」", () => {
  const ai = aiReadiness({ status: { aiMode: "local", discordReady: false, serviceAvailable: true } });
  const rows = prestartRows({ ai, backupChoice: "always" });
  assert.equal(rows[0].tone, "block");
  assert.ok(rows[0].secondary.some((b) => b.action === PRESTART_ACTION.skipAi && b.label === "這次不用 AI"));
  assert.equal(prestartBlocked(rows), true);
  const state = applyPrestart({ id: "READY", primary: { action: "run", label: "開始翻譯" }, disabledReason: "" }, rows);
  assert.equal(state.primary.disabled, true);
  assert.equal(state.disabledReason, BLOCKED_REASON);
  // 按「這次不用 AI」後就能開始，且這個選擇不記住
  const skipped = prestartRows({ ai, backupChoice: "always", skipAiOnce: true });
  assert.equal(prestartBlocked(skipped), false);
  assert.deepEqual(prestartCommits(skipped), []);
});

test("§3.1 本地翻不好時：只在本地模型＋已設好線上 AI＋從沒選過時出現，預設只用本地；按開始時記住並 toast（G0.6）", () => {
  const ai = aiReadiness({ status: { aiMode: "local", ...discordOk, localInstalled: true } });
  const shown = prestartRows({ ai, backupChoice: "always", cloud: { needsConsent: true, configured: true } });
  const row = shown.find((r) => r.id === "cloud");
  assert.ok(row);
  assert.equal(row.value, "0", "預設只用本地（不花錢）");
  assert.deepEqual(prestartCommits(shown), [{ kind: "cloud", value: "0", toast: "已記住：只用本地，可到 設定→翻譯與 AI 改" }]);
  assert.equal(prestartRows({ ai, backupChoice: "always", cloud: { needsConsent: true, configured: false } }).some((r) => r.id === "cloud"), false);
  assert.equal(prestartRows({ ai, backupChoice: "always", cloud: { needsConsent: false, configured: true } }).some((r) => r.id === "cloud"), false);
  const custom = aiReadiness({ status: { aiMode: "custom", ...discordOk, providerReady: true } });
  assert.equal(prestartRows({ ai: custom, backupChoice: "always", cloud: { needsConsent: true, configured: true } }).some((r) => r.id === "cloud"), false);
});

test("§3.1 備份列：全工具第一次才出現；預設備份（建議）；選不備份要同列勾「我了解」才可開始；按開始時記住", () => {
  const ai = aiReadiness({ useAi: false });
  assert.equal(prestartRows({ ai, backupChoice: "never" }).some((r) => r.id === "backup"), false);
  const first = prestartRows({ ai, backupChoice: "" });
  const row = first.find((r) => r.id === "backup");
  assert.equal(row.value, "always");
  assert.equal(prestartBlocked(first), false);
  assert.deepEqual(prestartCommits(first), [{ kind: "backup", value: "always", toast: "已記住：備份，可到 設定→資料與備份 改" }]);
  const never = prestartRows({ ai, backupChoice: "", backupDraft: { value: "never" } });
  assert.equal(prestartBlocked(never), true);
  assert.equal(never.find((r) => r.id === "backup").ack.label, "我了解之後無法還原被覆蓋的檔案");
  const acked = prestartRows({ ai, backupChoice: "", backupDraft: { value: "never", ack: true } });
  assert.equal(prestartBlocked(acked), false);
  assert.equal(prestartCommits(acked)[0].value, "never");
  for (const c of prestartCommits(acked)) assert.ok(Array.from(c.toast).length <= 24, c.toast);
});

test("重新翻譯刻意多一步：確認模式主要「開始重新翻譯」、次要「返回」（R-8）", () => {
  const s = reTranslatePrestart("ATM10");
  assert.equal(s.primary.label, "開始重新翻譯");
  assert.equal(s.primary.action, "run");
  assert.deepEqual(s.secondary, [{ action: PRESTART_ACTION.back, label: "返回" }]);
  assert.equal(s.showAiRow, true);
});

test("接續補完／修復從狀態卡按、AI 還沒就緒：狀態卡顯示紅色 AI 列並停用同一顆主要按鈕（不跳視窗）", () => {
  const ai = aiReadiness({ status: { aiMode: "custom", ...discordOk, providerReady: false } });
  for (const origin of ["supplement", "repair"]) {
    const rows = prestartRows({ ai, backupChoice: "always" });
    const s = applyPrestart(aiBlockedState(origin), rows);
    assert.equal(s.primary.action, origin);
    assert.equal(s.primary.disabled, true);
    assert.equal(s.disabledReason, BLOCKED_REASON);
    assert.equal(s.showAiRow, true);
    assert.equal(s.rows[0].text, "還沒填 API 金鑰");
  }
});

function gateDeps(over = {}) {
  const calls = { dialogs: 0, overlays: 0, startLocal: 0 };
  const deps = {
    useAi: () => true,
    skipAiOnce: () => false,
    refreshAiStatus: async () => ({ aiMode: "local", ...discordOk, localInstalled: true }),
    startLocal: async () => {
      calls.startLocal += 1;
      return true;
    },
    gptUsable: async () => true,
    // 閘門不得開任何對話框或浮層：有人呼叫就記下來
    confirmDialog: () => {
      calls.dialogs += 1;
    },
    openLocalLlmOverlay: () => {
      calls.overlays += 1;
    },
    ...over,
  };
  return { deps, calls };
}

test("AI 閘門：翻譯、修復、接續補完三個入口共用；缺哪一項就回哪一項，不開對話框也不開浮層", async () => {
  const cases = [
    [{ aiMode: "local", discordReady: false, serviceAvailable: true }, "discord"],
    [{ aiMode: "local", ...discordOk, localInstalled: false }, "local-model"],
    [{ aiMode: "custom", ...discordOk, providerReady: false }, "custom-key"],
    [{ aiMode: "gpt", ...discordOk, providerReady: false }, "gpt"],
    [null, "status"],
  ];
  for (const [status, missing] of cases) {
    const { deps, calls } = gateDeps({ refreshAiStatus: async () => status });
    const gate = createAiGate(deps);
    for (const entry of ["run", "repair", "supplement"]) {
      const r = await gate.ensure(entry);
      assert.equal(r.ready, false, `${entry} ${missing}`);
      assert.equal(r.missing, missing);
    }
    assert.equal(calls.dialogs + calls.overlays, 0);
    assert.equal(calls.startLocal, 0, "還沒就緒時不去啟動本地模型");
  }
});

test("AI 閘門：本地模型已安裝就算就緒但不啟動（審查 1c，G0.5：啟動在輪次開始之後）；ChatGPT 試不過回登入", async () => {
  const ok = gateDeps();
  assert.equal((await createAiGate(ok.deps).ensure()).ready, true);
  assert.equal(ok.calls.startLocal, 0);
  const gpt = gateDeps({ refreshAiStatus: async () => ({ aiMode: "gpt", ...discordOk, providerReady: true }), gptUsable: async () => false });
  assert.equal((await createAiGate(gpt.deps).ensure()).missing, "gpt");
  const none = gateDeps({ useAi: () => false });
  const n = await createAiGate(none.deps).ensure();
  assert.equal(n.ready, true);
  assert.equal(n.useAi, false);
  const skip = gateDeps({ skipAiOnce: () => true, refreshAiStatus: async () => null });
  assert.equal((await createAiGate(skip.deps).ensure()).useAi, false);
});

test("按下開始時落盤：備份與本地翻不好時各記一次並 toast；寫入失敗只記紀錄、不擋翻譯", async () => {
  const ai = aiReadiness({ status: { aiMode: "local", ...discordOk, localInstalled: true } });
  const rows = prestartRows({ ai, backupChoice: "", cloud: { needsConsent: true, configured: true }, cloudDraft: "1" });
  const saved = [];
  const toasts = [];
  const done = await commitPrestartChoices(rows, {
    saveBackupChoice: async (v) => saved.push(["backup", v]),
    saveCloudChoice: async (v) => saved.push(["cloud", v]),
    toast: (t) => toasts.push(t),
  });
  assert.deepEqual(done, ["cloud", "backup"]);
  assert.deepEqual(saved, [["cloud", "1"], ["backup", "always"]]);
  assert.equal(toasts.length, 2);
  const logs = [];
  const partial = await commitPrestartChoices(rows, {
    saveBackupChoice: async () => {
      throw new Error("disk");
    },
    saveCloudChoice: async () => {},
    log: (m) => logs.push(m),
  });
  assert.deepEqual(partial, ["cloud"]);
  assert.equal(logs.length, 1);
});

test("S12：依原因給主要按鈕（暫用字串判斷）——AI→換 AI 再試、翻譯檔可修→修復翻譯檔、其他→再試一次；次要問題回報", () => {
  assert.equal(classifyFailure("login_required").primary.label, "換 AI 再試");
  assert.equal(classifyFailure("自訂 API 額度或金鑰無法使用：HTTP 402").kind, "quota");
  assert.equal(classifyFailure("本地模型一直等不到回應").primary.action, "ai-change");
  assert.equal(classifyFailure("找不到工作階段，資源包 zip 不完整").primary.label, "修復翻譯檔");
  const other = classifyFailure("奇怪的錯誤", "supplement");
  assert.deepEqual(other.primary, { action: "supplement", label: "再試一次" });
  const s = failureState(classifyFailure("error sending request"));
  assert.equal(s.id, "S12");
  assert.equal(s.sentence, "翻譯沒完成：連不上服務，檢查網路");
  assert.deepEqual(s.secondary, [{ action: "issue-report", label: "問題回報" }]);
  assert.equal(s.showAiRow, false);
  assert.equal(failureState(classifyFailure("金鑰無效")).showAiRow, true);
  for (const t of ["login_required", "402", "金鑰", "本地模型", "ChatGPT", "工作階段", "磁碟空間", "逾時", "x"]) {
    assert.ok(Array.from(failureState(classifyFailure(t)).sentence).length <= 40);
  }
});
