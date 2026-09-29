import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { SETTING_PATHS, isAllowedSettingPath } from "./settings-paths.js";
import { applyOps, opDelete, opSet } from "./settings-patch.js";
import { routeSettingsUpdate } from "./settings-sync.js";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel) => readFileSync(join(here, rel), "utf8").replace(/\r\n/g, "\n");
// settings-store.js 依賴瀏覽器的 window，node 裡不能直接 import；從原始碼取出 KEY_MAP。
const storeText = read("./settings-store.js");
const mapStart = storeText.indexOf("const KEY_MAP = {");
const keyMapBlock = storeText.slice(mapStart, storeText.indexOf("};", mapStart));
const KEY_MAP = Object.fromEntries([...keyMapBlock.matchAll(/"([^"]+)":\s*"([^"]+)"/g)].map((m) => [m[1], m[2]]));

test("前端只寫白名單路徑，擋下原型鏈與 version／migration", () => {
  for (const bad of ["__proto__.polluted", "constructor.prototype", "appearance.__proto__", "version", "migration.version", "x.y"]) {
    assert.equal(isAllowedSettingPath(bad), false, bad);
    const target = {};
    assert.equal(applyOps(target, [opSet(bad, "1")]), 0, `不可寫入 ${bad}`);
    assert.equal(applyOps(target, [opDelete(bad)]), 0);
    assert.deepEqual(target, {});
  }
  assert.equal({}.polluted, undefined, "Object.prototype 不可被汙染");
  const settings = {};
  assert.equal(applyOps(settings, [opSet("appearance.theme", "light")]), 1);
});

test("中間節點不是物件時不覆蓋", () => {
  const settings = { appearance: "壞掉的值" };
  assert.equal(applyOps(settings, [opSet("appearance.theme", "dark")]), 0);
  assert.equal(settings.appearance, "壞掉的值");
});

test("設定同步不把白名單外的路徑寫進主視窗快取", () => {
  const stored = [];
  routeSettingsUpdate({ path: "__proto__.polluted", value: "1" }, { store: (p) => stored.push(p) });
  routeSettingsUpdate({ path: "version", value: "9" }, { store: (p) => stored.push(p) });
  routeSettingsUpdate({ path: "translate.backupChoice", value: null }, { store: (p) => stored.push(p) });
  assert.deepEqual(stored, ["translate.backupChoice"]);
});

test("KEY_MAP 的每個路徑都在白名單上（同一份清單）", () => {
  for (const dotted of Object.values(KEY_MAP)) {
    assert.ok(SETTING_PATHS.includes(dotted), `白名單缺 ${dotted}`);
  }
  assert.ok(KEY_MAP["mcpl-webview-scale"] === "appearance.uiScale", "縮放值要寫進設定檔");
  assert.ok(KEY_MAP["mcpl-webview-autoscale"] === "appearance.uiAutoScale");
  const rust = read("../../src-tauri/src/engine/app_settings.rs");
  assert.ok(rust.includes('include_str!("../../../src/core/settings-paths.js")'), "後端要讀同一份清單");
});

test("後端不再有整份覆寫設定檔的入口", () => {
  const lib = read("../../src-tauri/src/lib.rs");
  const devMode = read("../../src-tauri/src/engine/dev_mode.rs");
  assert.ok(!lib.includes("write_app_settings_cmd"));
  assert.ok(!devMode.includes("write_settings("), "開發模式開關要走合併寫入");
});

test("本地翻不好改用線上補完：預設不勾，未選擇要說明，打開要先同意", () => {
  const html = read("../settings.html");
  const script = read("../settings-window.js");
  assert.ok(!/id="local-cloud-topup"[^>]*checked/.test(html), "預設不得勾選");
  assert.ok(script.includes("尚未選擇（第一次需要時會問你）"));
  assert.ok(script.includes("CLOUD_TOPUP_CONSENT"), "設定視窗要用和主視窗相同的同意說明");
  // B5b：主視窗不再跳同意框，改成開始前確認的一列（預設只用本地；選「改用線上」＝同意，且寫明會用額度）
  assert.ok(!read("../app.js").includes("CLOUD_TOPUP_CONSENT"), "主視窗不再跳線上補完同意框");
  const prestart = read("../flow/prestart.js");
  assert.ok(prestart.includes("改用線上 AI 補完（會用額度）") && prestart.includes('src.cloudDraft === "1" ? "1" : "0"'), "預設只用本地，要明確選才算同意");
  assert.ok(read("./cloud-topup-consent.js").includes("額度"), "同意說明要講明會用到線上 AI 額度");
});

test("設定檔寫成功後才寫 localStorage", () => {
  const script = read("../settings-window.js");
  const body = script.slice(script.indexOf("export async function saveSetting"));
  const patchAt = body.indexOf("await patchSettings(");
  const localAt = body.indexOf("writeLocal(");
  assert.ok(patchAt > 0 && localAt > patchAt, "要先 patchSettings 成功才寫 localStorage");
});

test("改套用前要不要備份會通知主視窗更新快取（B5a-2：三選一，「每次詢問」＝刪除設定）", () => {
  const pane = read("../settings/data-pane.js");
  const handler = pane.slice(pane.indexOf("async function onBackupChoiceChange"));
  assert.ok(handler.slice(0, 1200).includes("saveAndAnnounce("), "要經合併寫入後送設定同步事件");
  assert.ok(handler.slice(0, 1200).includes("opDelete(BACKUP_CHOICE_PATH)"), "每次詢問＝刪除設定，不寫空值");
  const announce = pane.slice(pane.indexOf("async function saveAndAnnounce"));
  assert.ok(announce.slice(0, 900).includes("SETTINGS_UPDATED_EVENT"));
});

test("縮放值經合併寫入設定檔，主視窗快捷鍵調整也會同步給設定視窗", () => {
  const script = read("../settings-window.js");
  assert.ok(script.includes('opSet("appearance.uiScale"'), "設定視窗的縮放要寫進設定檔");
  assert.ok(read("../ui-scale.js").includes("onScalePersisted"), "主視窗縮放要有寫回設定檔的掛勾");
  assert.ok(script.includes('source === "main"'), "設定視窗要接收主視窗送來的縮放");
});

test("update.js 不再引用已刪除的檢查更新按鈕", () => {
  const update = read("./update.js");
  assert.ok(!update.includes("btn-check-update"));
  assert.ok(!update.includes('"update-status"'));
});

test("文案：產品名、模組整合包、備份說明", () => {
  const html = read("../settings.html");
  assert.ok(html.includes("<title>Minecraft 模組整合包翻譯工具・設定</title>"));
  assert.ok(html.includes("Minecraft 模組整合包翻譯工具 <span"));
  assert.ok(!html.includes("工具會在裝進遊戲前幫你備份"));
  const index = read("../index.html");
  assert.ok(index.includes("目前模組整合包"));
  assert.ok(!index.includes(">目前整合包<"));
});

test("ChatGPT 說明文字白話、誠實說明會用到額度", async () => {
  const { GPT_COPY } = await import("../ai/copy.js");
  const index = read("../index.html");
  const note = index.slice(index.indexOf('<p id="gpt-auth-note">'), index.indexOf("</p>", index.indexOf('<p id="gpt-auth-note">')));
  for (const text of [GPT_COPY.noteGpt, note]) {
    assert.ok(text.includes("ChatGPT"), text);
    assert.ok(text.includes("消耗你的 ChatGPT 帳號額度"), `要誠實說明會用到額度：${text}`);
    for (const word of ["Codex", "端點", "權杖", "探測", "預檢", "429", "OpenAI"]) {
      assert.ok(!text.includes(word), `不可出現「${word}」：${text}`);
    }
  }
});
