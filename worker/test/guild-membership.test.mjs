import test from "node:test";
import assert from "node:assert/strict";

import {
  GUILD_OK_TTL_SECONDS,
  guildOkKvKey,
  readGuildOkCached,
  verifyGuildMembership,
  writeGuildOkCached,
} from "../src/index.js";

function mockUsage() {
  const store = new Map();
  return {
    store,
    USAGE: {
      async get(key) {
        return store.has(key) ? store.get(key) : null;
      },
      async put(key, value, opts) {
        store.set(key, value);
        this.lastPut = { key, value, opts };
      },
    },
  };
}

function jsonResponse(body, status = 200) {
  return {
    ok: status >= 200 && status < 300,
    status,
    async json() {
      return body;
    },
  };
}

test("guildOkKvKey 格式", () => {
  assert.equal(guildOkKvKey("123456789012345678"), "guild_ok:123456789012345678");
  assert.equal(GUILD_OK_TTL_SECONDS, 900);
});

test("快取命中略過 member-tier fetch", async () => {
  const { USAGE, store } = mockUsage();
  store.set(guildOkKvKey("111"), "1");
  let fetches = 0;
  const result = await verifyGuildMembership(
    "111",
    "https://cloud.zeitfrei.uk",
    { USAGE },
    async () => {
      fetches += 1;
      throw new Error("should not fetch");
    }
  );
  assert.equal(result.ok, true);
  assert.equal(fetches, 0);
  assert.equal(await readGuildOkCached("111", { USAGE }, "https://cloud.zeitfrei.uk"), true);
});

test("第一次 inGuild false、第二次 true 則放行並寫快取", async () => {
  const { USAGE, store } = mockUsage();
  let calls = 0;
  const result = await verifyGuildMembership(
    "222",
    "https://cloud.zeitfrei.uk",
    { USAGE },
    async () => {
      calls += 1;
      if (calls === 1) return jsonResponse({ inGuild: false });
      return jsonResponse({ inGuild: true });
    }
  );
  assert.equal(result.ok, true);
  assert.equal(calls, 2);
  assert.equal(store.get(guildOkKvKey("222")), "1");
  assert.equal(USAGE.lastPut.opts.expirationTtl, GUILD_OK_TTL_SECONDS);
});

test("兩次皆非會員 → guild_required", async () => {
  let calls = 0;
  const result = await verifyGuildMembership(
    "333",
    "https://cloud.zeitfrei.uk",
    {},
    async () => {
      calls += 1;
      return jsonResponse({ inGuild: false });
    }
  );
  assert.deepEqual(result, { ok: false, type: "guild_required" });
  assert.equal(calls, 2);
});

test("member-tier HTTP 失敗 → auth_unavailable（非 guild_required）", async () => {
  const result = await verifyGuildMembership(
    "444",
    "https://cloud.zeitfrei.uk",
    {},
    async () => jsonResponse({ error: "boom" }, 500)
  );
  assert.deepEqual(result, { ok: false, type: "auth_unavailable" });
});

test("writeGuildOkCached 寫入 USAGE TTL", async () => {
  const { USAGE, store } = mockUsage();
  await writeGuildOkCached("555", { USAGE }, "https://cloud.zeitfrei.uk");
  assert.equal(store.get(guildOkKvKey("555")), "1");
  assert.equal(USAGE.lastPut.opts.expirationTtl, 900);
});
