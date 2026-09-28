// B5a-2 #1：設定視窗的對話框（D-04 同款、D-07、D-12、D-14）、備份目前值、翻譯中停用、線上 AI 說明。
import test from "node:test";
import assert from "node:assert/strict";

import {
  BACKUP_CHOICES,
  ROW_COPY,
  SETTINGS_DIALOGS,
  backupChoiceValue,
  backupCurrentLine,
  dataActionLocks,
  deleteBackupsDialog,
  deleteModelDialog,
  deleteResultsStep,
  describeOnlineAi,
  formatGb,
  migrateResultLine,
  textLength,
} from "./settings-copy.js";

const allDialogs = () => [
  SETTINGS_DIALOGS.clearKey,
  SETTINGS_DIALOGS.cloudTopUp,
  SETTINGS_DIALOGS.noBackup,
  deleteModelDialog({ sizeBytes: 5.2 * 1024 ** 3, dir: "D:/MCPL/local-llm" }),
  deleteBackupsDialog({ packName: "All the Mods 10 超長名字測試用", location: "C:/x/apply-backups/abc/originals" }),
];

test("對話框：標題是問句且 ≤18 字、內文 ≤80 字 ≤3 句、按鈕 2–8 字（規格 §5.2）", () => {
  for (const d of allDialogs()) {
    assert.ok(d.title.endsWith("？"), `標題要是問句：${d.title}`);
    assert.ok(textLength(d.title) <= 18, `標題太長：${d.title}`);
    assert.ok(textLength(d.body) <= 80, `內文太長：${d.body}`);
    const sentences = d.body.split(/[。？]/).filter((s) => s.trim());
    assert.ok(sentences.length <= 3, `內文超過 3 句：${d.body}`);
    for (const label of [d.confirmLabel, d.cancelLabel]) {
      assert.ok(textLength(label) >= 2 && textLength(label) <= 8, `按鈕字數：${label}`);
    }
  }
});

test("危險（刪除／無備份覆蓋）預設焦點在取消；D-14 花錢也預設取消（規格 §3.5）", () => {
  for (const d of [SETTINGS_DIALOGS.clearKey, SETTINGS_DIALOGS.noBackup, deleteModelDialog({}), deleteBackupsDialog({})]) {
    assert.equal(d.danger, true, d.title);
  }
  assert.equal(SETTINGS_DIALOGS.cloudTopUp.danger, false, "花錢不是刪除，不用紅色");
  assert.equal(SETTINGS_DIALOGS.cloudTopUp.initialFocus, "cancel");
  assert.match(SETTINGS_DIALOGS.cloudTopUp.body, /額度/, "D-14 要講會用到額度（G0.6）");
  assert.ok(SETTINGS_DIALOGS.noBackup.ackLabel, "不備份要同框勾選（D-04 同款）");
  assert.ok(deleteBackupsDialog({}).ackLabel, "D-07 危險＋勾選");
});

test("D-07 列實際備份位置、不寫「所有備份」；D-12 寫可釋放多少空間", () => {
  const d07 = deleteBackupsDialog({ packName: "ATM10", location: "C:/data/apply-backups/k/originals" });
  assert.equal(d07.title, "刪除這個遊戲資料夾的全部備份？");
  assert.deepEqual(d07.affected, ["C:/data/apply-backups/k/originals"]);
  assert.doesNotMatch(d07.body, /所有備份/);
  assert.deepEqual(deleteBackupsDialog({ location: "" }).affected, []);
  const d12 = deleteModelDialog({ sizeBytes: 5.2 * 1024 ** 3, dir: "D:/m" });
  assert.match(d12.body, /5\.2 GB/);
  assert.deepEqual(d12.affected, ["D:/m"]);
  assert.match(deleteModelDialog({}).body, /重新下載/);
});

test("備份：三選一、顯示「目前：<值>」（全部模組整合包共用）", () => {
  assert.deepEqual(BACKUP_CHOICES.map((c) => c.value), ["ask", "always", "never"]);
  assert.equal(backupChoiceValue({ translate: { backupChoice: "always" } }), "always");
  assert.equal(backupChoiceValue({ translate: { backupChoice: "never" } }), "never");
  assert.equal(backupChoiceValue({ translate: { backupChoice: "壞值" } }), "ask");
  assert.equal(backupChoiceValue(null), "ask");
  assert.equal(backupCurrentLine("always"), "目前：先備份（全部模組整合包共用）");
  assert.equal(backupCurrentLine("ask"), "目前：每次詢問（全部模組整合包共用）");
});

test("翻譯中：刪除全部備份、搬到工具旁邊、刪除本地模型都停用並寫原因", () => {
  const busy = dataActionLocks({ busy: true, instancePath: "D:/pack", packName: "ATM10" }, { modelInstalled: true });
  for (const key of ["deleteBackups", "migrate", "deleteModel"]) {
    assert.equal(busy[key].locked, true, key);
    assert.match(busy[key].reason, /翻譯/, key);
    assert.ok(textLength(busy[key].reason) <= 40);
  }
  const idle = dataActionLocks({ busy: false, instancePath: "D:/pack", packName: "ATM10" }, { modelInstalled: true });
  for (const key of ["deleteBackups", "migrate", "deleteModel"]) assert.equal(idle[key].locked, false, key);
  const noPack = dataActionLocks({ busy: false, instancePath: "" }, { modelInstalled: false });
  assert.equal(noPack.deleteBackups.locked, true);
  assert.match(noPack.deleteBackups.reason, /選好遊戲資料夾/);
  assert.equal(noPack.deleteModel.locked, true);
  assert.match(noPack.deleteModel.reason, /沒有/);
  // 主視窗還沒回報狀態：失效安全＝當成忙碌中（不讓玩家在翻譯中刪東西）
  const unknown = dataActionLocks(null, { modelInstalled: true });
  assert.equal(unknown.deleteBackups.locked, true);
  assert.equal(unknown.migrate.locked, true);
});

test("翻完刪除翻譯結果：第一次勾要再勾「我了解」才生效；勾過一次之後直接生效", () => {
  assert.deepEqual(deleteResultsStep({ wantOn: true, acked: false }), { save: null, showAck: true });
  assert.deepEqual(deleteResultsStep({ wantOn: true, acked: true }), { save: "1", showAck: false });
  assert.deepEqual(deleteResultsStep({ wantOn: false, acked: true }), { save: "0", showAck: false });
  assert.deepEqual(deleteResultsStep({ wantOn: false, acked: false }), { save: "0", showAck: false });
  assert.match(ROW_COPY.deleteResultsAck, /我了解之後模組整合包更新只能整包重翻/);
  // G1.8：不保留翻譯結果不影響備份
  assert.match(ROW_COPY.deleteResultsHelp, /備份/);
});

test("本地翻不好時：下方顯示目前會用哪個線上 AI 與是否可用", () => {
  assert.match(describeOnlineAi({ hasKey: true, provider: "deepseek" }), /自訂 API.*DeepSeek/);
  assert.match(describeOnlineAi({ hasKey: false, gptUsable: true }), /ChatGPT/);
  assert.match(describeOnlineAi({ hasKey: false, gptUsable: false }), /還沒有可用的線上 AI/);
  assert.match(describeOnlineAi(null), /還沒有可用的線上 AI/);
});

test("搬移結果同列顯示，並請玩家重新開啟工具；大小格式", () => {
  assert.match(migrateResultLine({ files: 12 }), /請重新開啟工具/);
  assert.equal(formatGb(5.2 * 1024 ** 3), "5.2 GB");
  assert.equal(formatGb(0), "");
  assert.equal(formatGb(undefined), "");
});

test("每一列說明 ≤40 字（規格 §5.2）", () => {
  for (const [key, text] of Object.entries(ROW_COPY)) {
    assert.ok(textLength(text) <= 40, `${key} 超過 40 字：${text}`);
  }
});
