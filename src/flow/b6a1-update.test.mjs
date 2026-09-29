/**
 * B6a-1 模組整合包更新（規格 §2.2 S15 B6a-1 起、S16；§8.2、§8.3；已幫你做的事「拿掉的模組舊翻譯已清掉」）。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { SENTENCE_MAX, computePackState } from "./pack-state.js";
import { BANNER_STATES, folderBanners } from "./folder-state.js";
import { PACK_UPDATE_ACTION, packUpdateState, updateDoneLines, updateExtraLine, updateSentence } from "./pack-update.js";
import { doneLines, summarizeRun } from "./result-card.js";
import { planStatusCard, GENERIC_PRIMARY_ID } from "./status-card.js";
import { DISCLOSURES } from "./disclosure.js";
import { SETTING_PATHS } from "../core/settings-paths.js";
import { findForbidden } from "../copy/terms.js";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel) => readFileSync(join(here, rel), "utf8").replace(/\r\n/g, "\n");

const PATH = "C:/Games/ATM10";
const base = { consentAccepted: true, instancePath: PATH, validation: { ok: true, reason: "", hints: [] }, packName: "ATM10" };
const update = (over = {}) => ({
  modsChanged: true,
  countsKnown: true,
  newMods: 3,
  updatedMods: 2,
  sentences: 412,
  textsChanged: 5,
  mcBefore: "1.20.1",
  mcNow: "1.20.1",
  mcChanged: false,
  ...over,
});
const len = (s) => Array.from(String(s || "")).length;

test("S15（B6a-1 起）：「已更新：約 K 句要翻，其他照舊」＋附加「新增 N 個模組、M 處任務文字改了」＋主要「翻譯更新的部分」直接開跑", () => {
  const s = computePackState({ ...base, packChanged: true, packUpdate: update() });
  assert.equal(s.id, "S15");
  assert.equal(s.sentence, "「ATM10」已更新：約 412 句要翻，其他照舊");
  assert.ok(len(s.sentence) <= SENTENCE_MAX);
  assert.equal(s.extraLine, "新增 3 個模組、更新 2 個模組、5 處任務文字改了");
  assert.equal(s.disclosureKey, "packChanged");
  assert.deepEqual(s.primary, { action: PACK_UPDATE_ACTION.updatePart, label: "翻譯更新的部分" });
  assert.equal(s.primary.action, "supplement", "走接續補完（後端先重掃，只翻新增／改過的句子）");
  assert.equal(s.reTranslate, false, "R-8：不經過重新翻譯的開始前確認");
  assert.equal(s.aiRowOnly, true, "只帶 AI 列（同 S14）");
  assert.deepEqual(s.more.map((m) => m.action), ["delete-and-restart"]);
  const plan = planStatusCard(s);
  assert.equal(plan.buttons[GENERIC_PRIMARY_ID].hidden, false, "主要按鈕用共用的 #btn-card-action");
  assert.equal(plan.buttons[GENERIC_PRIMARY_ID].label, "翻譯更新的部分");
});

test("S15：算不出句數（舊工作階段）時不寫數字；只有任務文字改了時說幾處", () => {
  const unknown = packUpdateState({ packName: "ATM10", packUpdate: update({ countsKnown: false, sentences: 0, newMods: 0, updatedMods: 0, textsChanged: 0 }) });
  assert.equal(unknown.sentence, "「ATM10」已更新，只翻有變的部分，其他照舊");
  assert.ok(!unknown.sentence.includes("句要翻"), "不寫句數");
  assert.equal(unknown.extraLine, "翻過的句子會直接沿用，只翻有變的");
  const texts = packUpdateState({ packName: "ATM10", packUpdate: update({ modsChanged: false, countsKnown: false, sentences: 0, textsChanged: 7 }) });
  assert.equal(texts.sentence, "「ATM10」已更新：7 處任務文字要重翻");
  assert.equal(updateExtraLine({ textsChanged: 7 }), "7 處任務文字改了");
  assert.equal(updateSentence("A", { countsKnown: true, sentences: 12345 }), "「A」已更新：約 12,345 句要翻，其他照舊");
  assert.equal(packUpdateState({ packName: "A", packUpdate: update({ modsChanged: false, textsChanged: 0 }) }), null, "沒有更新證據不出 S15");
  assert.equal(packUpdateState({ packName: "A", packUpdate: null }), null);
});

test("S16：MC 版本變了（有記版本且不同）優先於 S15；主要「重新翻譯」進入 §3.1", () => {
  const s = computePackState({ ...base, packChanged: true, packUpdate: update({ mcChanged: true, mcNow: "1.21.1" }) });
  assert.equal(s.id, "S16");
  assert.equal(s.sentence, "Minecraft 版本變了（1.20.1→1.21.1），要重新翻譯");
  assert.ok(len(s.sentence) <= SENTENCE_MAX);
  assert.equal(s.extraLine, "翻譯記憶與共享庫會讓它比第一次快");
  assert.equal(s.disclosureKey, "mcChanged");
  assert.deepEqual(s.primary, { action: "run", label: "重新翻譯" });
  assert.equal(s.reTranslate, true, "重新翻譯刻意多一步進入開始前確認");
  assert.ok(s.showAiRow);
  const retired = computePackState({ ...base, packChanged: true, packUpdate: update({ mcChanged: true, mcNow: "1.21.1" }), extraShown: () => false });
  assert.equal(retired.extraLine, "");
  assert.equal(planStatusCard(retired).extra.canRecall, true);
  // 失效安全：舊工作階段沒記版本 → 後端 mcChanged=false、mcBefore=null → 不判 S16
  const old = computePackState({ ...base, packChanged: true, packUpdate: update({ mcChanged: false, mcBefore: null, mcNow: "1.21.1" }) });
  assert.equal(old.id, "S15");
});

test("沒有更新差異資料（舊探測格式）時退回 S15 暫行；重新翻完後不再說有變動", () => {
  const s = computePackState({ ...base, packChanged: true });
  assert.equal(s.id, "S15");
  assert.equal(s.primary.label, "重新翻譯");
  assert.equal(computePackState({ ...base, packChanged: true, packUpdate: update(), translationComplete: true }).id, "READY");
});

test("S15 與 S14 同時成立顯示 S15（按一次順便補完舊缺口）", () => {
  const partial = summarizeRun({ applyStatus: "applied", pendingCount: 30, coveragePercent: 90, interruption: {} }, { origin: "run", finishedNow: false });
  const s = computePackState({ ...base, packChanged: true, packUpdate: update(), result: partial, resultCtx: {} });
  assert.equal(s.id, "S15");
});

test("已幫你做的事：模組整合包拿掉的 N 個模組，舊翻譯已清掉", () => {
  assert.deepEqual(updateDoneLines({ removedMods: 2 }), ["模組整合包拿掉的 2 個模組，舊翻譯已清掉"]);
  assert.deepEqual(updateDoneLines({ removedMods: 0 }), []);
  assert.deepEqual(updateDoneLines(null), []);
  assert.deepEqual(updateDoneLines({ removedMods: 0, unconfirmedRemoved: 2 }), ["2 個模組這次讀不到或暫時停用，舊翻譯先保留"], "審查 F1：無法確認的照實說");
  const s = summarizeRun(
    { applyStatus: "applied", pendingCount: 0, coveragePercent: 100, interruption: {}, packUpdate: { rescanned: true, removedMods: 2, newSentences: 10, changedSentences: 1 } },
    { origin: "supplement", finishedNow: true },
  );
  assert.ok(doneLines(s).includes("模組整合包拿掉的 2 個模組，舊翻譯已清掉"), JSON.stringify(doneLines(s)));
  const none = summarizeRun({ applyStatus: "applied", pendingCount: 0, interruption: {} }, { origin: "run", finishedNow: true });
  assert.ok(!doneLines(none).some((l) => l.includes("拿掉")));
});

test("橫幅 N-03／N-04 也在 S16 出現（規格 §3.4：S13–S16）", () => {
  assert.ok(BANNER_STATES.includes("S16"));
  const { show } = folderBanners({ stateId: "S16", gameRunning: true, hasOptions: false });
  assert.deepEqual(show.map((b) => b.id), ["N-03", "N-04"]);
});

test("S16 附加說明有獨立 key，設定路徑在前後端共用白名單（G0.2）與 KEY_MAP", () => {
  const d = DISCLOSURES.mcChanged;
  assert.ok(d && d.topic);
  assert.ok(SETTING_PATHS.includes(d.settingPath));
  assert.ok(read("../core/settings-store.js").includes(`"${d.storageKey}": "${d.settingPath}"`));
});

test("接線：探測的更新差異傳進狀態；有變動時也帶入上次的結果位置（翻譯更新的部分走接續補完）", () => {
  assert.ok(read("./pack-actions.js").includes("packUpdate: s.packChangeProbe ? s.packChangeProbe.packUpdate || null : null"));
  const app = read("../app.js");
  const at = app.indexOf("if (packChangeProbe) {");
  assert.ok(at > 0);
  assert.ok(app.slice(at, at + 400).includes("setAutoOutputDir(probe.outputDir)"));
});

test("詞表：S15／S16／完成卡的新字串沒有禁用詞", () => {
  const texts = [
    computePackState({ ...base, packChanged: true, packUpdate: update() }),
    computePackState({ ...base, packChanged: true, packUpdate: update({ mcChanged: true, mcNow: "1.21.1" }) }),
    packUpdateState({ packName: "A", packUpdate: update({ countsKnown: false, modsChanged: false, textsChanged: 2 }) }),
  ].flatMap((s) => [s.sentence, s.extraLine, s.primary.label, ...s.more.map((m) => m.label)]);
  texts.push(...updateDoneLines({ removedMods: 3, unconfirmedRemoved: 1 }));
  assert.deepEqual(findForbidden(texts.join("\n")), []);
});
