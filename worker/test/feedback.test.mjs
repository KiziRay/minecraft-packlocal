import test from "node:test";
import assert from "node:assert/strict";

import { buildFeedbackDiscordPayload } from "../src/feedback.mjs";

test("buildFeedbackDiscordPayload 產生 embed 而非 content", () => {
  const payload = buildFeedbackDiscordPayload({
    clientId: "f1bf1f85-abc123",
    rating: 5,
    note: "非常好用",
    painPoint: "none",
    wish: "more_mods",
    source: "nudge",
    toolVersion: "1.0.5",
  });
  assert.ok(payload);
  assert.equal(payload.content, undefined);
  assert.ok(Array.isArray(payload.embeds));
  assert.equal(payload.embeds.length, 1);
  assert.equal(payload.embeds[0].title, "MCPL 使用回饋（匿名）");
  assert.equal(payload.embeds[0].color, 0x35c5c9);
  assert.equal(payload.embeds[0].footer.text, "模組包翻譯工具 · ZeitFrei");
  assert.ok(payload.embeds[0].timestamp);

  const byName = Object.fromEntries(payload.embeds[0].fields.map((f) => [f.name, f.value]));
  assert.equal(byName["評分"], "5/5");
  assert.match(byName["痛點"], /沒問題/);
  assert.match(byName["期望"], /更多模組/);
  assert.equal(byName["備註"], "非常好用");
  assert.equal(byName["來源"], "nudge");
  assert.equal(byName["版本"], "1.0.5");
  assert.equal(byName["clientId"], "f1bf1f85-a…");
});

test("buildFeedbackDiscordPayload 只含有值的 fields", () => {
  const payload = buildFeedbackDiscordPayload({
    clientId: "abcdefghij",
    rating: 3,
  });
  const names = payload.embeds[0].fields.map((f) => f.name);
  assert.deepEqual(names, ["評分", "clientId"]);
});

test("buildFeedbackDiscordPayload 無 clientId 回 null", () => {
  assert.equal(buildFeedbackDiscordPayload({}), null);
  assert.equal(buildFeedbackDiscordPayload({ clientId: "" }), null);
});
