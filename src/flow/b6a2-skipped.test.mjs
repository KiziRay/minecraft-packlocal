import test from "node:test";
import assert from "node:assert/strict";
import { packUpdateState } from "./pack-update.js";

test("B6a-2：因完整度設定略過、來源已變的項目，S15 詳細行至少寫出件數", () => {
  const s = packUpdateState({ packName: "包", packUpdate: { modsChanged: true, skippedChanged: 3 } });
  assert.ok(s.detailLines.some((l) => l.includes("3") && l.includes("完整度") && l.includes("來源已變")), JSON.stringify(s.detailLines));
});

test("B6a-2：沒有略過來源時不多一行；只有略過來源有變不會讓 S15 出現", () => {
  const a = packUpdateState({ packName: "包", packUpdate: { modsChanged: true } });
  assert.deepEqual(a.detailLines, []);
  assert.equal(packUpdateState({ packName: "包", packUpdate: { skippedChanged: 2 } }), null);
});
