import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { effectiveUserBudget, isUserDailyQuotaExhausted, sharedUsageKey, utcIsoWeek } from "../src/index.js";

function mockUsage(getImpl) {
  return { get: async (key) => getImpl(key) };
}

test("effectiveUserBudget 未領 GP 時為基礎上限", async () => {
  const env = {
    PER_USER_DAILY_TOKEN_BUDGET: "500000",
    GP_REWARD_BONUS: "500000",
    USAGE: mockUsage(async () => null),
  };
  assert.equal(await effectiveUserBudget(env, "111"), 500000);
});

test("effectiveUserBudget 已領 GP 時為基礎 + 加成", async () => {
  const env = {
    PER_USER_DAILY_TOKEN_BUDGET: "500000",
    GP_REWARD_BONUS: "500000",
    USAGE: mockUsage(async (key) => (key === "gp_reward:222" ? "1" : null)),
  };
  assert.equal(await effectiveUserBudget(env, "222"), 1000000);
});

test("effectiveUserBudget 無 KV 時回基礎上限", async () => {
  const env = {
    PER_USER_DAILY_TOKEN_BUDGET: "500000",
    GP_REWARD_BONUS: "500000",
  };
  assert.equal(await effectiveUserBudget(env, "333"), 500000);
});

test("isUserDailyQuotaExhausted：未 GP 時 spent=500000 已觸頂", () => {
  assert.equal(isUserDailyQuotaExhausted(500000, 500000), true);
  assert.equal(isUserDailyQuotaExhausted(499999, 500000), false);
});

test("isUserDailyQuotaExhausted：已 GP 時 spent=500000 仍可請求", () => {
  assert.equal(isUserDailyQuotaExhausted(500000, 1000000), false);
  assert.equal(isUserDailyQuotaExhausted(1000000, 1000000), true);
});

test("managedUsage 回傳欄位語意：GP 後 userBudget 為 effective 總額度", async () => {
  const week = utcIsoWeek();
  const env = {
    WEEKLY_SHARED_TOKEN_BUDGET: "10000000",
    PER_USER_DAILY_TOKEN_BUDGET: "500000",
    GP_REWARD_BONUS: "500000",
    USAGE: mockUsage(async (key) => {
      if (key.startsWith("usage:user:")) return "500000";
      if (key === sharedUsageKey(week)) return "0";
      if (key === "gp_reward:444") return "1";
      return null;
    }),
  };
  const userBudget = await effectiveUserBudget(env, "444");
  assert.equal(userBudget, 1000000);
  assert.equal(isUserDailyQuotaExhausted(500000, userBudget), false);
});

test("join Discord 公告已恢復", () => {
  const src = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  assert.ok(src.includes("maybeNotifyDiscordJoinOncePerDay"));
  assert.ok(src.includes("renderDiscordJoinContent"));
});
