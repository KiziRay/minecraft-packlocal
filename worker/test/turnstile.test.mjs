import test from "node:test";
import assert from "node:assert/strict";

import {
  callSiteverify,
  describeSiteverifyFailure,
  isAllowedLoopbackCallback,
  prepareSiteverifyRequest,
  signPayload,
  truncateUpstreamBody,
  turnstileConfigured,
  verifySignedPayload,
  verifyTurnstileAccess,
} from "../src/turnstile.mjs";

const env = {
  TURNSTILE_SITE_KEY: "site-key",
  TURNSTILE_SECRET_KEY: "site-secret",
  TURNSTILE_PROOF_SECRET: "a".repeat(48),
};

test("只接受指定連接埠的 loopback callback", () => {
  assert.equal(isAllowedLoopbackCallback("http://127.0.0.1:19431/turnstile-callback"), true);
  assert.equal(isAllowedLoopbackCallback("http://127.0.0.1:19440/turnstile-callback"), true);
  assert.equal(isAllowedLoopbackCallback("http://localhost:19431/turnstile-callback"), false);
  assert.equal(isAllowedLoopbackCallback("https://127.0.0.1:19431/turnstile-callback"), false);
  assert.equal(isAllowedLoopbackCallback("http://127.0.0.1:19430/turnstile-callback"), false);
  assert.equal(isAllowedLoopbackCallback("http://127.0.0.1:19431/turnstile-callback?x=1"), false);
});

test("缺少任一 Cloudflare secret 時採拒絕存取", () => {
  assert.equal(turnstileConfigured(env), true);
  assert.equal(turnstileConfigured({ ...env, TURNSTILE_SECRET_KEY: "" }), false);
  assert.equal(turnstileConfigured({ ...env, TURNSTILE_PROOF_SECRET: "short" }), false);
});

test("turnstileMissingNames 只回缺項名稱", async () => {
  const { turnstileMissingNames } = await import("../src/turnstile.mjs");
  assert.deepEqual(turnstileMissingNames(env), []);
  assert.deepEqual(turnstileMissingNames({ ...env, TURNSTILE_SECRET_KEY: "  " }), [
    "TURNSTILE_SECRET_KEY",
  ]);
});

test("HMAC token 可驗簽且竄改後失效", async () => {
  const payload = {
    v: 1,
    kind: "access",
    uid: "123456789",
    nonce: "1234567890abcdef",
    exp: Math.floor(Date.now() / 1000) + 60,
  };
  const token = await signPayload(payload, env.TURNSTILE_PROOF_SECRET);
  assert.deepEqual(
    await verifySignedPayload(token, env.TURNSTILE_PROOF_SECRET, "access"),
    payload
  );
  assert.equal(
    await verifySignedPayload(
      `${token.split(".")[0]}.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA`,
      env.TURNSTILE_PROOF_SECRET,
      "access"
    ),
    null
  );
});

test("通行憑證綁定 Discord user id", async () => {
  const proof = await signPayload(
    {
      v: 1,
      kind: "access",
      uid: "123456789",
      nonce: "1234567890abcdef",
      exp: Math.floor(Date.now() / 1000) + 60,
    },
    env.TURNSTILE_PROOF_SECRET
  );
  assert.equal((await verifyTurnstileAccess(proof, env, "123456789")).ok, true);
  assert.equal((await verifyTurnstileAccess(proof, env, "987654321")).ok, false);
});

test("過期憑證不會被接受", async () => {
  const proof = await signPayload(
    {
      v: 1,
      kind: "access",
      uid: "123456789",
      nonce: "1234567890abcdef",
      exp: Math.floor(Date.now() / 1000) - 1,
    },
    env.TURNSTILE_PROOF_SECRET
  );
  assert.equal((await verifyTurnstileAccess(proof, env, "123456789")).ok, false);
});

test("Siteverify 失敗會回可行動文案而非暫時無法使用", () => {
  const invalid = describeSiteverifyFailure(
    { success: false, "error-codes": ["invalid-input-secret"] },
    "modpack-i18n.jolin34563.workers.dev"
  );
  assert.equal(invalid.title, "安全驗證未通過");
  assert.match(invalid.message, /invalid-input-secret/);
  assert.equal(invalid.message.includes("暫時無法使用"), false);

  const host = describeSiteverifyFailure(
    {
      success: true,
      action: "managed-ai",
      hostname: "evil.example",
    },
    "modpack-i18n.jolin34563.workers.dev"
  );
  assert.equal(host.title, "安全驗證網域不符");
  assert.match(host.message, /evil\.example/);

  const ok = describeSiteverifyFailure(
    {
      success: true,
      action: "managed-ai",
      hostname: "modpack-i18n.jolin34563.workers.dev",
    },
    "modpack-i18n.jolin34563.workers.dev"
  );
  assert.equal(ok, null);
});

test("空 secret／token 不打上游", () => {
  const missingSecret = prepareSiteverifyRequest({ response: "tok" });
  assert.equal(missingSecret.ok, false);
  assert.match(missingSecret.message, /TURNSTILE_SECRET_KEY/);

  const missingToken = prepareSiteverifyRequest({ secret: "sec" });
  assert.equal(missingToken.ok, false);
  assert.match(missingToken.message, /cf-turnstile-response/);

  const ready = prepareSiteverifyRequest({
    secret: "sec",
    response: "tok",
    remoteip: "1.2.3.4",
  });
  assert.equal(ready.ok, true);
  assert.equal(ready.headers["content-type"], "application/x-www-form-urlencoded");
  assert.match(ready.body, /secret=sec/);
  assert.match(ready.body, /response=tok/);
  assert.match(ready.body, /remoteip=1\.2\.3\.4/);
});

test("上游 HTTP 400 仍解析 JSON（invalid-input-secret）", async () => {
  const fetchImpl = async () =>
    new Response(
      JSON.stringify({
        success: false,
        "error-codes": ["invalid-input-secret"],
      }),
      { status: 400, headers: { "content-type": "application/json" } }
    );
  const result = await callSiteverify(
    { secret: "sec", response: "tok" },
    fetchImpl
  );
  assert.equal(result.ok, true);
  assert.equal(result.outcome.success, false);
  assert.deepEqual(result.outcome["error-codes"], ["invalid-input-secret"]);
  const failure = describeSiteverifyFailure(
    result.outcome,
    "modpack-i18n.jolin34563.workers.dev"
  );
  assert.match(failure.message, /invalid-input-secret/);
});

test("上游非 JSON 錯誤會附截斷 body", async () => {
  const fetchImpl = async () =>
    new Response("Bad Gateway upstream html page ".repeat(20), { status: 502 });
  const result = await callSiteverify(
    { secret: "sec", response: "tok" },
    fetchImpl
  );
  assert.equal(result.ok, false);
  assert.match(result.message, /上游 HTTP 502/);
  assert.match(result.message, /body：/);
  assert.equal(result.message.includes("暫時無法使用"), false);
});

test("截斷上游 body 會遮罩 secret=", () => {
  const out = truncateUpstreamBody("secret=super-secret-value&other=1");
  assert.match(out, /secret=\[redacted\]/);
  assert.equal(out.includes("super-secret-value"), false);
});
