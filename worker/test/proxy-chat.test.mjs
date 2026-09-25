import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

test("Worker 已移除免費代管 chat 代理", async () => {
  const source = await readFile(new URL("../src/index.js", import.meta.url), "utf8");
  assert.doesNotMatch(source, /async function proxyChat/);
  assert.doesNotMatch(source, /\/v1\/chat\/completions/);
  assert.doesNotMatch(source, /\/api\/managed\/usage/);
  assert.doesNotMatch(source, /\/api\/managed\/gp-reward/);
  assert.doesNotMatch(source, /function managedUsage/);
  assert.doesNotMatch(source, /function managedGpReward/);
});
