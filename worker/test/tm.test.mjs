import test from "node:test";
import assert from "node:assert/strict";

import {
  GLOSSARY_MAX_ZH_LEN,
  TM_MAX_ZH_LEN,
  TM_VOTES_CAP,
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

test("同包 query.pks 任一命中即可用，舊 pk 仍相容", () => {
  const rec = { zh: "扳手", packs: { pk2: "Pack 2" }, votes: [{ zh: "扳手", n: 1, packs: { pk2: "Pack 2" } }] };
  assert.equal(tmCanUse(rec, "", ["pk1", "pk2"], "create"), "扳手");
  assert.equal(tmCanUse(rec, "", "pk2", "create"), "扳手");
  assert.equal(tmCanUse(rec, "", ["pk1", "pk3"], "create"), null);
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

test("votes 滿時新變體視為 duplicate，不計 accepted", () => {
  const target = {
    k: {
      zh: "甲",
      packs: {},
      votes: Array.from({ length: TM_VOTES_CAP }, (_, i) => ({ zh: `譯文${i}`, n: 1, packs: {} })),
    },
  };
  const before = JSON.stringify(target.k);
  assert.equal(tmMerge(target, "k", { zh: "新變體", packs: { p1: "A" } }), "duplicate");
  assert.equal(JSON.stringify(target.k), before);
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

test("同一個模組的同一個版本，跨整合包一票就能用", () => {
  // 整合包之間大量共用同樣的模組。`create:item.wrench` 在任何裝了同一版
  // Create 的包裡都是同一個字串、同一個意思——不該因為整合包不同就要等第二票。
  const shard = {};
  tmMerge(shard, "kh1", {
    zh: "扳手",
    packs: { packA: "Pack A" },
    mods: { "create-1.20.1-0.5.1.f": 1 },
  });

  // 另一個整合包、同一版 Create → 一票就採用
  assert.equal(
    tmCanUse(shard.kh1, "", "packB", "create", "create-1.20.1-0.5.1.f"),
    "扳手"
  );
  // 沒帶 mod 版本（舊版工具）→ 維持兩票門檻
  assert.equal(tmCanUse(shard.kh1, "", "packB", "create"), null);
  // 帶的是不同版本 → 也維持兩票門檻（改版後字串可能整個換掉）
  assert.equal(
    tmCanUse(shard.kh1, "", "packB", "create", "create-1.20.1-0.5.0.i"),
    null
  );
  // 原本那個包當然還是一票就能用
  assert.equal(tmCanUse(shard.kh1, "", "packA", "create"), "扳手");
});

test("舊記錄沒有 mods 欄位時，行為完全不變", () => {
  // 向下相容是硬要求：已經存在 R2 裡的資料不可以因為這次改動而失效或變寬。
  const legacy = { zh: "扳手", packs: { packA: "A" }, votes: [{ zh: "扳手", n: 1, packs: { packA: "A" } }] };
  assert.equal(tmCanUse(legacy, "", "packB", "create", "create-1.20.1-0.5.1.f"), null);
  assert.equal(tmCanUse(legacy, "", "packA", "create", "create-1.20.1-0.5.1.f"), "扳手");
  // 湊到兩票之後照樣可用（tmMerge 寫回 target，不會就地改 legacy）
  const target = { k: legacy };
  tmMerge(target, "k", { zh: "扳手", packs: { packB: "B" } });
  assert.equal(tmCanUse(target.k, "", "packC", "create"), "扳手");
});

test("模組版本只是判斷依據，不會憑空多出票數", () => {
  // 同一個整合包重複貢獻同一條，就算帶了不同的 mod 版本也不能變成兩票——
  // 否則單一使用者就能自己把一條翻譯推過跨包門檻。
  const shard = {};
  tmMerge(shard, "kh1", { zh: "扳手", packs: { packA: "A" }, mods: { "create-0.5.1.f": 1 } });
  tmMerge(shard, "kh1", { zh: "扳手", packs: { packA: "A" }, mods: { "create-0.5.0.i": 1 } });
  const vote = shard.kh1.votes.find((v) => v.zh === "扳手");
  assert.equal(vote.n, 1, "同一個整合包只能算一票");
  // 但兩個版本都記下來了，兩邊都能重用
  assert.equal(tmCanUse(shard.kh1, "", "packB", "create", "create-0.5.1.f"), "扳手");
  assert.equal(tmCanUse(shard.kh1, "", "packB", "create", "create-0.5.0.i"), "扳手");
});
