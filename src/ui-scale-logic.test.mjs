import test from "node:test";
import assert from "node:assert/strict";
import {
  clampScalePercent,
  computeAutoScalePercent,
  evaluateHitProbes,
  hitTargetOk,
  parseCssZoom,
  parseStoredAuto,
  parseStoredScale,
  visualToCssPx,
} from "./ui-scale-logic.js";

test("clampScalePercent 限制 80–170 且以 10 為步進", () => {
  assert.equal(clampScalePercent(100), 100);
  assert.equal(clampScalePercent(84), 80);
  assert.equal(clampScalePercent(175), 170);
  assert.equal(clampScalePercent("120"), 120);
  assert.equal(clampScalePercent(NaN), 100);
});

test("computeAutoScalePercent 依螢幕寬（ZeitFrei 檔）", () => {
  assert.equal(computeAutoScalePercent(1366), 90);
  assert.equal(computeAutoScalePercent(1920), 100);
  assert.equal(computeAutoScalePercent(2560), 120);
  assert.equal(computeAutoScalePercent(3840), 140);
  assert.equal(computeAutoScalePercent(0), 100);
});

test("parseCssZoom 與 visualToCssPx", () => {
  assert.equal(parseCssZoom("1.4"), 1.4);
  assert.equal(parseCssZoom("0"), 1);
  assert.equal(visualToCssPx(140, 1.4), 100);
});

test("parseStored 讀取儲存值", () => {
  assert.equal(parseStoredScale("150"), 150);
  assert.equal(parseStoredAuto("1"), true);
  assert.equal(parseStoredAuto("0"), false);
});

test("hitTargetOk 接受自身或子節點", () => {
  const parent = { contains(node) { return node === this.child; }, child: {} };
  parent.child = { id: "child" };
  assert.equal(hitTargetOk(parent, parent), true);
  assert.equal(hitTargetOk(parent.child, parent), true);
  assert.equal(hitTargetOk(null, parent), false);
  assert.equal(hitTargetOk({}, parent), false);
});

test("evaluateHitProbes 失敗時列出 id", () => {
  const expected = { contains() { return false; } };
  const r = evaluateHitProbes([
    { id: "tab-translate", expected, hit: expected },
    { id: "btn-quit", expected, hit: null },
  ]);
  assert.equal(r.ok, false);
  assert.deepEqual(r.failed, ["btn-quit"]);
});
