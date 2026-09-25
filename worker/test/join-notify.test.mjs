import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import {
  buildDiscordJoinPayload,
  discordAvatarUrl,
  renderDiscordJoinContent,
} from "../src/index.js";

test("discordAvatarUrl 有 hash 時用 CDN avatars", () => {
  const url = discordAvatarUrl("123456789012345678", "abcdef0123456789abcdef0123456789");
  assert.equal(
    url,
    "https://cdn.discordapp.com/avatars/123456789012345678/abcdef0123456789abcdef0123456789.png?size=128"
  );
});

test("discordAvatarUrl 動畫 hash 用 gif", () => {
  const url = discordAvatarUrl("123456789012345678", "a_abcdef0123456789abcdef01234567");
  assert.match(url, /\.gif\?size=128$/);
});

test("discordAvatarUrl 無 hash 用預設頭像", () => {
  const url = discordAvatarUrl("123456789012345678", "");
  assert.match(url, /^https:\/\/cdn\.discordapp\.com\/embed\/avatars\/\d\.png$/);
});

test("buildDiscordJoinPayload 含 author／fields／成員名", () => {
  const payload = buildDiscordJoinPayload("123456789012345678", "TestUser", "abcdef0123456789abcdef0123456789");
  assert.ok(payload);
  assert.equal(payload.content, undefined);
  assert.equal(payload.embeds.length, 1);
  const emb = payload.embeds[0];
  assert.equal(emb.title, "通過官方伺服器驗證");
  assert.equal(emb.description, "TestUser 開始使用 MCPL。");
  assert.equal(emb.author.name, "TestUser");
  assert.equal(emb.author.url, "https://discord.com/users/123456789012345678");
  assert.match(emb.author.icon_url, /cdn\.discordapp\.com\/avatars\//);
  assert.equal(emb.fields[0].name, "成員");
  assert.equal(emb.fields[0].value, "TestUser");
  assert.equal(emb.fields[1].name, "Discord ID");
  assert.equal(emb.fields[1].value, "`123456789012345678`");
  assert.ok(emb.thumbnail?.url);
  assert.equal(emb.color, 0x35c5c9);
  assert.equal(emb.footer.text, "模組包翻譯工具 · ZeitFrei");
  assert.ok(emb.timestamp);
});

test("buildDiscordJoinPayload 無顯示名時用 userId", () => {
  const payload = buildDiscordJoinPayload("123456789012345678", "");
  assert.equal(payload.embeds[0].author.name, "123456789012345678");
  assert.match(payload.embeds[0].description, /^123456789012345678 開始使用/);
  assert.equal(payload.embeds[0].fields[0].value, "123456789012345678");
});

test("buildDiscordJoinPayload 無效 userId 回 null", () => {
  assert.equal(buildDiscordJoinPayload("bad", "x"), null);
  assert.equal(renderDiscordJoinContent("bad", "x"), null);
});

test("join 通知掛在 authorizeManagedIdentity 且含 KV 防刷", () => {
  const src = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  assert.match(src, /maybeNotifyDiscordJoinOncePerDay/);
  assert.match(src, /join_notify:\$\{day\}:\$\{userId\}/);
  assert.match(src, /joinNotifyConfigured/);
  assert.match(src, /DISCORD_JOIN_WEBHOOK/);
  assert.match(src, /buildDiscordJoinPayload/);
  assert.match(src, /discordAvatarUrl/);
});

test("authorizeManagedIdentity 成功後觸發 join 通知並傳 avatar", () => {
  const src = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  const start = src.indexOf("async function authorizeManagedIdentity");
  const end = src.indexOf("/** 正向會員快取 TTL", start);
  const body = src.slice(start, end);
  assert.match(body, /maybeNotifyDiscordJoinOncePerDay\(userId, displayName, env, avatarHash\)/);
  assert.match(body, /account\.avatar/);
});
