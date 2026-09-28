// B5a-2 #1：設定視窗四分類（規格 §1.3）、新設定路徑白名單一致（G0.1、G0.2）、主視窗即時跟著變。
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { SETTING_PATHS } from "../core/settings-paths.js";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel) => readFileSync(join(here, rel), "utf8").replace(/\r\n/g, "\n");
const html = read("../settings.html");
const app = read("../app.js");
const lib = read("../../src-tauri/src/lib.rs");
const settingsScripts = ["../settings-window.js", "../settings-window-actions.js", "./data-pane.js"].map(read).join("\n");

function pane(name) {
  const start = html.indexOf(`<section id="pane-${name}"`);
  assert.ok(start > 0, `缺少分類 ${name}`);
  const next = html.indexOf('<section id="pane-', start + 10);
  return html.slice(start, next > 0 ? next : html.indexOf("</main>"));
}

const storeText = read("../core/settings-store.js");
const mapStart = storeText.indexOf("const KEY_MAP = {");
const KEY_MAP = Object.fromEntries(
  [...storeText.slice(mapStart, storeText.indexOf("};", mapStart)).matchAll(/"([^"]+)":\s*"([^"]+)"/g)].map((m) => [m[1], m[2]])
);

test("四分類各放規格 §1.3 的項目（每項在唯一的分類）", () => {
  const expected = {
    general: ["theme-select", "ui-autoscale", "ui-scale", "ui-scale-reset", "minimize-on-close", "sfx-muted", "sfx-volume"],
    translate: ["remember-api-key", "clear-api-key", "local-cloud-topup", "local-cloud-topup-state", "local-cloud-topup-ai"],
    data: [
      "output-storage-mode", "pick-output-custom-root", "clear-output-custom-root", "output-storage-note",
      "delete-results-after-apply", "delete-results-ack", "backup-choice", "backup-choice-label",
      "delete-backups", "delete-backups-reason", "local-model-size", "local-model-dir", "delete-local-model",
      "delete-local-model-reason", "data-root", "migrate-data-root", "migrate-data-root-result", "dev-mode",
    ],
    help: ["check-update", "replay-onboarding"],
  };
  for (const [name, ids] of Object.entries(expected)) {
    const text = pane(name);
    for (const id of ids) {
      assert.ok(text.includes(`id="${id}"`), `${name} 缺 #${id}`);
      const count = html.split(`id="${id}"`).length - 1;
      assert.equal(count, 1, `#${id} 只能出現一次`);
    }
  }
  assert.match(pane("help"), /重看引導與說明/);
  assert.match(pane("translate"), /AI 來源在主畫面/);
});

test("刪除：發現已翻過時提醒我、到主畫面選 AI、回主畫面翻譯、回主畫面回報問題（規格 §1.3）", () => {
  for (const id of ["cache-remind", "goto-ai", "goto-translation", "report-issue", "reset-backup-choice"]) {
    assert.ok(!html.includes(`id="${id}"`), `#${id} 應已刪除`);
    assert.ok(!settingsScripts.includes(`$("${id}")`), `還在接 #${id}`);
  }
  assert.ok(!app.includes('listen("mcpl:open-main-section"'), "跨視窗捲動目標已刪");
  assert.ok(!app.includes('listen("mcpl:show-issue-report"'));
  assert.ok(!app.includes("readCacheRemindEnabled"), "提醒設定已刪，狀態卡永遠顯示現況");
});

test("設定視窗不再用原生 confirm；D-12／D-14／D-07／不備份都走工具自己的對話框", () => {
  assert.ok(!/window\.confirm\(/.test(settingsScripts));
  assert.ok(settingsScripts.includes('from "./ui/confirm.js"') || settingsScripts.includes('from "../ui/confirm.js"'));
  for (const name of ["SETTINGS_DIALOGS.clearKey", "SETTINGS_DIALOGS.cloudTopUp", "SETTINGS_DIALOGS.noBackup", "deleteBackupsDialog(", "deleteModelDialog("]) {
    assert.ok(settingsScripts.includes(name), `沒有用 ${name}`);
  }
});

test("新設定路徑同時在白名單與 KEY_MAP（G0.1、G0.2）；舊的提醒設定路徑留在白名單但不再讀", () => {
  for (const path of ["translate.deleteResultsAfterApply", "translate.deleteResultsAck"]) {
    assert.ok(SETTING_PATHS.includes(path), `白名單缺 ${path}`);
    assert.ok(Object.values(KEY_MAP).includes(path), `KEY_MAP 缺 ${path}`);
  }
  assert.ok(SETTING_PATHS.includes("translate.cacheRemind"), "舊設定檔含這個值時照常讀得進來（只是忽略）");
  assert.ok(!Object.values(KEY_MAP).includes("translate.cacheRemind"));
  assert.ok(storeText.includes('"modpack-i18n-cache-remind-v1"'), "舊 localStorage 鍵要順手清掉");
});

test("開始翻譯不再問「這次的翻譯結果要保留嗎」，改讀設定「翻完刪除翻譯結果」（唯一位置在設定）", () => {
  assert.ok(!app.includes("這次的翻譯結果要保留嗎"));
  assert.ok(app.includes("deleteResultsAfterApplyEnabled()"));
  const run = app.slice(app.indexOf("async function onRunInner"), app.indexOf("async function onRunInner") + 12000);
  assert.ok(run.includes("const skipResultFolder = deleteResultsAfterApplyEnabled();"));
});

test("主視窗即時跟著設定視窗變：清除金鑰、刪除本地模型、刪除備份、重看說明", () => {
  const bridge = read("../flow/settings-bridge.js");
  for (const name of ["apiKeyCleared", "localModelDeleted", "backupsDeleted"]) {
    assert.ok(bridge.includes(`listen(SETTINGS_NOTICE_EVENTS.${name}`), `主視窗沒收 ${name}`);
  }
  assert.ok(app.includes("wireSettingsNotices({"));
  const wiring = app.slice(app.indexOf("wireSettingsNotices({"), app.indexOf("wireSettingsNotices({") + 1200);
  assert.ok(wiring.includes("refreshApiSettings()"), "清除金鑰後主畫面立即更新金鑰欄");
  assert.ok(wiring.includes("refreshAiStatus()"));
  assert.ok(wiring.includes("refreshBackupState()"));
  assert.ok(app.includes("announceMainState();"), "狀態改變要回報給設定視窗");
  assert.ok(app.includes("disclosure.resetAll()"), "重看引導與說明要把說明重設為第一次");
  assert.ok(!app.includes("async function deleteAllBackupsFlow"), "D-07 在設定視窗內，主視窗舊流程刪除");
  assert.ok(!read("../index.html").includes('id="btn-local-llm-delete"'), "刪除本地模型只在設定");
});

test("後端：翻譯中不能搬工具資料（忙碌檢查）；備份實際位置有唯讀查詢", () => {
  const fnAt = lib.indexOf("async fn migrate_data_root_cmd");
  assert.ok(lib.slice(fnAt, fnAt + 600).includes("TRANSLATION_ACTIVE.load("));
  assert.ok(lib.includes("fn apply_backup_location_cmd("));
  assert.ok(lib.includes("apply_backup_location_cmd,"), "要註冊到 invoke_handler");
});
