/**
 * B5a-1 流程狀態（規格 §2）：一句現況＋唯一主要按鈕；主要按鈕 0 顆只准出現在例外清單。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import {
  ACTION,
  STATE,
  SENTENCE_MAX,
  ZERO_PRIMARY_ALLOWED,
  computePackState,
  folderAreaLock,
  packNameFromPath,
  removalDetailLines,
  removeTranslationControl,
  shortPackName,
} from "./pack-state.js";
import { PRIMARY_BUTTON_IDS, planStatusCard } from "./status-card.js";
import { DISCLOSURES, createDisclosure } from "./disclosure.js";
import { SETTING_PATHS } from "../core/settings-paths.js";

const here = dirname(fileURLToPath(import.meta.url));

const ok = { ok: true, reason: "實例可用。" };
const base = { consentAccepted: true, instancePath: "C:/Games/ATM10", validation: ok };

/** 各種輸入的組合，全部都要符合 R-1。 */
function allInputs() {
  const out = [];
  for (const consentAccepted of [false, true])
    for (const instancePath of ["", "C:/Games/ATM10"])
      for (const validation of [ok, { ok: false, reason: "找不到 mods 資料夾" }, {}])
        for (const busy of [false, true])
          for (const busyKind of ["", "translate", "apply", "font"])
            for (const hasResult of [false, true])
              for (const removal of [null, { instancePath, hasResult, result: { removed: 2 } }])
                for (const versionBlocked of [false, true])
                  out.push({ consentAccepted, instancePath, validation, busy, busyKind, hasResult, removal, versionBlocked });
  return out;
}

test("R-1：每個狀態的主要按鈕恰好 0 或 1 顆，0 顆只在例外清單", () => {
  const seen = new Set();
  for (const input of allInputs()) {
    const s = computePackState(input);
    seen.add(s.id);
    const count = s.primary ? 1 : 0;
    assert.ok(count === 1 || ZERO_PRIMARY_ALLOWED.includes(s.id), `${s.id} 沒有主要按鈕`);
    assert.ok(s.sentence, `${s.id} 沒有現況句`);
    assert.ok(Array.isArray(s.secondary) && s.secondary.length <= 3, `${s.id} 次要按鈕超過 3 顆`);
  }
  for (const id of ["S00", "S01", "S02", "S09", "S19a", "S19b", "READY", "BUSY"]) {
    assert.ok(seen.has(id), `組合裡沒有走到 ${id}`);
  }
});

test("R-4：主要按鈕停用時一定有就地原因", () => {
  for (const input of allInputs()) {
    const s = computePackState(input);
    if (s.primary && s.primary.disabled) assert.ok(s.disabledReason, `${s.id} 停用卻沒寫原因`);
  }
});

test("現況句、附加行、停用原因都在 40 字內（包名截到 16 字）", () => {
  const longName = "一個名字非常非常非常非常非常非常長的模組整合包";
  for (const input of allInputs()) {
    const s = computePackState({ ...input, packName: longName });
    for (const text of [s.sentence, s.extraLine, s.disabledReason]) {
      assert.ok(Array.from(text).length <= SENTENCE_MAX, `${s.id} 超過字數：${text}`);
    }
  }
  assert.equal(shortPackName(longName), Array.from(longName).slice(0, 16).join("") + "…");
  assert.equal(packNameFromPath("C:\\Users\\a\\ATM10\\"), "ATM10");
});

test("S00：沒同意過就是 S00，主要按鈕是同意頁的「我了解，開始使用」，狀態卡不重複畫", () => {
  const s = computePackState({ ...base, consentAccepted: false });
  assert.equal(s.id, STATE.consent);
  assert.equal(s.primary.action, ACTION.acceptConsent);
  assert.equal(s.primary.label, "我了解，開始使用");
  assert.equal(planStatusCard(s).hidden, true);
});

test("S01：沒選資料夾時一句話＋「選擇遊戲資料夾」；附加行只在第一次", () => {
  const first = computePackState({ consentAccepted: true });
  assert.equal(first.id, STATE.noFolder);
  assert.equal(first.primary.action, ACTION.pickFolder);
  assert.equal(first.primary.label, "選擇遊戲資料夾");
  assert.match(first.extraLine, /CurseForge/);
  assert.equal(first.showAiRow, false);
  const tenth = computePackState({ consentAccepted: true, pickFolderFresh: false });
  assert.equal(tenth.extraLine, "", "第十次不再顯示附加行");
  const plan = planStatusCard(tenth);
  assert.equal(plan.extra.canRecall, true, "退場後原位置留「？」");
});

test("暫行「可開始」：選好資料夾後一句現況＋「開始翻譯」（呼叫舊 onRun 的 #btn-run）", () => {
  const s = computePackState(base);
  assert.equal(s.id, STATE.ready);
  assert.match(s.sentence, /已選好「ATM10」/);
  assert.equal(s.primary.action, ACTION.run);
  assert.equal(s.primary.label, "開始翻譯");
  assert.equal(PRIMARY_BUTTON_IDS[s.primary.action], "btn-run");
  assert.equal(s.showAiRow, true);
  assert.deepEqual(s.more, [], "沒有結果時沒有「刪除結果並重翻」");
  const withResult = computePackState({ ...base, hasResult: true });
  assert.equal(withResult.more.length, 1);
  assert.equal(withResult.more[0].action, ACTION.deleteAndRestart, "刪除並重翻只在狀態卡「更多」");
});

test("S02（暫行）：資料夾沒通過或版本太舊時，擋下並給「重新選擇」", () => {
  const bad = computePackState({ ...base, validation: { ok: false, reason: "找不到 mods 資料夾" } });
  assert.equal(bad.id, STATE.folderNotRight);
  assert.equal(bad.tone, "block");
  assert.match(bad.sentence, /找不到 mods/);
  assert.equal(bad.primary.action, ACTION.pickFolder);
  const old = computePackState({ ...base, versionBlocked: true, versionBlockReason: "Minecraft 1.12 太舊" });
  assert.equal(old.id, STATE.folderNotRight);
  assert.match(old.sentence, /1\.12/);
});

test("S09：翻譯中狀態卡只有停止鈕，停止鈕文字交給 stop-button（G4.25）", () => {
  const s = computePackState({ ...base, busy: true, busyKind: "translate" });
  assert.equal(s.id, STATE.translating);
  assert.equal(s.primary.action, ACTION.stop);
  assert.equal(s.showAiRow, false, "翻譯中不顯示 AI 列");
  const plan = planStatusCard(s);
  assert.equal(plan.buttons["btn-stop"].hidden, false);
  assert.equal(plan.buttons["btn-stop"].label, null, "不覆寫停止鈕文字");
  assert.equal(plan.buttons["btn-run"].hidden, true);
});

test("其他工作進行中：開始翻譯停用並就地寫原因", () => {
  const s = computePackState({ ...base, busy: true, busyKind: "apply" });
  assert.equal(s.primary.disabled, true);
  assert.match(s.disabledReason, /套用/);
  const plan = planStatusCard(s);
  assert.equal(plan.buttons["btn-run"].ariaDisabled, true);
  assert.equal(plan.reason, s.disabledReason);
});

test("S19a／S19b：剛移除翻譯；有結果時「套用到遊戲」不顯示 AI 列，沒結果時「開始翻譯」顯示 AI 列", () => {
  const result = { removed: 3, restored: 2, unrestorable: ["a.jar"], quarantined: ["x — y"] };
  const a = computePackState({ ...base, removal: { instancePath: base.instancePath, hasResult: true, result } });
  assert.equal(a.id, STATE.removedWithResult);
  assert.equal(a.primary.action, ACTION.applyResult);
  assert.equal(a.primary.label, "套用到遊戲");
  assert.equal(a.showAiRow, false);
  assert.match(a.sentence, /已移除翻譯/);
  assert.match(a.detailLines[0], /刪了 3 個檔、放回 2 個原檔、1 個無法還原/);
  assert.match(a.detailLines[1], /隔離區/);
  const b = computePackState({ ...base, removal: { instancePath: base.instancePath, hasResult: false, result } });
  assert.equal(b.id, STATE.removedNoResult);
  assert.equal(b.primary.action, ACTION.run);
  assert.equal(b.showAiRow, true);
});

test("換資料夾整張換：別包的移除結果不會出現在這一包", () => {
  const s = computePackState({ ...base, removal: { instancePath: "C:/Games/Other", hasResult: true } });
  assert.equal(s.id, STATE.ready);
});

test("移除結果的條數兩種欄位形狀都認得", () => {
  assert.match(removalDetailLines({ removed_files: ["a", "b"], restored_files: ["c"] })[0], /刪了 2 個檔、放回 1 個原檔、0 個無法還原/);
  assert.match(removalDetailLines(null)[0], /刪了 0 個檔/);
});

test("S20：翻譯中 D 區整區停用，原因寫「正在翻「A」，翻完才能換資料夾」", () => {
  assert.deepEqual(folderAreaLock({ busy: false }), { locked: false, reason: "" });
  const lock = folderAreaLock({ busy: true, busyKind: "translate", instancePath: "C:/Games/ATM10" });
  assert.equal(lock.locked, true);
  assert.equal(lock.reason, "正在翻「ATM10」，翻完才能換資料夾");
});

test("移除翻譯只在 D 區：有翻譯紀錄才出現，翻譯中停用並寫原因", () => {
  assert.equal(removeTranslationControl({ instancePath: "C:/x", hasTranslationRecord: false }).visible, false);
  const idle = removeTranslationControl({ instancePath: "C:/x", hasTranslationRecord: true });
  assert.deepEqual(idle, { visible: true, disabledReason: "" });
  const busy = removeTranslationControl({ instancePath: "C:/x", hasTranslationRecord: true, busy: true, busyKind: "translate" });
  assert.equal(busy.visible, true);
  assert.match(busy.disabledReason, /翻完才能換資料夾/);
});

test("狀態卡同一時間只露出一顆主要按鈕", () => {
  for (const input of allInputs()) {
    const plan = planStatusCard(computePackState(input));
    const visible = Object.values(plan.buttons).filter((b) => !b.hidden).length;
    assert.ok(visible <= 1, `${plan.id} 露出 ${visible} 顆主要按鈕`);
    if (!plan.hidden) assert.equal(visible, 1, `${plan.id} 沒露出主要按鈕`);
  }
});

test("說明漸進退場：第一次顯示；動作成功或「不再顯示」後退場；「？」只叫回那一則", () => {
  const store = new Map();
  const d = createDisclosure({ read: (k) => store.get(k) ?? null, write: (k, v) => store.set(k, v) });
  assert.equal(d.isFresh("pickFolder"), true);
  d.retire("pickFolder");
  assert.equal(d.isFresh("pickFolder"), false);
  assert.equal(d.isShown("pickFolder"), false);
  d.recall("pickFolder");
  assert.equal(d.isShown("pickFolder"), true, "「？」叫回");
  assert.equal(d.isFresh("pickFolder"), false, "叫回不改退場紀錄");
  d.resetAll();
  assert.equal(d.isFresh("pickFolder"), true, "重看引導與說明＝全部重設");
  assert.equal(d.isFresh("unknown-key"), false);
  const broken = createDisclosure({ read: () => { throw new Error("x"); } });
  assert.equal(broken.isFresh("pickFolder"), true, "讀不到當第一次");
});

test("G0.2：說明退場的設定路徑在前後端共用白名單，且 KEY_MAP 對得上", () => {
  const store = readFileSync(join(here, "../core/settings-store.js"), "utf8");
  for (const { storageKey, settingPath } of Object.values(DISCLOSURES)) {
    assert.ok(SETTING_PATHS.includes(settingPath), `${settingPath} 不在白名單`);
    assert.ok(store.includes(`"${storageKey}": "${settingPath}"`), `KEY_MAP 沒有 ${storageKey}`);
  }
});

// ── 審查修正 中2：狀態卡不得與暫留的舊卡矛盾（B5b／B5c 取代前的保守分支）──
test("中2：待套用卡出現時，狀態句「已翻完，還沒裝進遊戲」且狀態卡不出主要按鈕（主要動作在該卡）", () => {
  const s = computePackState({ ...base, applyPendingShown: true, translationComplete: true });
  assert.equal(s.id, STATE.pendingCard);
  assert.equal(s.sentence, "已翻完，還沒裝進遊戲");
  assert.equal(s.primary, null);
  assert.ok(ZERO_PRIMARY_ALLOWED.includes(s.id));
  const plan = planStatusCard(s);
  assert.equal(Object.values(plan.buttons).filter((b) => !b.hidden).length, 0);
});

test("中2：接續卡出現時，狀態句「上次沒翻完」且不出主要按鈕", () => {
  const s = computePackState({ consentAccepted: true, resumeShown: true });
  assert.equal(s.id, STATE.resumeCard);
  assert.equal(s.sentence, "上次沒翻完");
  assert.equal(s.primary, null);
  assert.ok(ZERO_PRIMARY_ALLOWED.includes(s.id));
});

test("中2：翻譯已完成且沒有待套用時，狀態句「這個模組整合包已翻譯」，主要按鈕文字改「重新翻譯」（仍是 #btn-run）", () => {
  const s = computePackState({ ...base, translationComplete: true });
  assert.equal(s.sentence, "這個模組整合包已翻譯");
  assert.equal(s.primary.action, ACTION.run);
  assert.equal(s.primary.label, "重新翻譯");
  const fresh = computePackState(base);
  assert.equal(fresh.primary.label, "開始翻譯");
});

test("中2：翻譯中與同意頁優先於舊卡", () => {
  assert.equal(computePackState({ ...base, applyPendingShown: true, busy: true, busyKind: "translate" }).id, STATE.translating);
  assert.equal(computePackState({ ...base, consentAccepted: false, resumeShown: true }).id, STATE.consent);
});

// ── 審查修正 中1：翻譯中切到其他分頁仍看得到翻譯中與停止鈕（翻譯分頁可見時不重複）──
test("中1：翻譯中且不在翻譯分頁時，共用區一行「正在翻譯「<包名>」」；在翻譯分頁時不顯示", async () => {
  const { runElsewhereLine } = await import("./pack-state.js");
  const running = computePackState({ ...base, busy: true, busyKind: "translate" });
  assert.deepEqual(runElsewhereLine({ page: "font", state: running, packName: "ATM10" }), {
    shown: true,
    sentence: "正在翻譯「ATM10」",
  });
  assert.equal(runElsewhereLine({ page: "translate", state: running, packName: "ATM10" }).shown, false, "一件事只在一處說");
  assert.equal(runElsewhereLine({ page: "font", state: computePackState(base) }).shown, false);
});

test("中1：共用區在分頁外，停止鈕仍是同一顆 #btn-stop（文字由 stop-button 管）；翻譯分頁鈕有「翻譯中」標記", () => {
  const html = readFileSync(join(here, "../index.html"), "utf8");
  const shared = html.indexOf('id="run-elsewhere"');
  assert.ok(shared > 0, "缺共用區 #run-elsewhere");
  assert.ok(shared < html.indexOf('id="page-translate"'), "共用區要在分頁外");
  assert.ok(html.includes('id="tab-translate-badge"'));
  assert.equal(html.split('id="btn-stop"').length - 1, 1, "停止鈕只有一顆");
  const actions = readFileSync(join(here, "./pack-actions.js"), "utf8");
  assert.ok(actions.includes("runElsewhereLine("));
});
