import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { renderDiscordJoinContent } from "../src/index.js";

test("renderDiscordJoinContent 產生可點選 Discord 連結", () => {
  const content = renderDiscordJoinContent("123456789012345678", "TestUser");
  assert.match(content, /^<https:\/\/discord\.com\/users\/123456789012345678\|TestUser>/);
});

test("renderDiscordJoinContent 無效 userId 回 null", () => {
  assert.equal(renderDiscordJoinContent("bad", "x"), null);
});

test("join 通知掛在 authorizeManagedIdentity 且含 KV 防刷", () => {
  const src = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  assert.match(src, /maybeNotifyDiscordJoinOncePerDay/);
  assert.match(src, /join_notify:\$\{day\}:\$\{userId\}/);
  assert.match(src, /joinNotifyConfigured/);
  assert.match(src, /DISCORD_JOIN_WEBHOOK/);
});

test("authorizeManagedIdentity 成功後觸發 join 通知", () => {
  const src = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  const start = src.indexOf("async function authorizeManagedIdentity");
  const end = src.indexOf("/** 正向會員快取 TTL", start);
  const body = src.slice(start, end);
  assert.match(body, /maybeNotifyDiscordJoinOncePerDay\(userId, displayName, env\)/);
});
