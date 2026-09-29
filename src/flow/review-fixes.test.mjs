/**
 * B5a-1 審查修正（低 a–e）。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel) => readFileSync(join(here, rel), "utf8").replace(/\r\n/g, "\n");

test("低 a：刪掉已無作用的 CSS 與診斷說明", () => {
  const cssDir = join(here, "../styles");
  const css = readdirSync(cssDir).map((f) => read(`../styles/${f}`)).join("\n");
  for (const sel of ["#rail-diagnose", ".path-gate-hint", ".settings-health-card", ".workbench-nav-primary", ".stage-sub-ready", ".stage-sub-empty", ".output-danger-row"]) {
    assert.ok(!css.includes(sel), `還有無作用的 ${sel}`);
  }
  const copy = read("../ui/help-copy.js");
  assert.ok(!copy.includes('"diagnose"'));
});

test("低 b：匯入結果照實寫——只有找不到對應項目時不說「已併入翻譯」；紀錄寫完整清單", async () => {
  const { describeImportReport } = await import("./pack-actions.js");
  const onlyUnknown = describeImportReport({ summary: "", accepted: 0, rejected: [], unknownKeys: ["a", "b"] });
  assert.doesNotMatch(onlyUnknown.toast, /已併入翻譯/);
  assert.match(onlyUnknown.toast, /沒有併入/);
  const many = Array.from({ length: 12 }, (_, i) => `k${i}`);
  const mixed = describeImportReport({ summary: "併入 3 條", accepted: 3, rejected: many, unknownKeys: [] });
  assert.match(mixed.toast, /部分退回/);
  for (const k of many) assert.ok(mixed.log.includes(k), "紀錄要有完整清單");
  const ok = describeImportReport({ summary: "併入 5 條", accepted: 5, rejected: [], unknownKeys: [] });
  assert.equal(ok.toast, "已併入翻譯");
});

test("低 c：手動「檢查更新」不受曾關閉橫幅影響；翻譯中延後的手動檢查結束後開更新視窗，只有自動檢查走橫幅", async () => {
  const { pendingUpdateTarget } = await import("../core/update-status.js");
  assert.equal(pendingUpdateTarget({ interactive: true, hasBanner: true }), "modal");
  assert.equal(pendingUpdateTarget({ interactive: false, hasBanner: true }), "banner");
  assert.equal(pendingUpdateTarget({ interactive: false, hasBanner: false }), "modal");
  const src = read("../core/update.js");
  assert.ok(src.includes("pendingUpdateTarget("));
});

test("低 d：主要按鈕隱藏時 aria-disabled 設回 false", async () => {
  const { applyStatusCard, planStatusCard } = await import("./status-card.js");
  const { computePackState } = await import("./pack-state.js");
  const nodes = {};
  const $ = (id) => {
    if (!nodes[id]) {
      const attrs = {};
      nodes[id] = {
        hidden: false,
        dataset: {},
        textContent: "",
        attrs,
        setAttribute: (k, v) => (attrs[k] = String(v)),
        getAttribute: (k) => attrs[k] ?? null,
        removeAttribute: (k) => delete attrs[k],
        querySelector: () => null,
      };
    }
    return nodes[id];
  };
  const base = { consentAccepted: true, instancePath: "C:/x/ATM10", validation: { ok: true } };
  applyStatusCard(planStatusCard(computePackState({ ...base, busy: true, busyKind: "apply" })), { $ });
  assert.equal(nodes["btn-run"].getAttribute("aria-disabled"), "true");
  applyStatusCard(planStatusCard(computePackState({ ...base, busy: true, busyKind: "translate" })), { $ });
  assert.equal(nodes["btn-run"].hidden, true);
  assert.equal(nodes["btn-run"].getAttribute("aria-disabled"), "false", "隱藏時歸回 false");
});

test("低 e：引導第 1 步不叫玩家按背景裡按不到的鈕；按「知道了」後焦點回到框住的按鈕", async () => {
  const { TOUR_STEPS } = await import("../onboarding/tour-steps.js");
  assert.match(TOUR_STEPS[0].body, /按「知道了」後/);
  const onboarding = read("../onboarding/onboarding.js");
  assert.ok(onboarding.includes("focusStepTarget("), "暫停時把焦點交給框住的按鈕");
});

test("流程邏輯移出 app.js：pack-actions.js 提供狀態卡輸入、D 區與橫幅接線", async () => {
  const mod = await import("./pack-actions.js");
  for (const name of ["createPackActions", "describeImportReport"]) assert.equal(typeof mod[name], "function", name);
  const app = read("../app.js");
  assert.ok(app.includes("createPackActions("));
  assert.ok(!app.includes("function syncFolderArea()"), "D 區邏輯在 pack-actions.js");
  assert.ok(!app.includes("function bannerArea()"), "橫幅邏輯在 pack-actions.js");
});

test("R-1 例外清單只多一個暫行項（待套用卡提供主要動作；B5d 已刪接續卡），不得再擴大", async () => {
  const { ZERO_PRIMARY_ALLOWED } = await import("./pack-state.js");
  assert.deepEqual([...ZERO_PRIMARY_ALLOWED].sort(), ["S10", "S11-card", "S17", "S18"].sort());
});
