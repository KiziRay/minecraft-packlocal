/**
 * B5b 狀態整合：S13 開始前確認、重新翻譯多一步、AI-BLOCKED、S12、S09／S10；run-flow 不開對話框。
 */
import test from "node:test";
import assert from "node:assert/strict";

import { STATE, ZERO_PRIMARY_ALLOWED, computePackState } from "./pack-state.js";
import { PRIMARY_BUTTON_IDS, GENERIC_PRIMARY_ID, planStatusCard } from "./status-card.js";
import { createRunFlow } from "./run-flow.js";
import { createAiGate } from "./run-start.js";
import { DISCLOSURES } from "./disclosure.js";
import { SETTING_PATHS } from "../core/settings-paths.js";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { classifyFailure } from "./run-failure.js";

const here = dirname(fileURLToPath(import.meta.url));
const store = readFileSync(join(here, "../core/settings-store.js"), "utf8");
const keyMapHas = (k, v) => store.includes(`"${k}": "${v}"`);

const ok = { ok: true, reason: "可用。" };
const base = { consentAccepted: true, instancePath: "C:/Games/ATM10", validation: ok };
const discordOk = { discordReady: true, loggedIn: true, inGuild: true, serviceAvailable: true };

function flow(over = {}) {
  const state = {
    status: { aiMode: "local", ...discordOk, localInstalled: true },
    useAi: true,
    backupChoice: "always",
    cloud: null,
    saved: [],
    toasts: [],
    dialogs: 0,
    syncs: 0,
    path: "C:/Games/ATM10",
    fixes: [],
    ...over,
  };
  const gate = createAiGate({
    useAi: () => state.useAi,
    refreshAiStatus: async () => state.status,
    startLocal: async () => state.startLocal !== false,
    gptUsable: async () => true,
  });
  const f = createRunFlow({
    instancePath: () => state.path,
    aiStatus: () => state.status,
    useAi: () => state.useAi,
    backupChoice: () => state.backupChoice,
    cloudInfo: () => state.cloud,
    gate,
    saveBackupChoice: async (v) => {
      state.saved.push(["backup", v]);
      state.backupChoice = v;
    },
    saveCloudChoice: async (v) => state.saved.push(["cloud", v]),
    toast: (t) => state.toasts.push(t),
    sync: () => {
      state.syncs += 1;
    },
    aiFix: (a) => state.fixes.push(a),
  });
  const card = () => computePackState({ ...base, prestart: f.prestartInput(), failure: f.failureInput() });
  return { f, state, card };
}

test("S13：還沒翻過時開始翻譯就在狀態卡上（§3.1 列），AI 就緒且選過備份時沒有任何紅列", () => {
  const { card } = flow();
  const s = card();
  assert.equal(s.id, STATE.ready);
  assert.equal(s.primary.label, "開始翻譯");
  assert.equal(s.primary.disabled, undefined);
  assert.equal(s.rows[0].id, "ai");
  assert.equal(s.rows[0].text, "用誰翻：本地模型（就緒）");
  const plan = planStatusCard(s);
  assert.equal(plan.rows.length, 1);
});

test("S13：沒登入 Discord 時 AI 列變紅、只有一顆登入按鈕，開始翻譯停用並寫原因", () => {
  const { card } = flow({ status: { aiMode: "local", discordReady: false, loggedIn: false, serviceAvailable: true } });
  const s = card();
  assert.equal(s.rows[0].tone, "block");
  assert.equal(s.rows[0].fix.label, "登入 Discord");
  assert.equal(s.primary.disabled, true);
  assert.equal(s.disabledReason, "先處理標紅的那一列");
});

test("第一次按開始翻譯：備份在狀態卡上選，按下後記住並 toast；之後不再出現備份列（D-04 不會出現）", async () => {
  const { f, state, card } = flow({ backupChoice: "" });
  assert.ok(card().rows.some((r) => r.id === "backup"));
  const out = await f.beforeStart("run");
  assert.equal(out.ok, true);
  assert.deepEqual(state.saved, [], "閘門通過還不記（三選一之後才記，審查 1d）");
  await f.commitRunChoices();
  assert.deepEqual(state.saved, [["backup", "always"]]);
  assert.equal(state.toasts[0], "已記住：備份，可到 設定→資料與備份 改");
  assert.equal(card().rows.some((r) => r.id === "backup"), false);
});

test("選不備份沒勾「我了解」時按不下去；勾了才可開始並記住不備份", async () => {
  const { f, state, card } = flow({ backupChoice: "" });
  f.onRowChange("backup", "never", false);
  assert.equal(card().primary.disabled, true);
  assert.equal((await f.beforeStart("run")).ok, false);
  assert.deepEqual(state.saved, []);
  f.onRowChange("backup", "never", true);
  assert.equal(card().primary.disabled, undefined);
  assert.equal((await f.beforeStart("run")).ok, true);
  await f.commitRunChoices();
  assert.deepEqual(state.saved, [["backup", "never"]]);
});

test("重新翻譯刻意多一步：已翻譯／S15／S18 按「重新翻譯」先進確認模式（開始重新翻譯＋返回），再按才開跑", () => {
  const { f } = flow();
  for (const input of [
    { ...base, translationComplete: true },
    { ...base, packChanged: true },
    { ...base, hasTranslationRecord: true },
  ]) {
    const before = computePackState({ ...input, prestart: f.prestartInput() });
    assert.equal(before.reTranslate, true, JSON.stringify(input));
    assert.equal(f.interceptRun(before), true);
    const open = computePackState({ ...input, prestart: f.prestartInput() });
    assert.equal(open.id, STATE.prestart);
    assert.equal(open.primary.label, "開始重新翻譯");
    assert.equal(f.interceptRun(open), false, "確認模式裡再按就開跑");
    f.onAction("prestart-back");
    assert.notEqual(computePackState({ ...input, prestart: f.prestartInput() }).id, STATE.prestart);
  }
  // 還沒翻過的 S13 不多一步
  const fresh = computePackState({ ...base, prestart: f.prestartInput() });
  assert.equal(f.interceptRun(fresh), false);
});

test("從卡片按接續補完、AI 還沒就緒：狀態卡顯示紅色 AI 列、接續補完停用；登入後回到原狀態（修復同理）", async () => {
  for (const origin of ["supplement", "repair"]) {
    const { f, state, card } = flow({ status: { aiMode: "custom", ...discordOk, providerReady: false } });
    const out = await f.beforeStart(origin);
    assert.equal(out.ok, false);
    const s = card();
    assert.equal(s.id, STATE.aiBlocked);
    assert.equal(s.primary.action, origin);
    assert.equal(s.primary.disabled, true);
    assert.equal(s.rows[0].text, "還沒填 API 金鑰");
    assert.equal(PRIMARY_BUTTON_IDS[s.primary.action] || GENERIC_PRIMARY_ID, GENERIC_PRIMARY_ID);
    state.status = { aiMode: "custom", ...discordOk, providerReady: true };
    assert.notEqual(card().id, STATE.aiBlocked);
  }
});

test("「這次不用 AI」：紅列變成可開始、這一輪不用 AI、不記住", async () => {
  const { f, state, card } = flow({ status: { aiMode: "gpt", ...discordOk, providerReady: false } });
  f.onAction("ai-skip-once");
  assert.equal(card().primary.disabled, undefined);
  const out = await f.beforeStart("run");
  assert.equal(out.ok, true);
  assert.equal(out.useAi, false);
  // 同一次點擊在三選一改成接續補完：明確帶入，同樣不用 AI（審查 1b）
  const carried = await f.beforeStart("supplement", { skipAi: !out.useAi });
  assert.equal(carried.ok, true);
  assert.equal(carried.useAi, false);
  await f.commitRunChoices();
  assert.deepEqual(state.saved, []);
  // 開跑後恢復（下一次照 AI 設定）
  assert.equal(card().rows[0].tone, "block");
  assert.equal((await f.beforeStart("supplement")).ok, false);
});

test("本地模型已安裝時閘門不啟動模型（啟動在輪次開始之後，啟動失敗走 S12 本地模型啟動不起來）", async () => {
  const { f } = flow();
  const out = await f.beforeStart("run");
  assert.equal(out.ok, true);
  f.recordFailure("run", "本地模型啟動不起來（詳見紀錄），可以改用其他 AI 或這次不用 AI");
  const s = computePackState({ ...base, failure: f.failureInput() });
  assert.equal(s.sentence, "翻譯沒完成：本地模型啟動不起來");
  assert.equal(s.primary.label, "換 AI 再試");
});

test("S12：開跑後失敗依原因給主要按鈕；換資料夾整張換；再開跑清掉", async () => {
  const { f, state, card } = flow();
  f.recordFailure("run", "自訂 API 金鑰無效（401）");
  let s = card();
  assert.equal(s.id, STATE.failed);
  assert.equal(s.primary.label, "換 AI 再試");
  f.recordFailure("repair", "找不到工作階段");
  assert.equal(card().primary.label, "修復翻譯檔");
  state.path = "C:/Games/Other";
  assert.notEqual(card().id, STATE.failed);
  state.path = "C:/Games/ATM10";
  assert.equal((await f.beforeStart("run")).ok, true);
  assert.notEqual(card().id, STATE.failed);
});

test("S09／S10：翻譯中句子來自進度畫面，停止中只有停止鈕本身", () => {
  const { f } = flow();
  f.beginRun({ localMode: false });
  f.onProgress({ percent: 40, stepIndex: 2, payload: { done: 40, total: 100, unit: "條" }, message: "AI 翻譯：第 3/9 批" });
  const s = computePackState({ ...base, busy: true, busyKind: "translate", progress: f.progressInput("ATM10") });
  assert.equal(s.id, STATE.translating);
  assert.equal(s.sentence, "正在翻「ATM10」：第 3／5 段");
  assert.equal(planStatusCard(s).progress.count, "已完成 40／100 條");
  f.markStopping();
  const stop = computePackState({ ...base, busy: true, busyKind: "translate", progress: f.progressInput("ATM10") });
  assert.equal(stop.id, STATE.stopping);
  assert.equal(stop.primary.action, "stop");
  assert.equal(stop.sentence, "正在停止，寫出已翻好的部分…");
  f.endRun();
  assert.equal(f.progressInput("ATM10"), null);
});

test("N-07：記憶體不太夠只出一次橫幅，本輪結束收掉", () => {
  const shown = [];
  const hidden = [];
  const f = createRunFlow({
    instancePath: () => "C:/A",
    aiStatus: () => null,
    useAi: () => false,
    backupChoice: () => "always",
    gate: { ensure: async () => ({ ready: true, useAi: false }) },
    saveBackupChoice: async () => {},
    saveCloudChoice: async () => {},
    showBanner: (b) => shown.push(b.id),
    hideBanner: (id) => hidden.push(id),
  });
  f.beginRun({ localMode: true });
  f.noteMessage("注意：記憶體仍不夠放下一批需要的量");
  f.noteMessage("注意：記憶體仍不夠放下一批需要的量");
  f.endRun();
  assert.deepEqual(shown, ["N-07"]);
  assert.deepEqual(hidden, ["N-07"]);
});

test("R-1：B5b 新狀態主要按鈕恰好一顆（S10 是停止鈕本身）", () => {
  const { f } = flow({ status: { aiMode: "custom", ...discordOk, providerReady: false } });
  const states = [
    computePackState({ ...base, busy: true, busyKind: "translate", stopping: true }),
    computePackState({ ...base, failure: { instancePath: base.instancePath, classified: classifyFailure("x") } }),
    computePackState({ ...base, prestart: { rows: [], open: true } }),
    computePackState({ ...base, prestart: { ...f.prestartInput(), aiBlockOrigin: "supplement" } }),
  ];
  for (const s of states) {
    assert.ok(s.primary, s.id);
    assert.ok(!ZERO_PRIMARY_ALLOWED.includes(s.id) || s.id === "S10");
    if (s.primary.disabled) assert.ok(s.disabledReason, s.id);
  }
});

test("G0.1／G0.2：B5b 新設定路徑（translate.useAi、四則說明）在共用白名單與 KEY_MAP；#backup-before-apply 的舊鍵已移出 KEY_MAP", () => {
  for (const key of ["prestart", "aiChoice", "runTips", "localStart"]) {
    const d = DISCLOSURES[key];
    assert.ok(d, key);
    assert.ok(SETTING_PATHS.includes(d.settingPath), d.settingPath);
    assert.ok(keyMapHas(d.storageKey, d.settingPath));
  }
  assert.ok(SETTING_PATHS.includes("translate.useAi"));
  assert.ok(keyMapHas("mcpl-use-ai", "translate.useAi"));
  assert.equal(keyMapHas("modpack-i18n-backup-before-apply", "translate.backupBeforeApply"), false);
});
