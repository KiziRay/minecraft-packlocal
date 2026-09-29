// B5a-2 #4 清死碼：刪掉的東西不再被任何程式引用；#use-ai、#backup-before-apply 仍在（B5b 才取代）。
import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { HELP_COPY } from "./ui/help-copy.js";

const here = dirname(fileURLToPath(import.meta.url));
const read = (name) => readFileSync(join(here, name), "utf8").replace(/\r\n/g, "\n");
const html = read("index.html");
const app = read("app.js");

test("help-copy 只留畫面上真的有「？」在用的 key（刪 4 個死 key）", () => {
  for (const dead of ["supplement", "force-refresh", "pack-options", "settings"]) {
    assert.ok(!(dead in HELP_COPY), `死 key ${dead} 還在`);
  }
  const used = new Set([...html.matchAll(/data-help="([^"]+)"/g)].map((m) => m[1]));
  for (const key of Object.keys(HELP_COPY)) assert.ok(used.has(key), `${key} 沒有「？」在用`);
});

test("縮放不再呼叫已不存在的 flashAppSettingsSaved", () => {
  assert.ok(!read("ui-scale.js").includes("flashAppSettingsSaved"));
});

test("設定視窗早就沒有本包選項：mcpl:pack-options 跨視窗橋刪除", () => {
  for (const dead of ["mcpl:pack-options", "PACK_OPTION_FIELDS", "readPackOptions", "applyPackOptions"]) {
    assert.ok(!app.includes(dead), `app.js 還有 ${dead}`);
  }
});

test("審查 F4：atmosphere-banner.png 保留（計畫與 deferred 要求，B10 刪素材時排除）", () => {
  assert.ok(existsSync(join(here, "assets/atmosphere-banner.png")));
});

test("審查 F5：.update-button 死 CSS 刪除（HTML／JS 0 引用）", () => {
  const js = [app, read("core/update.js"), read("core/update-status.js"), read("flow/pack-actions.js")].join("\n");
  assert.ok(!html.includes("update-button") && !js.includes("update-button"), "仍有引用就不能刪");
  assert.ok(!read("styles/options.css").includes(".update-button"));
});

test("本包選項沒有空標題「翻譯方式」；#use-ai 仍保留（#backup-before-apply 在 B5b 由開始前確認的備份列取代後刪除）", () => {
  assert.ok(!html.includes('id="translation-method-group"'));
  assert.ok(!html.includes(">翻譯方式<"));
  assert.ok(!html.includes('id="backup-before-apply"'), "B5b：備份改在開始前確認的備份列問");
  assert.ok(html.includes('id="use-ai"'), "#use-ai 是 AI 來源的現行狀態欄位");
});
