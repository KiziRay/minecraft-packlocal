import test from "node:test";
import assert from "node:assert/strict";

import { buildToolUpdateDiscordPayload } from "../src/index.js";

test("buildToolUpdateDiscordPayload 產生 embed 而非純 content", () => {
  const payload = buildToolUpdateDiscordPayload(
    "1.0.3",
    "更新自動重開；額度指示",
    "https://example.test/download/MCPL-1.0.3.exe"
  );
  assert.ok(payload);
  assert.ok(Array.isArray(payload.embeds));
  assert.equal(payload.embeds.length, 1);
  assert.equal(payload.content, undefined);
  assert.equal(payload.embeds[0].title, "MCPL v1.0.3 更新");
  assert.match(payload.embeds[0].description, /更新自動重開/);
  assert.match(payload.embeds[0].description, /額度指示/);
  assert.equal(payload.embeds[0].url, "https://example.test/download/MCPL-1.0.3.exe");
  assert.equal(payload.embeds[0].fields[0].name, "下載");
});

test("buildToolUpdateDiscordPayload 無 notes 回 null", () => {
  assert.equal(buildToolUpdateDiscordPayload("1.0.3", "", "https://x"), null);
});
