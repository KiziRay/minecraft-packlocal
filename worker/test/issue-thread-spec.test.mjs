/**
 * 問題回報流程的規格測試。
 *
 * 站長明確指定的六個步驟：
 *  1. 使用者用工具登入 Discord
 *  2. 選回報項目＋選填說明
 *  3. 工具把內容送到後端
 *  4. 後端拿內容與 Discord user id 比對會籍
 *  5. 在 308120017201922048 的 609381371390984202 頻道開**私人**討論串
 *  6. 討論串內用 Components V2 標記回報者與開發人員 305389581304463360，附上內容
 *
 * 這個檔守住 4～6。1～3 屬於工具端與 OAuth，在 Rust 那邊測。
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  buildIssueV2Message,
  checkGuildMembership,
  createPrivateThread,
  submitIssueThread,
  BOT_ISSUE_PATH,
  MCPL_DEV_USER_ID,
  MCPL_GUILD_ID,
  MCPL_REPORT_CHANNEL_ID,
} from "../src/issue-thread.mjs";

const REPORT = {
  userId: "111222333444555666",
  username: "測試玩家",
  summary: "翻譯結果不對或沒翻到",
  cause: "剛更新工具或整合包",
  detail: "任務書的說明整段都還是英文，重跑一次也一樣。",
  toolVersion: "1.0.9",
  membershipVerified: true,
};

/** 依序回傳預先排好的假回應，並記錄每一次呼叫。 */
function fakeDiscord(responses) {
  const calls = [];
  const original = globalThis.fetch;
  globalThis.fetch = async (url, init = {}) => {
    const entry = { url: String(url), method: init.method || "GET", body: init.body };
    calls.push(entry);
    const key = Object.keys(responses).find((k) => entry.url.includes(k));
    const r = key ? responses[key] : { ok: true, status: 200, json: {} };
    const spec = typeof r === "function" ? r(entry) : r;
    return {
      ok: spec.ok !== false,
      status: spec.status || (spec.ok === false ? 500 : 200),
      json: async () => spec.json || {},
      text: async () => JSON.stringify(spec.json || {}),
    };
  };
  return {
    calls,
    restore() {
      globalThis.fetch = original;
    },
  };
}

test("步驟 5：討論串開在指定伺服器的指定頻道，而且是私人討論串", async () => {
  assert.equal(MCPL_GUILD_ID, "308120017201922048");
  assert.equal(MCPL_REPORT_CHANNEL_ID, "609381371390984202");

  const fake = fakeDiscord({
    "/threads": { json: { id: "999000111" } },
  });
  try {
    const out = await createPrivateThread({ DISCORD_BOT_TOKEN: "tok" }, REPORT);
    assert.equal(out.ok, true);
    const create = fake.calls.find((c) => c.url.endsWith("/threads") && c.method === "POST");
    assert.ok(create, "必須有一次建立討論串的呼叫");
    assert.ok(
      create.url.includes(`/channels/${MCPL_REPORT_CHANNEL_ID}/threads`),
      "討論串要開在站長指定的頻道"
    );
    const body = JSON.parse(create.body);
    assert.equal(body.type, 12, "type 12 = 私人討論串；改成別的就變公開了");
    assert.equal(body.invitable, false);
  } finally {
    fake.restore();
  }
});

test("步驟 5：開發人員一定要被加進私人討論串，加不進去就不能報成功", async () => {
  // 私人討論串只有成員看得到。少加開發人員的話，回報會躺在沒人看得到的地方，
  // 使用者卻收到「已送出」——比直接失敗更糟，因為沒有人會發現。
  const ok = fakeDiscord({ "/threads": { json: { id: "999000111" } } });
  try {
    await createPrivateThread({ DISCORD_BOT_TOKEN: "tok" }, REPORT);
    const added = ok.calls
      .filter((c) => c.url.includes("/thread-members/") && c.method === "PUT")
      .map((c) => c.url.split("/thread-members/")[1]);
    assert.ok(added.includes(REPORT.userId), "回報者要加進去才看得到後續回覆");
    assert.ok(added.includes(MCPL_DEV_USER_ID), "開發人員要加進去才收得到回報");
  } finally {
    ok.restore();
  }

  const devFails = fakeDiscord({
    "/threads": { json: { id: "999000111" } },
    [`/thread-members/${MCPL_DEV_USER_ID}`]: { ok: false, status: 403 },
  });
  try {
    const out = await createPrivateThread({ DISCORD_BOT_TOKEN: "tok" }, REPORT);
    assert.equal(out.ok, false, "開發人員加不進去就等於沒人收得到，不可回報成功");
    assert.equal(out.reason, "dev_not_added");
  } finally {
    devFails.restore();
  }
});

test("步驟 6：用 Components V2，並且同時標記回報者與開發人員", () => {
  const msg = buildIssueV2Message(REPORT);
  // V2 旗標
  assert.equal(msg.flags, 1 << 15, "IS_COMPONENTS_V2");
  assert.equal(msg.components[0].type, 17, "type 17 = Container");
  assert.equal(msg.components[0].components[0].type, 10, "type 10 = Text Display");

  // 兩個人都要被標到。content 那一行才會觸發通知，只放容器是不會通知的。
  assert.ok(msg.content.includes(`<@${REPORT.userId}>`), "要標記回報者");
  assert.ok(msg.content.includes(`<@${MCPL_DEV_USER_ID}>`), "要標記開發人員");
  assert.deepEqual(msg.allowed_mentions.users.sort(), [REPORT.userId, MCPL_DEV_USER_ID].sort());
  assert.deepEqual(msg.allowed_mentions.parse, [], "parse 必須是空的，否則 @everyone 會被放行");

  // 回報內容要在訊息裡
  const text = msg.components[0].components[0].content;
  assert.ok(text.includes(REPORT.summary));
  assert.ok(text.includes(REPORT.cause));
  assert.ok(text.includes(REPORT.detail));
  assert.ok(text.includes(REPORT.toolVersion));
  assert.ok(text.includes(REPORT.userId));
});

test("步驟 6：使用者的自由文字不能 ping 到任何人", () => {
  const msg = buildIssueV2Message({
    ...REPORT,
    detail: "@everyone @here <@999999999999999999> 快來看這個問題啦啦啦",
  });
  const text = msg.components[0].components[0].content;
  assert.ok(!text.includes("@everyone"), "@everyone 要被清掉");
  assert.ok(!text.includes("@here"), "@here 要被清掉");
  // 就算文字裡還留著別人的 id，allowed_mentions 也只放行這兩個人
  assert.deepEqual(msg.allowed_mentions.users.sort(), [REPORT.userId, MCPL_DEV_USER_ID].sort());
  assert.deepEqual(msg.allowed_mentions.parse, []);
});

test("步驟 6：會籍查不出來時要在討論串標註，不能默默當成已確認", () => {
  const unverified = buildIssueV2Message(REPORT, { membershipVerified: false });
  assert.ok(
    unverified.components[0].components[0].content.includes("未能確認"),
    "查不到會籍要標註出來，否則站長無從分辨"
  );
  const verified = buildIssueV2Message(REPORT, { membershipVerified: true });
  assert.ok(!verified.components[0].components[0].content.includes("未能確認"));
});

test("步驟 4：只有 Discord 明講 404 才算不在伺服器，查詢失敗一律放行", async () => {
  // 站長實測：人在伺服器裡卻被擋，就是因為把「查詢失敗」當成「不在伺服器」。
  const cases = [
    [{ ok: true, status: 200 }, true, "查到就是在"],
    [{ ok: false, status: 404 }, false, "404 才是明確不在"],
    [{ ok: false, status: 403 }, null, "權限不足查不出來，不可判定為不在"],
    [{ ok: false, status: 500 }, null, "Discord 端出錯查不出來，不可判定為不在"],
  ];
  for (const [resp, want, why] of cases) {
    const fake = fakeDiscord({ "/members/": resp });
    try {
      const got = await checkGuildMembership({ DISCORD_BOT_TOKEN: "tok" }, REPORT.userId);
      assert.equal(got, want, why);
    } finally {
      fake.restore();
    }
  }
  // 沒有 token 就無從查起，一樣回 null（不阻擋）
  assert.equal(await checkGuildMembership({}, REPORT.userId), null);
});

test("步驟 3：備援必須打 MCPL 自己的端點，不是舊的『聯絡站長』", async () => {
  // 這是回報功能修了三輪都沒好的真正原因。
  // /api/contact-staff 是 bot 上另一個功能（聯絡站長 #12），有自己的頻道與會籍檢查；
  // 我們每一輪修的 runMcplIssueThread 掛在 /api/mcpl-issue-thread，從來沒被呼叫到。
  assert.equal(BOT_ISSUE_PATH, "/api/mcpl-issue-thread");

  const fake = fakeDiscord({ "/api/": { json: { ok: true } } });
  try {
    await submitIssueThread(
      { json: async () => ({ summary: REPORT.summary, cause: REPORT.cause, toolVersion: "1.0.9" }) },
      { BOT_API_URL: "https://bot.example", BOT_API_SECRET: "s" },
      { userId: REPORT.userId, displayName: REPORT.username }
    );
    const hit = fake.calls.find((c) => c.method === "POST" && c.url.includes("bot.example"));
    assert.ok(hit, "必須真的打了 bot");
    assert.ok(
      hit.url.endsWith(BOT_ISSUE_PATH),
      `打錯端點了：${hit.url}（打到 contact-staff 就是這次的 bug 重演）`
    );
    // payload 要符合 parseMcplIssueBody 的欄位，不是舊的 { message }
    const body = JSON.parse(hit.body);
    assert.equal(body.summary, REPORT.summary, "要送結構化欄位");
    assert.equal(body.cause, REPORT.cause);
    assert.ok("toolVersion" in body);
  } finally {
    fake.restore();
  }
});
