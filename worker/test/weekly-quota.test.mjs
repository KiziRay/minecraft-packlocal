import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import {
  utcIsoWeek,
  nextUtcWeekStartIso,
  sharedUsageKey,
  isSharedWeeklyQuotaExhausted,
  isUserDailyQuotaExhausted,
  tryIncrementUsageKv,
} from "../src/index.js";

test("utcIsoWeek 格式 YYYY-Www（UTC 週一為週首）", () => {
  assert.equal(utcIsoWeek(new Date("2026-08-17T12:00:00.000Z")), "2026-W34");
  assert.equal(utcIsoWeek(new Date("2026-08-16T23:59:00.000Z")), "2026-W33");
});

test("E15：ISO 週切換產生不同 shared key", () => {
  const w33 = utcIsoWeek(new Date("2026-08-16T12:00:00.000Z"));
  const w34 = utcIsoWeek(new Date("2026-08-17T12:00:00.000Z"));
  assert.notEqual(w33, w34);
  assert.equal(sharedUsageKey(w33), `usage:shared:${w33}`);
  assert.equal(sharedUsageKey(w34), `usage:shared:${w34}`);
});

test("nextUtcWeekStartIso 為下週一 00:00 UTC", () => {
  const reset = nextUtcWeekStartIso(new Date("2026-08-20T15:00:00.000Z"));
  assert.equal(reset, "2026-08-24T00:00:00.000Z");
});

test("E14：共享週 spent=10M 觸發週上限", () => {
  const budget = 10_000_000;
  assert.equal(isSharedWeeklyQuotaExhausted(10_000_000, budget), true);
  assert.equal(isSharedWeeklyQuotaExhausted(9_999_999, budget), false);
});

test("E16：個人滿額與共享週額度分離", () => {
  const personalBudget = 1_000_000;
  const personalSpent = 500_000;
  assert.equal(isUserDailyQuotaExhausted(personalSpent, personalBudget), false);

  const exhaustedPersonal = 1_000_000;
  assert.equal(isUserDailyQuotaExhausted(exhaustedPersonal, personalBudget), true);
  assert.equal(isSharedWeeklyQuotaExhausted(0, 10_000_000), false);
});

test("tryIncrementUsageKv 寫前再讀，超過 maxTotal 拒絕", async () => {
  const store = new Map();
  const kv = {
    get: async (key) => store.get(key) ?? null,
    put: async (key, value) => {
      store.set(key, value);
    },
  };
  const key = sharedUsageKey("2026-W34");
  const first = await tryIncrementUsageKv(kv, key, 100, 604800, 150);
  assert.equal(first.ok, true);
  assert.equal(first.spent, 100);
  const blocked = await tryIncrementUsageKv(kv, key, 100, 604800, 150);
  assert.equal(blocked.ok, false);
  assert.equal(blocked.spent, 100);
});

test("Worker 不再代理免費代管 chat 或額度 API", () => {
  const src = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  assert.doesNotMatch(src, /async function proxyChat/);
  assert.doesNotMatch(src, /async function managedUsage/);
  assert.doesNotMatch(src, /\/v1\/chat\/completions/);
  assert.doesNotMatch(src, /\/api\/managed\/usage/);
  assert.match(src, /sharedUsageKey/);
  assert.match(src, /tryIncrementUsageKv/);
});
