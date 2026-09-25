import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { applyOps, opDelete, opSet, planLocalStorageMigration } from "./settings-patch.js";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel) => readFileSync(join(here, rel), "utf8");

test("設值只改指定路徑，其餘欄位保留", () => {
  const settings = { version: 1, appearance: { theme: "dark", sfxVolume: "0.3" } };
  assert.equal(applyOps(settings, [opSet("appearance.theme", "light")]), 1);
  assert.equal(settings.appearance.theme, "light");
  assert.equal(settings.appearance.sfxVolume, "0.3");
});

test("null 值不會覆寫；清除必須用明確的刪除操作", () => {
  const settings = { translate: { outputCustomRoot: "D:/x", backupChoice: "never" } };
  assert.equal(applyOps(settings, [opSet("translate.outputCustomRoot", null)]), 0);
  assert.equal(settings.translate.outputCustomRoot, "D:/x");
  assert.equal(applyOps(settings, [opDelete("translate.backupChoice")]), 1);
  assert.ok(!("backupChoice" in settings.translate));
  assert.equal(applyOps(settings, [opDelete("translate.backupChoice")]), 0, "重複刪除不算改動");
});

test("舊 localStorage 只補設定檔缺的欄位，並認得 1.0.6／1.0.7 舊鍵", () => {
  const keyMap = {
    "modpack-i18n-theme": "appearance.theme",
    "modpack-i18n-consent-hide-v1.0.9": "consent.hideVersion",
    "modpack-i18n-onboarding-seen-v1.0.9": "onboarding.seenVersion",
  };
  const aliases = {
    "modpack-i18n-consent-hide-v1.0.6": "modpack-i18n-consent-hide-v1.0.9",
    "modpack-i18n-onboarding-seen-v1.0.7": "modpack-i18n-onboarding-seen-v1.0.9",
  };
  const local = {
    "modpack-i18n-theme": "light",
    "modpack-i18n-consent-hide-v1.0.6": "1",
    "modpack-i18n-onboarding-seen-v1.0.7": "1",
  };
  const file = { appearance: { theme: "dark" } };
  const ops = planLocalStorageMigration(file, keyMap, aliases, (k) => (k in local ? local[k] : null));
  assert.deepEqual(ops, [
    { path: "consent.hideVersion", value: "1" },
    { path: "onboarding.seenVersion", value: "1" },
  ]);
});

test("設定視窗與設定存取層都改走依路徑合併寫入，不再整包覆寫", () => {
  const windowScript = read("../settings-window.js");
  const store = read("./settings-store.js");
  for (const [name, text] of [["settings-window.js", windowScript], ["settings-store.js", store]]) {
    assert.ok(text.includes('"patch_app_settings_cmd"'), `${name} 要用 patch_app_settings_cmd`);
    assert.ok(!text.includes('invoke("write_app_settings_cmd"'), `${name} 不可再整份覆寫設定檔`);
  }
  assert.ok(store.includes('health.status !== "unreadable"'), "設定檔讀不出來時不可寫入");
});

test("本地模型常駐設定已移除", () => {
  const app = read("../app.js");
  // 字串拆開寫，讓「src/ 內 0 處」的 grep 驗收不會被測試本身命中
  assert.ok(!app.includes(["KEEP_LOCAL", "MODEL_KEY"].join("_")));
  assert.ok(!app.includes("shouldKeepLocalModelAlive"));
  assert.ok(!read("../index.html").includes('id="keep-local-model"'));
  assert.ok(!read("../settings.html").includes("常駐"));
});
