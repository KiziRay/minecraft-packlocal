import test from "node:test";
import assert from "node:assert/strict";

import {
  GLOSSARY_MAX_ZH_LEN,
  TM_MAX_ZH_LEN,
  isPackLayerNs,
  pickWinningVote,
  tmCanUse,
  tmMerge,
  tmNormalizeRecord,
  tmZhAcceptable,
  utf8ByteLength,
} from "../src/tm.mjs";

test("舊字串紀錄視為 1 票且 conflict 不再阻擋", () => {
  const record = tmNormalizeRecord("扳手");
  assert.equal(record.votes[0].n, 1);
  assert.equal(tmCanUse("扳手", "", "", "create"), null);
  assert.equal(tmCanUse({ zh: "扳手", conflict: true, packs: { a: "A" }, votes: undefined }, "", "a", "create"), "扳手");
});

test("舊 packs 鍵數可當票數，跨包需 ≥2", () => {
  const old = { zh: "扳手", ctx: "", packs: { aa: "A", bb: "B" }, conflict: true };
  assert.equal(tmCanUse(old, "", "", "create"), "扳手");
  assert.equal(tmCanUse({ zh: "扳手", packs: { aa: "A" } }, "", "", "create"), null);
});

test("同包 query.pk 在 packs 內時 1 票可 hit", () => {
  const rec = { zh: "扳手", packs: { pk1: "Pack" }, votes: [{ zh: "扳手", n: 1, packs: { pk1: "Pack" } }] };
  assert.equal(tmCanUse(rec, "", "pk1", "create"), "扳手");
  assert.equal(tmCanUse(rec, "", "other", "create"), null);
});

test("pack.* 層 1 票即可", () => {
  assert.equal(isPackLayerNs("pack.abc"), true);
  const rec = { zh: "戰役", votes: [{ zh: "戰役", n: 1, packs: {} }] };
  assert.equal(tmCanUse(rec, "", "", "pack.abcdef"), "戰役");
  assert.equal(tmCanUse(rec, "", "", "create"), null);
});

test("不同譯文累積 votes、不再永久 conflict；平手不 hit", () => {
  const target = {};
  assert.equal(tmMerge(target, "k", { zh: "甲", packs: { p1: "A" } }), "accepted");
  assert.equal(tmMerge(target, "k", { zh: "乙", packs: { p2: "B" } }), "variant");
  assert.equal(target.k.conflict, undefined);
  assert.equal(target.k.votes.length, 2);
  assert.equal(pickWinningVote(target.k.votes), null);
  assert.equal(tmCanUse(target.k, "", "", "create"), null);
  tmMerge(target, "k", { zh: "甲", packs: { p3: "C" } });
  assert.equal(tmCanUse(target.k, "", "", "create"), "甲");
});

test("長句約 8KB 可接受，超過則拒", () => {
  const ok = "翻".repeat(2000);
  assert.ok(utf8ByteLength(ok) > 400);
  assert.ok(tmZhAcceptable(ok, TM_MAX_ZH_LEN));
  assert.equal(tmZhAcceptable(ok, GLOSSARY_MAX_ZH_LEN), false);
  const tooLong = "翻".repeat(5000);
  assert.ok(utf8ByteLength(tooLong) > TM_MAX_ZH_LEN);
  assert.equal(tmZhAcceptable(tooLong), false);
});
