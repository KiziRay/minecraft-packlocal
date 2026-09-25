import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import worker from "../src/index.js";
import { corsHeaders } from "../src/cors.mjs";
import { isPrivateOrReservedHost, isSafeOutboundUrl } from "../src/security.mjs";

function mockUsage() {
  const store = new Map();
  return {
    store,
    async get(key) {
      return store.has(key) ? store.get(key) : null;
    },
    async put(key, value, opts) {
      store.set(key, value);
      this.lastPut = { key, value, opts };
    },
  };
}

function contributeRequest(headers = {}) {
  return new Request("https://example.com/tm/contribute", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...headers,
    },
    body: JSON.stringify({ items: [] }),
  });
}

test("corsHeaders 反射 allowlist Origin", () => {
  const req = new Request("https://example/", {
    headers: { Origin: "https://modpack-i18n.jolin34563.workers.dev" },
  });
  const h = corsHeaders(req);
  assert.equal(h["access-control-allow-origin"], "https://modpack-i18n.jolin34563.workers.dev");
  assert.equal(h.vary, "Origin");
});

test("corsHeaders 未知 Origin 不反射 *", () => {
  const req = new Request("https://example/", {
    headers: { Origin: "https://evil.example" },
  });
  const h = corsHeaders(req);
  assert.equal(h["access-control-allow-origin"], undefined);
});

test("corsHeaders 無 Origin 可用（桌面 WebView）", () => {
  const h = corsHeaders(new Request("https://example/"));
  assert.equal(h["access-control-allow-origin"], undefined);
  assert.ok(h["access-control-allow-methods"]);
});

test("isPrivateOrReservedHost 擋 localhost 與 RFC1918", () => {
  assert.equal(isPrivateOrReservedHost("127.0.0.1"), true);
  assert.equal(isPrivateOrReservedHost("10.0.0.1"), true);
  assert.equal(isPrivateOrReservedHost("192.168.1.1"), true);
  assert.equal(isPrivateOrReservedHost("169.254.169.254"), true);
  assert.equal(isPrivateOrReservedHost("8.8.8.8"), false);
});

test("isSafeOutboundUrl 拒絕內網 webhook", () => {
  assert.equal(isSafeOutboundUrl("http://127.0.0.1/hook"), false);
  assert.equal(isSafeOutboundUrl("https://discord.com/api/webhooks/1/2"), true);
});

test("TM/glossary contribute 走 gatedContribute，但不再硬擋 Discord", () => {
  const src = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  assert.match(src, /gatedContribute\(request, env, tmContribute\)/);
  assert.match(src, /gatedContribute\(request, env, glossaryContribute\)/);
  assert.match(src, /contribute:day:/);
  const start = src.indexOf("async function gatedContribute");
  const end = src.indexOf("// ───────────────────────── 共享翻譯記憶", start);
  const body = src.slice(start, end);
  assert.doesNotMatch(body, /if \(!access\.ok\) return access\.response/);
  assert.match(body, /clientIpBucket/);
});

test("共享庫貢獻缺 protocol 回 426 client_upgrade_required", async () => {
  const res = await worker.fetch(contributeRequest(), {
    USAGE: mockUsage(),
    TRANSLATIONS: {},
    MANAGED_AI_PROTOCOL: "3",
  });
  assert.equal(res.status, 426);
  const body = await res.json();
  assert.equal(body.error.type, "client_upgrade_required");
});

test("共享庫貢獻無 session 走 IP 日限", async () => {
  const env = {
    USAGE: mockUsage(),
    TRANSLATIONS: {},
    MANAGED_AI_PROTOCOL: "3",
    CONTRIBUTE_IP_DAILY_LIMIT: "2",
  };
  const headers = {
    "x-zeitfrei-ai-protocol": "3",
    "CF-Connecting-IP": "203.0.113.10",
  };
  assert.equal((await worker.fetch(contributeRequest(headers), env)).status, 200);
  assert.equal((await worker.fetch(contributeRequest(headers), env)).status, 200);
  const limited = await worker.fetch(contributeRequest(headers), env);
  assert.equal(limited.status, 429);
  assert.ok([...env.USAGE.store.keys()].some((key) => key.includes("ip:203.0.113.10")));
});

test("共享庫貢獻 session 驗證失敗會降級 IP，不擋貢獻", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => {
    throw new Error("auth unavailable");
  };
  try {
    const env = {
      USAGE: mockUsage(),
      TRANSLATIONS: {},
      MANAGED_AI_PROTOCOL: "3",
      CONTRIBUTE_IP_DAILY_LIMIT: "40",
    };
    const res = await worker.fetch(
      contributeRequest({
        "x-zeitfrei-ai-protocol": "3",
        "x-zeitfrei-session": "a".repeat(40),
        "CF-Connecting-IP": "203.0.113.11",
      }),
      env
    );
    assert.equal(res.status, 200);
    assert.ok([...env.USAGE.store.keys()].some((key) => key.includes("ip:203.0.113.11")));
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("共享庫貢獻有效 Discord session 走 user 日限", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url) => {
    if (String(url).includes("/api/check-upload")) {
      return new Response(JSON.stringify({ user_id: "123456789012345678", username: "tester" }), {
        headers: { "content-type": "application/json" },
      });
    }
    if (String(url).includes("/api/member-tier/")) {
      return new Response(JSON.stringify({ inGuild: true }), {
        headers: { "content-type": "application/json" },
      });
    }
    throw new Error("unexpected fetch");
  };
  try {
    const env = {
      USAGE: mockUsage(),
      TRANSLATIONS: {},
      MANAGED_AI_PROTOCOL: "3",
      CONTRIBUTE_USER_DAILY_LIMIT: "1",
      AUTH_BASE_URL: "https://auth.example",
    };
    const headers = {
      "x-zeitfrei-ai-protocol": "3",
      "x-zeitfrei-session": "a".repeat(40),
      "CF-Connecting-IP": "203.0.113.12",
    };
    assert.equal((await worker.fetch(contributeRequest(headers), env)).status, 200);
    assert.equal((await worker.fetch(contributeRequest(headers), env)).status, 429);
    assert.ok([...env.USAGE.store.keys()].some((key) => key.includes("user:123456789012345678")));
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("TM lookup 有 cache-control 且 per-IP 輕量節流", async () => {
  const env = {
    USAGE: mockUsage(),
    MANAGED_AI_PROTOCOL: "3",
    LOOKUP_IP_MINUTE_LIMIT: "1",
  };
  const req = () =>
    new Request("https://example.com/tm/lookup", {
      method: "POST",
      headers: { "content-type": "application/json", "CF-Connecting-IP": "203.0.113.20" },
      body: JSON.stringify({ items: [] }),
    });
  const first = await worker.fetch(req(), env);
  assert.equal(first.status, 200);
  assert.equal(first.headers.get("cache-control"), "private, max-age=60");
  const second = await worker.fetch(req(), env);
  assert.equal(second.status, 429);
});

test("共享資料內部上限與 parse error 會留下紀錄", () => {
  const src = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  assert.match(src, /GLOSSARY_CAP/);
  assert.match(src, /tmGlobalSoftCapExceeded/);
  assert.match(src, /globalSkipped/);
  assert.match(src, /recordTranslationReadError/);
  assert.match(src, /translations:parse_error/);
  assert.match(src, /tmGlobal:/);
});
