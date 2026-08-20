import test from "node:test";
import assert from "node:assert/strict";

import {
  REPORT_CATEGORIES,
  REPORT_MAX_BYTES,
  REPORT_PREFIX,
  REPORT_TTL_SECONDS,
  buildWebhookText,
  cleanupReports,
  isAllowedReportName,
  isReportCategory,
  isReportToken,
  looksLikeZipMagic,
  reportMpuComplete,
  reportMpuCreate,
  reportObjectKey,
  reportQuotaDecision,
  sanitizePackLabel,
} from "../src/report.mjs";

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
    async delete(key) {
      store.delete(key);
    },
    async list({ prefix, cursor } = {}) {
      return {
        keys: [...store.keys()].filter((name) => name.startsWith(prefix)).map((name) => ({ name })),
        list_complete: true,
        cursor,
      };
    },
  };
}

function mockShares() {
  const objects = new Map();
  const uploads = new Map();
  let uploadSeq = 0;
  return {
    objects,
    uploads,
    async put(key, _body, opts = {}) {
      const rec = {
        key,
        size: 12,
        body: "PK\x03\x04zip",
        customMetadata: { ...(opts.customMetadata || {}) },
        httpMetadata: { ...(opts.httpMetadata || {}) },
        writeHttpMetadata(headers) {
          if (this.httpMetadata.contentType) headers.set("content-type", this.httpMetadata.contentType);
        },
      };
      objects.set(key, rec);
      return rec;
    },
    async get(key, opts = {}) {
      const rec = objects.get(key);
      if (!rec) return null;
      if (opts.range) {
        return {
          arrayBuffer: async () => new Uint8Array([0x50, 0x4b, 0x03, 0x04]).buffer,
        };
      }
      return rec;
    },
    async head(key) {
      return objects.get(key) || null;
    },
    async delete(key) {
      objects.delete(key);
    },
    async list({ prefix }) {
      return {
        objects: [...objects.keys()].filter((key) => key.startsWith(prefix)).map((key) => ({ key })),
      };
    },
    async createMultipartUpload(key, opts = {}) {
      uploadSeq += 1;
      const uploadId = `up-${uploadSeq}`;
      uploads.set(uploadId, { key, opts, aborted: false });
      return { key, uploadId };
    },
    resumeMultipartUpload(key, uploadId) {
      const self = this;
      return {
        async uploadPart(partNumber) {
          return { partNumber, etag: `etag-${partNumber}` };
        },
        async complete() {
          const rec = {
            key,
            size: 32,
            body: "PK-report",
            customMetadata: { expiresAt: String(Math.floor(Date.now() / 1000) + REPORT_TTL_SECONDS) },
            httpMetadata: { contentType: "application/zip" },
            writeHttpMetadata() {},
          };
          objects.set(key, rec);
          return rec;
        },
        async abort() {
          const rec = uploads.get(uploadId);
          if (rec) rec.aborted = true;
        },
      };
    },
  };
}

test("問題類型白名單與檔名白名單", () => {
  assert.equal(isReportCategory("crash_after_apply"), true);
  assert.equal(isReportCategory("not_a_category"), false);
  assert.ok(REPORT_CATEGORIES.has("tool_ai_managed"));
  assert.equal(isAllowedReportName("manifest.json"), true);
  assert.equal(isAllowedReportName("覆蓋範圍說明.txt"), true);
  assert.equal(isAllowedReportName("../mods/a.jar"), false);
  assert.equal(isAllowedReportName("evil.exe"), false);
  assert.equal(isAllowedReportName("crash-2026-01-01.txt"), true);
});

test("配額：每日 3 次、同時上限、短時節流", () => {
  assert.equal(reportQuotaDecision({ dailyCount: 3, activeCount: 0, lastAt: 0, nowSec: 100 }).status, 429);
  assert.equal(reportQuotaDecision({ dailyCount: 0, activeCount: 5, lastAt: 0, nowSec: 100 }).status, 429);
  assert.equal(reportQuotaDecision({ dailyCount: 0, activeCount: 0, lastAt: 90, nowSec: 100 }).status, 429);
  assert.equal(reportQuotaDecision({ dailyCount: 0, activeCount: 0, lastAt: 0, nowSec: 100 }).ok, true);
});

test("webhook 短訊含類別、包名與連結，不含路徑", () => {
  const text = buildWebhookText({
    category: "crash_after_apply",
    packLabel: "Example Pack",
    packVersion: "1.2.3",
    errorCode: "RESOURCE_PATH_CORRUPTED",
    toolVersion: "1.0.0",
    url: "https://modpack-i18n.jolin34563.workers.dev/report/abc",
  });
  assert.match(text, /^\[crash_after_apply\]/);
  assert.match(text, /Example Pack/);
  assert.match(text, /RESOURCE_PATH_CORRUPTED/);
  assert.match(text, /\/report\/abc/);
  assert.equal(text.includes("C:\\"), false);
  assert.equal(sanitizePackLabel("", true), "與包無關");
});

test("zip 魔數與 100MB 上限", () => {
  assert.equal(looksLikeZipMagic(new Uint8Array([0x50, 0x4b, 0x03, 0x04])), true);
  assert.equal(looksLikeZipMagic(new Uint8Array([0x4d, 0x5a])), false);
  assert.equal(REPORT_MAX_BYTES, 100 * 1024 * 1024);
  assert.equal(REPORT_TTL_SECONDS, 3 * 24 * 60 * 60);
  assert.ok(reportObjectKey("abc").startsWith(REPORT_PREFIX));
});

test("MPU create 無 USAGE 或無 webhook 會失敗", async () => {
  const SHARES = mockShares();
  const req = new Request("https://example.com/api/report/mpu-create", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      size: 12,
      reportCategory: "other",
      packUnrelated: true,
    }),
  });
  const noUsage = await reportMpuCreate(req.clone(), { SHARES, DISCORD_REPORT_WEBHOOK: "https://example.invalid/hook" }, "u1");
  assert.equal(noUsage.status, 503);
  const noHook = await reportMpuCreate(req, { SHARES, USAGE: mockUsage() }, "u1");
  assert.equal(noHook.status, 503);
});

test("MPU create 成功並寫入 reports/v1 前綴", async () => {
  const env = {
    SHARES: mockShares(),
    USAGE: mockUsage(),
    DISCORD_REPORT_WEBHOOK: "https://example.invalid/hook",
    SHARE_PUBLIC_URL: "https://modpack-i18n.jolin34563.workers.dev",
  };
  const req = new Request("https://example.com/api/report/mpu-create", {
    method: "POST",
    headers: { "content-type": "application/json", "x-zeitfrei-client-version": "1.0.0" },
    body: JSON.stringify({
      size: 32,
      reportCategory: "crash_after_apply",
      packName: "Test Pack",
      packVersion: "1.0",
      errorCode: "X",
    }),
  });
  const resp = await reportMpuCreate(req, env, "user-1", 1_700_000_000);
  assert.equal(resp.status, 200);
  const body = await resp.json();
  assert.equal(isReportToken(body.token), true);
  assert.ok(String(body.key).startsWith(REPORT_PREFIX));
  assert.ok(body.uploadId);
});

test("complete 時 webhook 失敗則不把部分成功當完成", async () => {
  const env = {
    SHARES: mockShares(),
    USAGE: mockUsage(),
    DISCORD_REPORT_WEBHOOK: "https://example.invalid/hook",
    SHARE_PUBLIC_URL: "https://example.com",
  };
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => new Response("fail", { status: 500 });
  try {
    const created = await reportMpuCreate(
      new Request("https://example.com/api/report/mpu-create", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          size: 32,
          reportCategory: "other",
          packUnrelated: true,
        }),
      }),
      env,
      "user-1",
      1_700_000_000
    );
    const createdBody = await created.json();
    const complete = await reportMpuComplete(
      new Request("https://example.com/api/report/mpu-complete", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          token: createdBody.token,
          key: createdBody.key,
          uploadId: createdBody.uploadId,
          parts: [{ partNumber: 1, etag: "etag-1" }],
        }),
      }),
      env,
      "user-1",
      1_700_000_000
    );
    assert.equal(complete.status, 502);
    const msg = await complete.json();
    assert.equal(msg.error, "report notify failed");
    assert.equal(env.SHARES.objects.has(createdBody.key), false);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("cleanupReports 刪過期 R2 與 KV 索引", async () => {
  const now = 1_700_000_000;
  const USAGE = mockUsage();
  const SHARES = mockShares();
  await SHARES.put(`${REPORT_PREFIX}old.zip`, null, { customMetadata: { expiresAt: String(now - 10) } });
  await SHARES.put(`${REPORT_PREFIX}fresh.zip`, null, { customMetadata: { expiresAt: String(now + 100) } });
  await USAGE.put("report:id:old", "{}");
  await cleanupReports({ USAGE, SHARES }, now);
  assert.equal(SHARES.objects.has(`${REPORT_PREFIX}old.zip`), false);
  assert.equal(SHARES.objects.has(`${REPORT_PREFIX}fresh.zip`), true);
});
