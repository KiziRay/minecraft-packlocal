import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { routeSettingsAction, routeSettingsUpdate } from "./settings-sync.js";

const here = dirname(fileURLToPath(import.meta.url));

function recorder() {
  const calls = [];
  const handler = (name) => (...args) => calls.push([name, ...args]);
  return {
    calls,
    handlers: Object.fromEntries(
      ["store", "theme", "uiScale", "uiAutoScale", "sfx", "outputStorage", "cacheRemind", "rememberApiKey"].map(
        (name) => [name, handler(name)]
      )
    ),
  };
}

test("音量與靜音在主視窗即時生效", () => {
  const { calls, handlers } = recorder();
  routeSettingsUpdate({ path: "appearance.sfxVolume", value: "0.2" }, handlers);
  routeSettingsUpdate({ path: "appearance.sfxMuted", value: "1" }, handlers);
  assert.deepEqual(calls.filter(([n]) => n === "sfx"), [
    ["sfx", { volume: 0.2 }],
    ["sfx", { muted: true }],
  ]);
  assert.equal(calls.filter(([n]) => n === "store").length, 2, "快取也要同步");
});

test("主題、縮放、自動縮放、結果位置都會分派到主視窗", () => {
  const { calls, handlers } = recorder();
  assert.equal(routeSettingsUpdate({ path: "appearance.theme", value: "light" }, handlers), "theme");
  assert.equal(routeSettingsUpdate({ path: "appearance.uiScale", value: "160" }, handlers), "uiScale");
  assert.equal(routeSettingsUpdate({ path: "appearance.uiAutoScale", value: "1" }, handlers), "uiAutoScale");
  assert.equal(routeSettingsUpdate({ path: "translate.outputCustomRoot", value: null }, handlers), "outputStorage");
  assert.ok(calls.some(([n, v]) => n === "uiAutoScale" && v === true));
  assert.ok(calls.some(([n, p, v]) => n === "store" && p === "translate.outputCustomRoot" && v === null));
  assert.equal(routeSettingsUpdate({ path: "" }, handlers), "ignored");
});

test("只接受已知的設定視窗動作", () => {
  const seen = [];
  const handlers = { "delete-backups": () => seen.push("delete"), "replay-onboarding": () => seen.push("onboard") };
  assert.equal(routeSettingsAction({ action: "delete-backups" }, handlers), "delete-backups");
  assert.equal(routeSettingsAction({ action: "replay-onboarding" }, handlers), "replay-onboarding");
  assert.equal(routeSettingsAction({ action: "rm -rf" }, handlers), "ignored");
  assert.deepEqual(seen, ["delete", "onboard"]);
});

test("主視窗接上即時同步與設定視窗動作", () => {
  const app = readFileSync(join(here, "../app.js"), "utf8");
  assert.ok(app.includes("routeSettingsUpdate("), "主視窗要用 routeSettingsUpdate 處理設定更新");
  assert.ok(app.includes("routeSettingsAction("), "主視窗要處理設定視窗送來的動作");
  assert.ok(app.includes("applySfxPrefs"), "音量要即時套用到主視窗音效");
});
