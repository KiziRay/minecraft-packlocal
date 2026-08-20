import test from "node:test";
import assert from "node:assert/strict";

import {
  buildChatForwardBody,
  clampCompletionTokens,
  normalizeResponseFormat,
  normalizeThinking,
} from "../src/index.js";

const env = { UPSTREAM_MODEL: "deepseek-v4-flash" };
const baseBody = {
  model: "deepseek-v4-flash",
  messages: [{ role: "user", content: "hello" }],
  temperature: 0.2,
};

test("buildChatForwardBody 保留 model/messages/temperature", () => {
  const forward = buildChatForwardBody(baseBody, env);
  assert.equal(forward.model, "deepseek-v4-flash");
  assert.deepEqual(forward.messages, baseBody.messages);
  assert.equal(forward.temperature, 0.2);
});

test("buildChatForwardBody 鎖定 env.UPSTREAM_MODEL，忽略 body.model", () => {
  const forward = buildChatForwardBody({ ...baseBody, model: "gpt-4" }, env);
  assert.equal(forward.model, "deepseek-v4-flash");
});

test("buildChatForwardBody 強制 thinking.disabled（忽略客戶端 enabled）", () => {
  const forward = buildChatForwardBody(baseBody, env);
  assert.deepEqual(forward.thinking, { type: "disabled" });
  const enabled = buildChatForwardBody(
    { ...baseBody, thinking: { type: "enabled" } },
    env
  );
  assert.deepEqual(enabled.thinking, { type: "disabled" });
});

test("normalizeThinking 只放行 enabled／disabled", () => {
  assert.deepEqual(normalizeThinking({ type: "disabled" }), { type: "disabled" });
  assert.deepEqual(normalizeThinking({ type: "enabled" }), { type: "enabled" });
  assert.equal(normalizeThinking({ type: "maybe" }), undefined);
  assert.equal(normalizeThinking(null), undefined);
});

test("response_format 只放行 json_object", () => {
  assert.deepEqual(normalizeResponseFormat({ type: "json_object" }), { type: "json_object" });
  assert.deepEqual(normalizeResponseFormat("json_object"), { type: "json_object" });
  assert.equal(normalizeResponseFormat({ type: "text" }), undefined);
  assert.equal(normalizeResponseFormat({ type: "json_object", schema: {} }), undefined);
  assert.equal(normalizeResponseFormat(null), undefined);
});

test("buildChatForwardBody 轉發合法 response_format，忽略其他 shape", () => {
  const pass = buildChatForwardBody(
    { ...baseBody, response_format: { type: "json_object" } },
    env
  );
  assert.deepEqual(pass.response_format, { type: "json_object" });

  const drop = buildChatForwardBody(
    { ...baseBody, response_format: { type: "text" } },
    env
  );
  assert.equal(drop.response_format, undefined);
});

test("max_tokens 只接受 number 並夾在 1..8192", () => {
  assert.equal(clampCompletionTokens(512), 512);
  assert.equal(clampCompletionTokens(0), 1);
  assert.equal(clampCompletionTokens(99999), 8192);
  assert.equal(clampCompletionTokens(1.9), 1);
  assert.equal(clampCompletionTokens("512"), undefined);
  assert.equal(clampCompletionTokens(null), undefined);
});

test("buildChatForwardBody 轉發 max_tokens，忽略非法值", () => {
  const pass = buildChatForwardBody({ ...baseBody, max_tokens: 2048 }, env);
  assert.equal(pass.max_tokens, 2048);

  const clamped = buildChatForwardBody({ ...baseBody, max_tokens: 12000 }, env);
  assert.equal(clamped.max_tokens, 8192);

  const drop = buildChatForwardBody({ ...baseBody, max_tokens: "2048" }, env);
  assert.equal(drop.max_tokens, undefined);
});

test("buildChatForwardBody 也支援 max_completion_tokens", () => {
  const pass = buildChatForwardBody({ ...baseBody, max_completion_tokens: 1024 }, env);
  assert.equal(pass.max_completion_tokens, 1024);

  const drop = buildChatForwardBody({ ...baseBody, max_completion_tokens: "1024" }, env);
  assert.equal(drop.max_completion_tokens, undefined);
});

test("proxyChat 使用 buildChatForwardBody 組裝轉發 body", async () => {
  const { readFile } = await import("node:fs/promises");
  const source = await readFile(new URL("../src/index.js", import.meta.url), "utf8");
  const start = source.indexOf("async function proxyChat");
  const end = source.indexOf("async function authorizeManagedAi", start);
  const proxy = source.slice(start, end);
  assert.match(proxy, /buildChatForwardBody\(body, env\)/);
  assert.doesNotMatch(proxy, /const forward = \{\s*\n\s*model:/);
});
