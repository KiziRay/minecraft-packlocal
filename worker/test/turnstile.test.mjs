import test from "node:test";
import assert from "node:assert/strict";

import {
  isAllowedLoopbackCallback,
  signPayload,
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
    await verifySignedPayload(`${token.slice(0, -1)}x`, env.TURNSTILE_PROOF_SECRET, "access"),
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
