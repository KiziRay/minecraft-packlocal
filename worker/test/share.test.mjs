import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

import {
  SHARE_OG_DESCRIPTION,
  SHARE_OG_TITLE,
  SHARE_SFX_DOWNLOAD_NAME,
  cleanupShares,
  isLongShareToken,
  isShortShareCode,
  quotaDecision,
  renderShareLanding,
  reserveShareSlot,
  resolveShareObject,
  shareContentDisposition,
  shareDownload,
  shareMpuComplete,
  shareMpuCreate,
  shareOgImage,
  shareUpload,
} from "../src/share.mjs";

const indexSource = await readFile(new URL("../src/index.js", import.meta.url), "utf8");
const shareSource = await readFile(new URL("../src/share.mjs", import.meta.url), "utf8");
const tmSource = await readFile(new URL("../src/tm.mjs", import.meta.url), "utf8");

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
    async list({ prefix }) {
      return {
        keys: [...store.keys()].filter((name) => name.startsWith(prefix)).map((name) => ({ name })),
        list_complete: true,
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
        body: "MZ-share",
        customMetadata: { ...(opts.customMetadata || {}) },
        httpMetadata: { ...(opts.httpMetadata || {}) },
        writeHttpMetadata(headers) {
          if (this.httpMetadata.contentType) headers.set("content-type", this.httpMetadata.contentType);
        },
      };
      objects.set(key, rec);
      return rec;
    },
    async get(key) {
      return objects.get(key) || null;
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
          const rec = uploads.get(uploadId);
          const obj = {
            key,
            size: 12,
            body: "MZ-share",
            customMetadata: { ...(rec?.opts?.customMetadata || {}) },
            httpMetadata: { ...(rec?.opts?.httpMetadata || {}) },
            writeHttpMetadata(headers) {
              if (this.httpMetadata.contentType) headers.set("content-type", this.httpMetadata.contentType);
            },
          };
          objects.set(key, obj);
          return obj;
        },
        async abort() {
          const rec = uploads.get(uploadId);
          if (rec) rec.aborted = true;
          self.aborted = uploadId;
        },
      };
    },
  };
}

function jsonRequest(url, body, hdrs = {}) {
  const headers = new Map(Object.entries(hdrs).map(([k, v]) => [k.toLowerCase(), String(v)]));
  return {
    url,
    headers: {
      get(name) {
        return headers.get(String(name).toLowerCase()) || null;
      },
    },
    async json() {
      return body;
    },
    body: typeof body === "string" ? body : "MZ",
  };
}

test("分享檔使用獨立 SHARES bucket，不寫入 DOWNLOADS", () => {
  assert.match(shareSource, /env\.SHARES\.put/);
  assert.doesNotMatch(shareSource, /env\.DOWNLOADS/);
  assert.match(indexSource, /from "\.\/share\.mjs"/);
});

test("分享下載會檢查期限並讓過期物件失效", () => {
  assert.match(shareSource, /expiresAt/);
  assert.match(shareSource, /share expired/);
  assert.match(shareSource, /cache-control/);
});

test("分享上傳要求有效 Content-Length，避免繞過大小限制", () => {
  const start = shareSource.indexOf("export async function shareUpload");
  const end = shareSource.indexOf("export async function shareMpuCreate", start);
  const upload = shareSource.slice(start, end);
  assert.match(upload, /content-length/);
  assert.match(upload, /content length required/);
  assert.match(upload, /share file too large/);
});

test("分享上傳接受 exe／zip，上限由 SHARE_MAX_BYTES／shareMaxBytes 決定（無程式內死鎖 100MB）", () => {
  assert.match(shareSource, /function shareMaxBytes/);
  assert.match(shareSource, /SHARE_MAX_BYTES/);
  assert.doesNotMatch(shareSource, /Math\.min\([^\n]*104857600/);
  const start = shareSource.indexOf("export async function shareUpload");
  const end = shareSource.indexOf("export async function shareMpuCreate", start);
  const upload = shareSource.slice(start, end);
  assert.match(upload, /application\/vnd\.microsoft\.portable-executable/);
  assert.match(upload, /shareMaxBytes/);
  assert.match(upload, /\.exe/);
});

test("分享 multipart 走 SHARES createMultipartUpload／uploadPart／complete", () => {
  assert.match(shareSource, /export async function shareMpuCreate/);
  assert.match(shareSource, /export async function shareMpuPart/);
  assert.match(shareSource, /export async function shareMpuComplete/);
  assert.match(shareSource, /createMultipartUpload/);
  assert.match(shareSource, /resumeMultipartUpload/);
  assert.match(shareSource, /uploadPart/);
  assert.doesNotMatch(shareSource, /env\.DOWNLOADS/);
  assert.match(shareSource, /partSize/);
});

test("分享連結先顯示可嵌入的介紹頁，下載需明確指定", () => {
  assert.match(shareSource, /og:title/);
  assert.match(shareSource, /og:image/);
  assert.match(shareSource, /discord\.gg\/zeitfrei/);
  assert.match(shareSource, /zeitfrei\.bobaboba\.me/);
  assert.match(shareSource, /cloud\.zeitfrei\.uk/);
  assert.match(shareSource, /解壓密碼/);
  assert.match(shareSource, /選擇 Minecraft/);
  assert.match(shareSource, new RegExp(SHARE_OG_TITLE));
  assert.match(shareSource, new RegExp(SHARE_OG_DESCRIPTION));
});

test("短碼與長 token 分流：8 碼走 KV，32+ 仍可下載舊檔", () => {
  assert.equal(isShortShareCode("Ab12Cd34"), true);
  assert.equal(isShortShareCode("too-short"), false);
  assert.equal(isLongShareToken("a".repeat(32)), true);
  assert.equal(isLongShareToken("Ab12Cd34"), false);
  assert.equal(isLongShareToken("a".repeat(16)), false);
});

test("下載 Content-Disposition 使用中文檔名 filename*", () => {
  const header = shareContentDisposition(true);
  assert.match(header, /filename="modpack-zh-tw-sfx\.exe"/);
  assert.match(header, /filename\*=UTF-8''/);
  assert.match(header, new RegExp(encodeURIComponent(SHARE_SFX_DOWNLOAD_NAME)));
});

test("配額決策：每日 3、同時 2 超限回 429", () => {
  assert.equal(quotaDecision({ dailyCount: 2, activeCount: 1, dailyLimit: 3, activeLimit: 2 }).ok, true);
  assert.equal(quotaDecision({ dailyCount: 3, activeCount: 0, dailyLimit: 3, activeLimit: 2 }).status, 429);
  assert.equal(quotaDecision({ dailyCount: 0, activeCount: 2, dailyLimit: 3, activeLimit: 2 }).error, "share active limit reached");
});

test("沒有 USAGE KV 時不擋分享", async () => {
  const reserved = await reserveShareSlot({}, "user-1", 1_700_000_000);
  assert.equal(reserved.ok, true);
  assert.equal(reserved.skipped, true);
});

test("USAGE KV 啟用後同時未過期檔達上限回 429", async () => {
  const now = 1_700_000_000;
  const USAGE = mockUsage();
  const env = { USAGE, SHARE_DAILY_LIMIT: "3", SHARE_ACTIVE_LIMIT: "2" };
  const first = await reserveShareSlot(env, "u1", now, { key: "v1/a.exe", kind: "pending", expiresAt: now + 3600 });
  const second = await reserveShareSlot(env, "u1", now, { key: "v1/b.exe", kind: "pending", expiresAt: now + 3600 });
  const third = await reserveShareSlot(env, "u1", now, { key: "v1/c.exe", kind: "pending", expiresAt: now + 3600 });
  assert.equal(first.ok, true);
  assert.equal(second.ok, true);
  assert.equal(third.status, 429);
  assert.equal(third.error, "share active limit reached");
});

test("USAGE KV 啟用後每日次數達上限回 429", async () => {
  const now = 1_700_000_000;
  const USAGE = mockUsage();
  USAGE.store.set("share:day:u2:2023-11-14", "3");
  const env = { USAGE, SHARE_DAILY_LIMIT: "3", SHARE_ACTIVE_LIMIT: "10" };
  const result = await reserveShareSlot(env, "u2", now);
  assert.equal(result.status, 429);
  assert.equal(result.error, "share daily limit reached");
});

test("短碼可下載、過期後 404 並刪 R2 與短碼", async () => {
  const now = 1_700_000_000;
  const USAGE = mockUsage();
  const SHARES = mockShares();
  await SHARES.put("v1/longtokenlongtokenlongtoken12.exe", "MZ", {
    customMetadata: { expiresAt: String(now + 60), shortCode: "Ab12Cd34", kind: "sfx-exe" },
  });
  await USAGE.put("share:id:Ab12Cd34", JSON.stringify({ key: "v1/longtokenlongtokenlongtoken12.exe", expiresAt: now + 60 }));
  const env = { USAGE, SHARES };
  const found = await resolveShareObject(env, "Ab12Cd34", now);
  assert.equal(found.publicId, "Ab12Cd34");
  const expired = await resolveShareObject(env, "Ab12Cd34", now + 120);
  assert.equal(expired.status, 404);
  assert.equal(expired.error, "share expired");
  assert.equal(SHARES.objects.has("v1/longtokenlongtokenlongtoken12.exe"), false);
  assert.equal(USAGE.store.has("share:id:Ab12Cd34"), false);
});

test("長 token 仍能下載舊檔直到過期", async () => {
  const now = 1_700_000_000;
  const token = "A".repeat(32);
  const SHARES = mockShares();
  await SHARES.put(`v1/${token}.exe`, "MZ", {
    customMetadata: { expiresAt: String(now + 60), kind: "sfx-exe" },
  });
  const found = await resolveShareObject({ SHARES }, token, now);
  assert.equal(found.publicId, token);
  const expired = await resolveShareObject({ SHARES }, token, now + 90);
  assert.equal(expired.status, 404);
});

test("cleanupShares 同時刪過期 R2 與短碼，並中止逾時 MPU", async () => {
  const now = 1_700_000_000;
  const USAGE = mockUsage();
  const SHARES = mockShares();
  await SHARES.put("v1/old.exe", "MZ", {
    customMetadata: { expiresAt: String(now - 10), shortCode: "ZzYyXxWw" },
  });
  await USAGE.put("share:id:ZzYyXxWw", JSON.stringify({ key: "v1/old.exe" }));
  SHARES.uploads.set("stale-up", { key: "v1/pending.exe", aborted: false });
  await USAGE.put(
    "share:mpu:stale-up",
    JSON.stringify({ key: "v1/pending.exe", uploadId: "stale-up", userId: "u3", createdAt: now - 7200 })
  );
  await cleanupShares({ USAGE, SHARES, SHARE_MPU_STALE_SECONDS: "3600" }, now);
  assert.equal(SHARES.objects.has("v1/old.exe"), false);
  assert.equal(USAGE.store.has("share:id:ZzYyXxWw"), false);
  assert.equal(SHARES.uploads.get("stale-up").aborted, true);
});

test("落地頁 OG 標題與副標固定，包名在次要列", async () => {
  const res = renderShareLanding(
    new URL("https://modpack-i18n.jolin34563.workers.dev/s/Ab12Cd34"),
    { SHARE_PUBLIC_URL: "https://modpack-i18n.jolin34563.workers.dev" },
    "Ab12Cd34",
    {
      customMetadata: { name: "測試包", kind: "sfx-exe", password: "cloud.zeitfrei.uk" },
      httpMetadata: { contentType: "application/vnd.microsoft.portable-executable" },
    },
    1_700_000_000
  );
  const html = await res.text();
  assert.match(html, new RegExp(`og:title" content="${SHARE_OG_TITLE}"`));
  assert.match(html, new RegExp(`og:description" content="${SHARE_OG_DESCRIPTION}"`));
  assert.match(html, /<h1>繁體中文模組包翻譯工具<\/h1>/);
  assert.match(html, /讓模組包翻譯不再困難/);
  assert.match(html, /包名：測試包/);
  assert.match(html, /解壓密碼/);
  assert.match(html, /選擇 Minecraft/);
});

test("OG 圖同步主標與副標", async () => {
  const svg = await shareOgImage(false).text();
  assert.match(svg, new RegExp(SHARE_OG_TITLE));
  assert.match(svg, new RegExp(SHARE_OG_DESCRIPTION));
});

test("有 USAGE 時 MPU 完成回短碼 URL", async () => {
  const now = 1_700_000_000;
  const env = {
    USAGE: mockUsage(),
    SHARES: mockShares(),
    SHARE_PUBLIC_URL: "https://modpack-i18n.jolin34563.workers.dev",
    SHARE_DAILY_LIMIT: "3",
    SHARE_ACTIVE_LIMIT: "2",
  };
  const created = await shareMpuCreate(
    jsonRequest("https://example/api/share/mpu-create", { name: "測", kind: "sfx-exe", size: 100, contentType: "application/vnd.microsoft.portable-executable" }),
    env,
    "user-x",
    now
  );
  assert.equal(created.status, 200);
  const createdBody = await created.json();
  assert.equal(isLongShareToken(createdBody.token), true);
  const complete = await shareMpuComplete(
    jsonRequest("https://example/api/share/mpu-complete", {
      token: createdBody.token,
      key: createdBody.key,
      uploadId: createdBody.uploadId,
      parts: [{ partNumber: 1, etag: "etag-1" }],
    }),
    env,
    "user-x",
    now
  );
  assert.equal(complete.status, 200);
  const done = await complete.json();
  assert.match(done.url, /^https:\/\/modpack-i18n\.jolin34563\.workers\.dev\/s\/[A-Za-z0-9]{8}$/);
});

test("沒有 USAGE 時 MPU 完成仍回長 token URL", async () => {
  const now = 1_700_000_000;
  const env = {
    SHARES: mockShares(),
    SHARE_PUBLIC_URL: "https://modpack-i18n.jolin34563.workers.dev",
  };
  const created = await shareMpuCreate(
    jsonRequest("https://example/api/share/mpu-create", { name: "測", kind: "sfx-exe", size: 100, contentType: "application/vnd.microsoft.portable-executable" }),
    env,
    "user-y",
    now
  );
  const createdBody = await created.json();
  const complete = await shareMpuComplete(
    jsonRequest("https://example/api/share/mpu-complete", {
      token: createdBody.token,
      key: createdBody.key,
      uploadId: createdBody.uploadId,
      parts: [{ partNumber: 1, etag: "etag-1" }],
    }),
    env,
    "user-y",
    now
  );
  const done = await complete.json();
  assert.match(done.url, /\/s\/[A-Za-z0-9_-]{32,128}$/);
});

test("下載短碼附中文 Content-Disposition", async () => {
  const now = 1_700_000_000;
  const USAGE = mockUsage();
  const SHARES = mockShares();
  await SHARES.put("v1/longtokenlongtokenlongtoken12.exe", "MZ", {
    customMetadata: { expiresAt: String(now + 60), shortCode: "Ab12Cd34", kind: "sfx-exe" },
    httpMetadata: { contentType: "application/vnd.microsoft.portable-executable" },
  });
  await USAGE.put("share:id:Ab12Cd34", JSON.stringify({ key: "v1/longtokenlongtokenlongtoken12.exe", expiresAt: now + 60 }));
  const res = await shareDownload(
    new URL("https://example/s/Ab12Cd34?download=1"),
    { USAGE, SHARES },
    false,
    now
  );
  assert.equal(res.status, 200);
  assert.match(res.headers.get("content-disposition"), /filename\*=UTF-8''/);
  assert.match(res.headers.get("content-disposition"), new RegExp(encodeURIComponent(SHARE_SFX_DOWNLOAD_NAME)));
});

test("單檔上傳缺 Content-Length 仍拒絕", async () => {
  const res = await shareUpload(
    jsonRequest("https://example/api/share/upload", "MZ", {
      "content-type": "application/vnd.microsoft.portable-executable",
    }),
    { SHARES: mockShares() },
    "user-z"
  );
  assert.equal(res.status, 411);
});

test("共享翻譯記憶改多數決，不再永久 conflict 凍結", () => {
  assert.match(indexSource, /tm\/v2\/global\.json\.gz/);
  assert.match(indexSource, /tmMerge/);
  assert.match(tmSource, /export function tmMerge/);
  assert.match(tmSource, /TM_CROSS_PACK_MIN_VOTES = 2/);
  assert.match(tmSource, /ctx/);
});

test("共享翻譯資料使用獨立 TRANSLATIONS bucket", () => {
  assert.match(indexSource, /env\.TRANSLATIONS/);
  assert.match(indexSource, /glossary\/v1\/global\.json\.gz/);
  assert.match(indexSource, /function glossaryLookup/);
  assert.match(indexSource, /function glossaryContribute/);
});

test("glossaryLookup 同包可命中、跨包仍需 votes≥2", () => {
  const start = indexSource.indexOf("async function glossaryLookup");
  const end = indexSource.indexOf("async function glossaryContribute", start);
  const body = indexSource.slice(start, end);
  assert.match(body, /tmCanUse/);
  assert.match(tmSource, /samePackHit/);
  assert.match(tmSource, /TM_PACK_MIN_VOTES = 1/);
});

test("共享術語與 TM 共用多數決，不再寫死 conflict", () => {
  assert.match(indexSource, /tmMerge\(glossary/);
  assert.doesNotMatch(tmSource, /previous\.conflict = true/);
});

test("代管閘門改為僅 Discord（不再強制 Turnstile）", () => {
  const start = indexSource.indexOf("async function authorizeManagedAi");
  const end = indexSource.indexOf("async function authorizeManagedIdentity", start);
  const auth = indexSource.slice(start, end);
  assert.match(auth, /authorizeManagedIdentity/);
  assert.doesNotMatch(auth, /turnstileEnforced/);
  assert.doesNotMatch(auth, /verifyTurnstileAccess/);
});

test("共享 TM／glossary packs 上限 16，避免無限併包撐大", () => {
  assert.match(tmSource, /TM_PACKS_CAP\s*=\s*16/);
  assert.match(tmSource, /Object\.keys\(previous\.packs\)\.length < TM_PACKS_CAP/);
  const glossStart = indexSource.indexOf("async function glossaryContribute");
  const gloss = indexSource.slice(glossStart, glossStart + 2500);
  assert.match(gloss, /tmMerge\(glossary/);
});
