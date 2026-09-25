import test from "node:test";
import assert from "node:assert/strict";

import worker from "../src/index.js";
import {
  AUTH_SESSION_CACHE_TTL_SECONDS,
  readSessionCached,
  sessionCacheKey,
  writeSessionCached,
} from "../src/index.js";

// 這一組測試釘住本地模型下載變慢的根因修法：authorizeManagedIdentity() 原本每次
// 呼叫都對 AUTH_BASE_URL 做一次跨站 fetch——7.38 GB 的模型切成 32 MB 一段，
// 等於一次下載要對外打 ~230 次帳號驗證。快取讓同一個 session 在 TTL 內免再打。

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

test("sessionCacheKey 是固定長度的雜湊，不是原始 session", async () => {
  const key = await sessionCacheKey("a".repeat(100));
  assert.match(key, /^session_ok:[0-9a-f]{64}$/);
  // 同輸入同輸出，不同輸入不同輸出
  assert.equal(await sessionCacheKey("same"), await sessionCacheKey("same"));
  assert.notEqual(await sessionCacheKey("a"), await sessionCacheKey("b"));
  // KV 鍵有 512 bytes 上限；就算 session 到 8192 字元，雜湊後的鍵也遠低於上限
  assert.ok(key.length < 100);
});

test("寫入後可以讀回同一份帳號資訊，且帶 TTL", async () => {
  const usage = mockUsage();
  const account = { user_id: "123456789012345678", username: "測試" };
  await writeSessionCached("session-abc", account, usage);
  const got = await readSessionCached("session-abc", usage);
  assert.deepEqual(got, account);
  assert.equal(usage.USAGE.lastPut.opts.expirationTtl, AUTH_SESSION_CACHE_TTL_SECONDS);
});

test("沒快取過的 session 讀回 null，不是拋錯", async () => {
  const usage = mockUsage();
  assert.equal(await readSessionCached("never-seen", usage), null);
});

test("沒有 USAGE binding 時安全降級（讀回 null、寫入不拋錯）", async () => {
  await assert.doesNotReject(writeSessionCached("s", { user_id: "1" }, {}));
  assert.equal(await readSessionCached("s", {}), null);
  assert.equal(await readSessionCached("s", undefined), null);
});

test("同一個 session 連續兩次請求，第二次不再打外部帳號驗證", async () => {
  // 這是本地模型下載變慢的實際根因：7.38 GB 模型切成 32 MB 一段，
  // 舊版每一段都對外重打一次帳號驗證，等於一次下載打了約 230 次。
  const usage = mockUsage();
  const session = "s".repeat(50);
  let checkUploadCalls = 0;
  let memberTierCalls = 0;
  const origFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    const href = String(url?.url ?? url);
    if (href.includes("/api/check-upload")) {
      checkUploadCalls += 1;
      return { ok: true, status: 200, async json() { return { user_id: "123456789012345678" }; } };
    }
    if (href.includes("/api/member-tier/")) {
      memberTierCalls += 1;
      return { ok: true, status: 200, async json() { return { inGuild: true }; } };
    }
    return origFetch(url, init);
  };
  try {
    const req = () =>
      new Request("https://example.com/api/local-llm/manifest", {
        headers: { "x-zeitfrei-ai-protocol": "3", "x-zeitfrei-session": session },
      });
    const env = { MANAGED_AI_PROTOCOL: "3", USAGE: usage.USAGE };
    await worker.fetch(req(), env);
    await worker.fetch(req(), env);
    assert.equal(checkUploadCalls, 1, "第二次應該吃 session 快取，不再打帳號驗證");
    assert.equal(memberTierCalls, 1, "會員資格本來就有 900 秒快取，第二次也不該再打");
  } finally {
    globalThis.fetch = origFetch;
  }
});

test("不同 session 各自獨立，不會互相污染", async () => {
  const usage = mockUsage();
  await writeSessionCached("session-a", { user_id: "111" }, usage);
  await writeSessionCached("session-b", { user_id: "222" }, usage);
  assert.equal((await readSessionCached("session-a", usage)).user_id, "111");
  assert.equal((await readSessionCached("session-b", usage)).user_id, "222");
});
