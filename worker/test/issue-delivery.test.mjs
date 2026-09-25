// P0-09：回報「成功」不代表回報者看得到私人討論串。
//
// 舊版把 createPrivateThread 回傳的 reporterAdded 整個丟棄，一律回 { ok: true }。
// 於是「沒能把你加進討論串」和「你可以在討論串裡追後續」對使用者長得一模一樣。

import test from "node:test";
import assert from "node:assert/strict";

import {
  submitIssueThread,
  buildCaseId,
  idempotencyCacheKey,
  parseIssueBody,
  issueDayKey,
  taipeiYmd,
} from "../src/issue-thread.mjs";

const USER = "123456789012345678";
const valid = { summary: "翻譯結果不對或沒翻到", cause: "不確定", toolVersion: "1.0.9" };

function mockUsage(initial = {}) {
  const store = new Map(Object.entries(initial));
  return {
    store,
    async get(key) {
      return store.has(key) ? store.get(key) : null;
    },
    async put(key, value) {
      store.set(key, value);
    },
  };
}

function issueRequest(body = valid) {
  return new Request("https://x.invalid/api/issue-thread", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

function stubFetch(handler) {
  const original = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init });
    return handler(String(url), init);
  };
  return { calls, restore: () => (globalThis.fetch = original) };
}

/** 讓 createPrivateThread 走完整流程；addMember 的結果由 reporterOk 決定。 */
function stubDiscord({ reporterOk }) {
  return stubFetch(async (url, init) => {
    if (url.endsWith("/threads") && init?.method === "POST") {
      return new Response(JSON.stringify({ id: "thread-1" }), { status: 200 });
    }
    if (url.includes("/thread-members/")) {
      const isReporter = url.endsWith(`/thread-members/${USER}`);
      const ok = isReporter ? reporterOk : true;
      // 204 不得帶 body（Node 會丟 TypeError，而 addMember 的 catch 會把它變成 false）
      return ok ? new Response(null, { status: 204 }) : new Response("{}", { status: 403 });
    }
    if (url.includes("/messages")) {
      return new Response(JSON.stringify({ id: "msg-1" }), { status: 200 });
    }
    if (url.includes("/guilds/") && url.includes("/members/")) {
      return new Response(JSON.stringify({ user: { id: USER } }), { status: 200 });
    }
    return new Response("{}", { status: 200 });
  });
}

const env = (extra = {}) => ({
  USAGE: mockUsage(),
  DISCORD_BOT_TOKEN: "bot-token",
  ...extra,
});

test("回報者成功加入討論串 → delivered，並帶得出案件編號", async () => {
  const f = stubDiscord({ reporterOk: true });
  try {
    const res = await submitIssueThread(issueRequest(), env(), {
      userId: USER,
      displayName: "測試",
    });
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.ok, true);
    assert.equal(body.delivery, "delivered");
    assert.equal(body.reporterVisible, true);
    assert.match(body.caseId, /^MCPL-\d{8}-[0-9A-F]{6}$/);
    assert.equal(
      f.calls.some((call) => call.url.includes("/guilds/") && call.url.includes("/members/")),
      false,
      "直接建立私人串時，addMember 已是權限驗證；不得再先串行查會籍拖慢回覆"
    );
  } finally {
    f.restore();
  }
});

test("回報者加不進討論串 → partial，且不得宣稱可追蹤的私人討論已建立", async () => {
  const f = stubDiscord({ reporterOk: false });
  try {
    const res = await submitIssueThread(issueRequest(), env(), {
      userId: USER,
      displayName: "測試",
    });
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.delivery, "partial", "加不進去就不是 delivered");
    assert.equal(body.reporterVisible, false);
    // 這是本條 P0 的核心：訊息不得讓使用者以為有一條他看得到的討論串
    assert.ok(!/你看得到/.test(body.message), "partial 不得說使用者看得到討論串");
    assert.match(body.message, /不會收到串內回覆|案件編號/);
    // 必須給得出可以拿去對帳的東西
    assert.match(body.caseId, /^MCPL-/);
  } finally {
    f.restore();
  }
});

test("走 bot 通道時無法確認可見性 → 一律 partial，不假設看得到", async () => {
  const f = stubFetch(async () =>
    new Response(JSON.stringify({ ok: true, threadId: "1" }), { status: 200 })
  );
  try {
    const res = await submitIssueThread(
      issueRequest(),
      { USAGE: mockUsage(), BOT_API_URL: "https://bot.invalid", BOT_API_SECRET: "s" },
      { userId: USER, displayName: "測試" }
    );
    const body = await res.json();
    assert.equal(body.ok, true);
    assert.equal(body.delivery, "partial");
    assert.equal(body.channel, "via_bot");
  } finally {
    f.restore();
  }
});

test("同一把 idempotencyKey 重送只開一條討論串，且回同一個 caseId", async () => {
  const usage = mockUsage();
  const body = { ...valid, idempotencyKey: "retry-key-0001" };

  const first = stubDiscord({ reporterOk: true });
  let firstBody;
  try {
    const res = await submitIssueThread(issueRequest(body), env({ USAGE: usage }), {
      userId: USER,
      displayName: "測試",
    });
    firstBody = await res.json();
  } finally {
    first.restore();
  }

  // 第二次：如果去重沒生效，這個 stub 會再開一條新討論串
  let threadCreates = 0;
  const second = stubFetch(async (url, init) => {
    if (url.endsWith("/threads") && init?.method === "POST") threadCreates += 1;
    return new Response(JSON.stringify({ id: "thread-2" }), { status: 200 });
  });
  try {
    const res = await submitIssueThread(issueRequest(body), env({ USAGE: usage }), {
      userId: USER,
      displayName: "測試",
    });
    const secondBody = await res.json();
    assert.equal(threadCreates, 0, "重送不得再開一條討論串");
    assert.equal(secondBody.caseId, firstBody.caseId, "重送要拿到同一個案件編號");
  } finally {
    second.restore();
  }
});

test("每日額度只算一次，重送不重複扣", async () => {
  const usage = mockUsage();
  const body = { ...valid, idempotencyKey: "retry-key-0002" };
  for (let i = 0; i < 2; i++) {
    const f = stubDiscord({ reporterOk: true });
    try {
      await submitIssueThread(issueRequest(body), env({ USAGE: usage }), {
        userId: USER,
        displayName: "測試",
      });
    } finally {
      f.restore();
    }
  }
  assert.equal(usage.store.get(issueDayKey(USER, taipeiYmd())), "1");
});

test("idempotencyKey 格式不合就當沒帶，不因此擋下回報", () => {
  assert.equal(parseIssueBody({ ...valid, idempotencyKey: "short" }).idempotencyKey, null);
  assert.equal(parseIssueBody({ ...valid, idempotencyKey: "has space here" }).idempotencyKey, null);
  assert.equal(parseIssueBody(valid).idempotencyKey, null);
  assert.equal(parseIssueBody({ ...valid, idempotencyKey: "ok-key_12345678" }).ok, true);
  assert.equal(
    parseIssueBody({ ...valid, idempotencyKey: "ok-key_12345678" }).idempotencyKey,
    "ok-key_12345678"
  );
});

test("caseId 不含使用者身分，可安全對外引用", () => {
  const id = buildCaseId(USER, Date.parse("2026-09-04T00:00:00Z"));
  assert.match(id, /^MCPL-\d{8}-[0-9A-F]{6}$/);
  assert.ok(!id.includes(USER), "案件編號不得夾帶 Discord user id");
  // 同一使用者同一時刻要穩定（重送才拿得到同一個）
  assert.equal(id, buildCaseId(USER, Date.parse("2026-09-04T00:00:00Z")));
});

test("idempotency 快取鍵綁使用者，換人不會共用", () => {
  assert.notEqual(idempotencyCacheKey("111", "k"), idempotencyCacheKey("222", "k"));
});
