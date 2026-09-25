import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import worker from "../src/index.js";
import {
  looksLikeBotJsonError,
  buildContactStaffMessage,
  buildIssueEmbed,
  buildThreadName,
  ISSUE_DAILY_LIMIT,
  issueDayKey,
  parseIssueBody,
  resolveBotOrigins,
  submitIssueThread,
  taipeiYmd,
  buildIssueV2Message,
  checkGuildMembership,
  createPrivateThread,
  MCPL_DEV_USER_ID,
  MCPL_GUILD_ID,
  MCPL_REPORT_CHANNEL_ID,
} from "../src/issue-thread.mjs";

const valid = {
  summary: "翻譯結果不對或沒翻到",
  cause: "不確定",
  toolVersion: "1.0.9",
};

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

test("rejects illegal summary and cause", () => {
  assert.equal(parseIssueBody({ ...valid, summary: "聯絡站長" }).ok, false);
  assert.equal(parseIssueBody({ ...valid, cause: "隨便" }).ok, false);
});

test("rejects detail of 1–10 characters", () => {
  assert.equal(parseIssueBody({ ...valid, detail: "1234567890" }).ok, false);
  assert.equal(parseIssueBody({ ...valid, detail: "十個字不到" }).ok, false);
  assert.equal(parseIssueBody({ ...valid, detail: "這段詳細說明超過十個字了" }).ok, true);
  assert.equal(parseIssueBody({ ...valid, detail: "" }).ok, true);
});

test("submitIssueThread 401-equivalent when userId missing", async () => {
  const req = new Request("https://example.test/api/issue-thread", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(valid),
  });
  const res = await submitIssueThread(req, {}, { userId: "", displayName: "x" });
  assert.equal(res.status, 401);
});

test("submitIssueThread 429 after daily limit", async () => {
  const userId = "123456789012345678";
  const usage = mockUsage({ [issueDayKey(userId, taipeiYmd())]: String(ISSUE_DAILY_LIMIT) });
  const req = new Request("https://example.test/api/issue-thread", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(valid),
  });
  const res = await submitIssueThread(
    req,
    { USAGE: usage, BOT_API_URL: "https://videobot.zeitfrei.uk", BOT_API_SECRET: "s" },
    { userId, displayName: "測試" }
  );
  assert.equal(res.status, 429);
});

// ⚠️ 這條測試以前是反過來寫的（斷言必須打 contact-staff、不得打 mcpl-issue-thread），
// 等於把 bug 寫成規格——回報功能因此連修三輪都沒好，每一輪修的程式碼都沒被呼叫到。
// 別再翻回去。
test("備援要打 MCPL 自己的端點，不是舊的聯絡站長", async () => {
  const calls = [];
  const orig = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), body: JSON.parse(init.body) });
    return new Response(JSON.stringify({ ok: true, threadId: "1" }), { status: 200 });
  };
  try {
    const userId = "123456789012345678";
    const usage = mockUsage();
    const req = new Request("https://example.test/api/issue-thread", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ ...valid, detail: "這段詳細說明超過十個字了" }),
    });
    const res = await submitIssueThread(
      req,
      { USAGE: usage, BOT_API_URL: "https://videobot.zeitfrei.uk", BOT_API_SECRET: "secret" },
      { userId, displayName: "測試" }
    );
    assert.equal(res.status, 200);
    assert.equal(calls.length, 1);
    assert.equal(calls[0].url, "https://videobot.zeitfrei.uk/api/mcpl-issue-thread");
    assert.equal(calls[0].url.includes("contact-staff"), false);
    assert.equal(calls[0].body.userId, userId);
    // payload 改成 parseMcplIssueBody 要的結構化欄位，不是舊的 { message }
    assert.equal(calls[0].body.summary, "翻譯結果不對或沒翻到");
    assert.ok("cause" in calls[0].body && "toolVersion" in calls[0].body);
    assert.equal(usage.store.get(issueDayKey(userId, taipeiYmd())), "1");
  } finally {
    globalThis.fetch = orig;
  }
});

test("index POST /api/issue-thread 無 session 回 401", async () => {
  const res = await worker.fetch(
    new Request("https://example.com/api/issue-thread", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-zeitfrei-ai-protocol": "3",
      },
      body: JSON.stringify(valid),
    }),
    { MANAGED_AI_PROTOCOL: "3" }
  );
  assert.equal(res.status, 401);
  const body = await res.json();
  assert.equal(body.error?.type, "login_required");
});

test("index POST /api/issue-thread 未入伺服器回 403", async () => {
  const orig = globalThis.fetch;
  globalThis.fetch = async (url) => {
    if (String(url).includes("/api/check-upload")) {
      return new Response(JSON.stringify({ user_id: "123456789012345678", username: "tester" }), {
        headers: { "content-type": "application/json" },
      });
    }
    if (String(url).includes("/api/member-tier/")) {
      return new Response(JSON.stringify({ inGuild: false }), {
        headers: { "content-type": "application/json" },
      });
    }
    throw new Error(`unexpected fetch ${url}`);
  };
  try {
    const res = await worker.fetch(
      new Request("https://example.com/api/issue-thread", {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-zeitfrei-ai-protocol": "3",
          "x-zeitfrei-session": "a".repeat(40),
        },
        body: JSON.stringify(valid),
      }),
      { MANAGED_AI_PROTOCOL: "3", AUTH_BASE_URL: "https://auth.example", USAGE: mockUsage() }
    );
    assert.equal(res.status, 403);
    const body = await res.json();
    assert.equal(body.error?.type, "guild_required");
  } finally {
    globalThis.fetch = orig;
  }
});

test("Azatosz 回 403 轉成 guild_required 且不記次數", async () => {
  const orig = globalThis.fetch;
  globalThis.fetch = async () => new Response(JSON.stringify({ error: "not in guild" }), { status: 403 });
  try {
    const userId = "123456789012345678";
    const usage = mockUsage();
    const req = new Request("https://example.test/api/issue-thread", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(valid),
    });
    const res = await submitIssueThread(
      req,
      { USAGE: usage, BOT_API_URL: "https://videobot.zeitfrei.uk", BOT_API_SECRET: "secret" },
      { userId, displayName: "測試" }
    );
    assert.equal(res.status, 403);
    assert.equal(usage.store.has(issueDayKey(userId, taipeiYmd())), false);
  } finally {
    globalThis.fetch = orig;
  }
});

test("issue-thread 走 mcpl-issue-thread，secret 不進 wrangler vars", () => {
  const indexSrc = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  const issueSrc = readFileSync(new URL("../src/issue-thread.mjs", import.meta.url), "utf8");
  const wrangler = readFileSync(new URL("../wrangler.toml", import.meta.url), "utf8");
  assert.match(indexSrc, /pathname === "\/api\/issue-thread"/);
  assert.match(indexSrc, /submitIssueThread/);
  // 反過來：必須打 MCPL 端點。contact-staff 只允許出現在說明為什麼不能用它的註解裡。
  assert.equal(issueSrc.includes("/api/mcpl-issue-thread"), true);
  assert.equal(/fetch\(`\$\{origin\}\/api\/contact-staff`/.test(issueSrc), false);
  assert.equal(issueSrc.includes("contact_threads"), false);
  assert.match(wrangler, /LATEST_VERSION = "1\.0\.8\.11"/);
  const varsBlock = wrangler.split("[vars]")[1]?.split("\n[[")[0] || "";
  assert.equal(/^\s*BOT_API_(URL|SECRET)\s*=/m.test(varsBlock), false);
});

function issueRequest() {
  return new Request("https://example.test/api/issue-thread", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(valid),
  });
}

/** 攔截 fetch，記錄每次呼叫，並依 URL 決定回應。 */
function stubFetch(handler) {
  const orig = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), body: init?.body ? JSON.parse(init.body) : null });
    return handler(String(url), init);
  };
  return { calls, restore: () => (globalThis.fetch = orig) };
}

test("resolveBotOrigins 只留 https：內網與任何明文 HTTP 都丟棄", () => {
  // P0-08：這些 origin 之後都會收到 Authorization: Bearer BOT_API_SECRET。
  // 舊版會保留一個硬編的 http://<IP>:3001 備援，等於把服務憑證用明文送出去。
  const origins = resolveBotOrigins({
    BOT_API_URL: "https://videobot.zeitfrei.uk",
    BOT_API_FALLBACK_URL: "http://127.0.0.1:3001",
  });
  assert.deepEqual(origins, ["https://videobot.zeitfrei.uk"]);

  // 公網 IP 的明文 HTTP 一樣要擋——它不是內網，靠 isSafeOutboundUrl 擋不掉
  assert.deepEqual(
    resolveBotOrigins({ BOT_API_URL: "http://23.146.248.124:3001" }),
    [],
    "明文 HTTP 不得承載 Bearer 憑證"
  );

  // 兩個都是 https 才會同時保留（故障轉移仍可用）
  assert.deepEqual(
    resolveBotOrigins({
      BOT_API_URL: "https://a.invalid",
      BOT_API_FALLBACK_URL: "https://b.invalid",
    }),
    ["https://a.invalid", "https://b.invalid"]
  );

  // 完全沒設定就是沒有通道，不會偷偷補一個進來
  assert.deepEqual(resolveBotOrigins({}), []);
});

test("送 Bearer 的 fetch 只會打到 https", async () => {
  const seen = [];
  const f = stubFetch(async (url, init) => {
    seen.push({ url, auth: init?.headers?.Authorization || "" });
    return new Response(JSON.stringify({ ok: true, threadId: "1" }), { status: 200 });
  });
  try {
    await submitIssueThread(
      issueRequest(),
      {
        USAGE: mockUsage(),
        BOT_API_URL: "http://insecure.invalid",
        BOT_API_FALLBACK_URL: "https://secure.invalid",
        BOT_API_SECRET: "secret",
      },
      { userId: "123456789012345678", displayName: "測試" }
    );
    for (const call of seen) {
      if (call.auth.startsWith("Bearer ")) {
        assert.ok(call.url.startsWith("https://"), `Bearer 送到了非 https：${call.url}`);
      }
    }
    assert.ok(
      seen.every((c) => !c.url.startsWith("http://insecure.invalid")),
      "明文 origin 完全不該被打到"
    );
  } finally {
    f.restore();
  }
});

test("Tunnel 502 時改打 VPS 後備仍算送出", async () => {
  const f = stubFetch(async (url) => {
    if (url.includes("videobot.invalid")) return new Response("error code: 502", { status: 502 });
    if (url.includes("vps-fallback.invalid")) {
      return new Response(JSON.stringify({ ok: true, threadId: "1" }), { status: 200 });
    }
    return new Response("no", { status: 404 });
  });
  try {
    const userId = "123456789012345678";
    const usage = mockUsage();
    const res = await submitIssueThread(
      issueRequest(),
      {
        USAGE: usage,
        BOT_API_URL: "https://videobot.invalid",
        BOT_API_FALLBACK_URL: "https://vps-fallback.invalid",
        BOT_API_SECRET: "secret",
      },
      { userId, displayName: "測試" }
    );
    assert.equal(res.status, 200);
    assert.ok(f.calls.some((c) => c.url.includes("vps-fallback.invalid/api/mcpl-issue-thread")));
    assert.equal(usage.store.get(issueDayKey(userId, taipeiYmd())), "1");
  } finally {
    f.restore();
  }
});

test("bot 掛掉且沒有 webhook → 503 說明通道離線，不記次數", async () => {
  const f = stubFetch(async () => {
    throw new Error("timeout");
  });
  try {
    const userId = "123456789012345678";
    const usage = mockUsage();
    const res = await submitIssueThread(
      issueRequest(),
      {
        USAGE: usage,
        BOT_API_URL: "https://videobot.zeitfrei.uk",
        // 備援不再由程式硬編（那個硬編值是明文 HTTP，已移除）；
        // 要有第二條路就必須自己設定，而且一樣得是 https。
        BOT_API_FALLBACK_URL: "https://vps-fallback.invalid",
        BOT_API_SECRET: "secret",
      },
      { userId, displayName: "測試" }
    );
    assert.equal(res.status, 503);
    const body = await res.json();
    assert.equal(body.error?.type, "bot_unavailable");
    // 訊息現在附上實際打過的位址：這一輪的教訓就是「打錯端點」完全沒有跡象可查
    assert.match(body.error?.message || "", /連不上/);
    assert.match(body.error?.message || "", /mcpl-issue-thread/);
    assert.ok(f.calls.length >= 2, "主網域失敗後要改打下一個 origin");
    assert.equal(usage.store.has(issueDayKey(userId, taipeiYmd())), false, "沒送出去就不該吃額度");
  } finally {
    f.restore();
  }
});

test("第一個 origin 502、下一個成功仍算送出", async () => {
  let n = 0;
  const f = stubFetch(async () => {
    n += 1;
    if (n < 2) return new Response("error code: 502", { status: 502 });
    return new Response(JSON.stringify({ ok: true, threadId: "1" }), { status: 200 });
  });
  try {
    const userId = "123456789012345678";
    const usage = mockUsage();
    const res = await submitIssueThread(
      issueRequest(),
      {
        USAGE: usage,
        BOT_API_URL: "https://videobot.invalid",
        BOT_API_FALLBACK_URL: "https://vps-fallback.invalid",
        BOT_API_SECRET: "secret",
      },
      { userId, displayName: "測試" }
    );
    assert.equal(res.status, 200);
    assert.ok(n >= 2);
    assert.equal(usage.store.get(issueDayKey(userId, taipeiYmd())), "1");
  } finally {
    f.restore();
  }
});

test("bot 端點失敗時即使有 webhook 也不得當討論串成功", async () => {
  const f = stubFetch(async (url) => {
    if (url.includes("mcpl-issue-thread")) return new Response("not found", { status: 404 });
    return new Response(JSON.stringify({ id: "1" }), { status: 200 });
  });
  try {
    const userId = "123456789012345678";
    const usage = mockUsage();
    const res = await submitIssueThread(
      issueRequest(),
      {
        USAGE: usage,
        BOT_API_URL: "https://videobot.zeitfrei.uk",
        BOT_API_SECRET: "secret",
        DISCORD_REPORT_WEBHOOK: "https://discord.com/api/webhooks/1/abc",
        DISCORD_ISSUE_THREAD_WEBHOOK: "https://discord.com/api/webhooks/1/dedicated",
      },
      { userId, displayName: "測試" }
    );
    assert.equal(res.status, 503);
    const body = await res.json();
    assert.equal(body.error?.type, "thread_not_available");
    assert.equal(
      f.calls.some((c) => c.url.includes("webhooks")),
      false,
      "不得改送 webhook 到別的頻道"
    );
    assert.equal(usage.store.has(issueDayKey(userId, taipeiYmd())), false);
  } finally {
    f.restore();
  }
});

test("BOT_API_URL 尾端 /api 或引號仍打得到正確端點", async () => {
  const calls = [];
  const orig = globalThis.fetch;
  globalThis.fetch = async (url) => {
    calls.push(String(url));
    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  };
  try {
    const res = await submitIssueThread(
      issueRequest(),
      {
        USAGE: mockUsage(),
        BOT_API_URL: '"https://videobot.zeitfrei.uk/api/"',
        BOT_API_SECRET: '"secret"',
      },
      { userId: "123456789012345678", displayName: "測試" }
    );
    assert.equal(res.status, 200);
    assert.deepEqual(calls, ["https://videobot.zeitfrei.uk/api/mcpl-issue-thread"]);
  } finally {
    globalThis.fetch = orig;
  }
});

test("webhook 指向一般文字頻道也不得當討論串成功", async () => {
  const f = stubFetch(async (url, init) => {
    if (url.includes("mcpl-issue-thread")) return new Response("", { status: 404 });
    const body = JSON.parse(init.body);
    if (body.thread_name) {
      return new Response(JSON.stringify({ message: "Invalid Form Body" }), { status: 400 });
    }
    return new Response(JSON.stringify({ id: "1" }), { status: 200 });
  });
  try {
    const usage = mockUsage();
    const res = await submitIssueThread(
      issueRequest(),
      { USAGE: usage, DISCORD_REPORT_WEBHOOK: "https://discord.com/api/webhooks/1/abc" },
      { userId: "123456789012345678", displayName: "測試" }
    );
    assert.equal(res.status, 503);
    const body = await res.json();
    assert.equal(body.ok, false);
    assert.equal(body.error?.type, "thread_not_available");
    assert.equal(usage.store.has(issueDayKey("123456789012345678", taipeiYmd())), false);
  } finally {
    f.restore();
  }
});

test("沒有 bot secret、只有文字頻道 webhook 不得回討論串成功", async () => {
  const f = stubFetch(async (_url, init) => {
    const body = init?.body ? JSON.parse(init.body) : {};
    if (body.thread_name) {
      return new Response(JSON.stringify({ message: "Invalid Form Body" }), { status: 400 });
    }
    return new Response(JSON.stringify({ id: "1" }), { status: 200 });
  });
  try {
    const usage = mockUsage();
    const res = await submitIssueThread(
      issueRequest(),
      { USAGE: usage, DISCORD_REPORT_WEBHOOK: "https://discord.com/api/webhooks/1/abc" },
      { userId: "123456789012345678", displayName: "測試" }
    );
    assert.equal(res.status, 503);
    const body = await res.json();
    assert.equal(body.error?.type, "thread_not_available");
    const text = JSON.stringify(body);
    assert.equal(text.includes("discord.com/api/webhooks"), false);
    assert.equal(text.includes("BOT_API"), false);
    assert.equal(usage.store.has(issueDayKey("123456789012345678", taipeiYmd())), false);
  } finally {
    f.restore();
  }
});

test("回應與錯誤訊息不得洩漏 webhook 或 secret", async () => {
  const f = stubFetch(async () => new Response("", { status: 500 }));
  try {
    const res = await submitIssueThread(
      issueRequest(),
      {
        USAGE: mockUsage(),
        BOT_API_SECRET: "super-secret-value",
        DISCORD_REPORT_WEBHOOK: "https://discord.com/api/webhooks/1/tokenvalue",
      },
      { userId: "123456789012345678", displayName: "測試" }
    );
    const text = await res.text();
    assert.ok(!text.includes("super-secret-value"));
    assert.ok(!text.includes("tokenvalue"));
    assert.ok(!text.includes("discord.com/api/webhooks"));
  } finally {
    f.restore();
  }
});

// ── 討論串建立 ────────────────────────────────────────────────────────────
// 備援路徑：Azatosz `/api/mcpl-issue-thread`（MCPL 專用私人討論串）。webhook 不算成功。

test("contact-staff 訊息含概要與版本，不含 mention", () => {
  const msg = buildContactStaffMessage({
    summary: "套用後遊戲異常",
    cause: "不確定",
    detail: "@everyone 這段詳細說明超過十個字了",
    toolVersion: "1.0.9",
  });
  assert.match(msg, /套用後遊戲異常/);
  assert.match(msg, /1\.0\.9/);
  assert.ok(msg.length <= 1000);
});

test("討論串標題含問題概要與回報者，且不含使用者自由文字", () => {
  const name = buildThreadName({
    username: "玩家A",
    summary: "套用後遊戲異常",
    detail: "@everyone 這段不可以出現在標題",
  });
  assert.ok(name.includes("套用後遊戲異常"));
  assert.ok(name.includes("玩家A"));
  assert.ok(!name.includes("@everyone"), "自由文字不得進標題");
  assert.ok(name.length <= 90, "Discord 標題長度上限");
});

test("標題在缺欄位時仍可用", () => {
  const name = buildThreadName({});
  assert.ok(name.length > 0);
  assert.ok(!name.includes("undefined"));
});

test("embed 帶齊欄位且截斷過長詳述", () => {
  const embed = buildIssueEmbed({
    summary: "翻譯結果不對或沒翻到",
    cause: "剛更新工具或整合包",
    detail: "x".repeat(5000),
    toolVersion: "1.0.9",
    userId: "123456789",
  });
  const detail = embed.fields.find((f) => f.name === "詳細說明");
  assert.ok(detail.value.length <= 1000, "詳述必須截斷");
  assert.ok(embed.fields.some((f) => f.value.includes("123456789")));
  const empty = buildIssueEmbed({});
  assert.ok(empty.fields.every((f) => typeof f.value === "string" && f.value.length > 0));
});

test("中間層擋下的 403 要換下一個位址，不能當成使用者沒加入伺服器", async () => {
  // 實測背景：Tunnel（videobot.zeitfrei.uk）當時回 502／403，而備援的 VPS 位址
  // 其實是活的。舊版一收到 403 就直接判定 guild_required 並中止整條重試鏈，
  // 使用者因此被叫去做「重新加入伺服器」這件完全沒用的事。
  assert.equal(looksLikeBotJsonError('{"error":"請先加入官方 Discord 伺服器"}'), true);
  // Cloudflare／反向代理擋下來的是 HTML 或空白 → 不是 bot 講的
  assert.equal(looksLikeBotJsonError("<!DOCTYPE html><html>403 Forbidden</html>"), false);
  assert.equal(looksLikeBotJsonError(""), false);
  assert.equal(looksLikeBotJsonError("Forbidden"), false);
  // JSON 但沒有 error 欄位也不算
  assert.equal(looksLikeBotJsonError('{"ok":true}'), false);
});

test("回報者就是開發人員時 mentions 不可以重複", () => {
  // 實測 400：{"code":50035,"errors":{"allowed_mentions":{"users":{"1":
  // {"_errors":[{"code":"SET_TYPE_ALREADY_CONTAINS_VALUE"}]}}}}
  // 站長本人就是開發人員（同一個 Discord id），[userId, devId] 因此有重複，
  // Discord 直接把整則訊息打回。討論串開得出來，只有最後一則訊息送不出去。
  const same = "305389581304463360";
  const msg = buildIssueV2Message(
    { userId: same, username: "jolin", summary: "測試", cause: "測試" },
    { devUserId: same }
  );
  assert.deepEqual(msg.allowed_mentions.users, [same], "重複的 id 必須去掉");
  assert.equal(msg.content, `<@${same}>`, "內容也只該標一次");

  // 一般使用者（跟開發人員不同人）兩個都要在
  const other = "999999999999999999";
  const two = buildIssueV2Message(
    { userId: other, username: "player", summary: "測試", cause: "測試" },
    { devUserId: same }
  );
  assert.deepEqual(two.allowed_mentions.users, [other, same]);
  assert.equal(two.content, `<@${other}> <@${same}>`);
});
