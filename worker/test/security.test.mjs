import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { corsHeaders } from "../src/cors.mjs";
import { isPrivateOrReservedHost, isSafeOutboundUrl } from "../src/security.mjs";

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

test("TM/glossary contribute 需 gatedContribute", () => {
  const src = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  assert.match(src, /gatedContribute\(request, env, tmContribute\)/);
  assert.match(src, /gatedContribute\(request, env, glossaryContribute\)/);
  assert.match(src, /contribute:day:/);
});
