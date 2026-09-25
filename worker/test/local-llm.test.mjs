import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import worker from "../src/index.js";
import {
  allowlistFromManifest,
  applyCors,
  isSafeObjectName,
  localLlmBound,
  localLlmFile,
  parseRangeHeader,
  LOCAL_LLM_MANIFEST_KEY,
} from "../src/local-llm.mjs";

// 回歸釘：舊版用 Object.assign 把 CORS 套進 Headers，對 Headers 完全無效，
// 標頭是靜默消失的。這條測試確保套用方式真的寫得進去。
test("applyCors writes onto a Headers instance", () => {
  const headers = new Headers();
  applyCors(headers, new Request("https://example.com", { headers: { Origin: "tauri://localhost" } }));
  assert.equal(headers.get("access-control-allow-origin"), "tauri://localhost");
  assert.ok(headers.get("access-control-allow-headers")?.includes("x-zeitfrei-session"));

  const naive = new Headers();
  Object.assign(naive, { "access-control-allow-origin": "tauri://localhost" });
  assert.equal(naive.get("access-control-allow-origin"), null, "Object.assign 對 Headers 無效");
});

const FIXTURE_MANIFEST = {
  version: 1,
  modelId: "fixture-q4",
  gguf: { name: "model-q4.gguf", sha256: "a".repeat(64), bytes: 16 },
  runtimes: {
    cuda: { name: "llama-cuda.zip", sha256: "b".repeat(64), bytes: 8 },
    vulkan: { name: "llama-vulkan.zip", sha256: "c".repeat(64), bytes: 8 },
    cpu: { name: "llama-cpu.zip", sha256: "d".repeat(64), bytes: 8 },
  },
};

function mockR2(files) {
  return {
    async get(key, opts) {
      const rec = files.get(key);
      if (!rec) return null;
      let body = rec.body;
      if (opts?.range) {
        const start = Number(opts.range.offset || 0);
        const length = Number(opts.range.length || body.byteLength);
        body = body.slice(start, start + length);
      }
      return {
        body,
        size: rec.body.byteLength,
        httpEtag: `"${key}"`,
        writeHttpMetadata(headers) {
          headers.set("content-type", rec.type || "application/octet-stream");
        },
        async text() {
          return new TextDecoder().decode(rec.body);
        },
      };
    },
    async head(key) {
      const rec = files.get(key);
      return rec ? { size: rec.body.byteLength } : null;
    },
  };
}

test("allowlist 只含 manifest＋GGUF＋三套 zip 檔名", () => {
  const allow = allowlistFromManifest(FIXTURE_MANIFEST);
  assert.equal(allow.has(LOCAL_LLM_MANIFEST_KEY), true);
  assert.equal(allow.has("model-q4.gguf"), true);
  assert.equal(allow.has("llama-cuda.zip"), true);
  assert.equal(allow.has("llama-cpu.zip"), true);
  assert.equal(isSafeObjectName("../secret"), false);
  assert.equal(isSafeObjectName("a/b.gguf"), false);
});

test("parseRangeHeader 支援 bytes=start-end", () => {
  assert.deepEqual(parseRangeHeader("bytes=0-3", 16), { start: 0, end: 3, length: 4 });
  assert.equal(parseRangeHeader("bytes=90-99", 16).error, "unsatisfiable");
  assert.equal(parseRangeHeader("bytes=abc", 16).error, "invalid range");
});

test("GET /api/local-llm/manifest 無 Discord 回 401", async () => {
  const res = await worker.fetch(
    new Request("https://example.com/api/local-llm/manifest", {
      headers: { "x-zeitfrei-ai-protocol": "3" },
    }),
    {
      LOCAL_LLM: mockR2(new Map()),
      MANAGED_AI_PROTOCOL: "3",
    }
  );
  assert.equal(res.status, 401);
  const body = await res.json();
  assert.equal(body.error?.type, "login_required");
});

test("GET /api/local-llm/file 無 Discord 回 401", async () => {
  const res = await worker.fetch(
    new Request("https://example.com/api/local-llm/file/model-q4.gguf", {
      headers: { Range: "bytes=0-3", "x-zeitfrei-ai-protocol": "3" },
    }),
    { LOCAL_LLM: mockR2(new Map()), MANAGED_AI_PROTOCOL: "3" }
  );
  assert.equal(res.status, 401);
});

test("清單有檔名但 R2 沒物件時 Range 回 404 不是 400", async () => {
  const files = new Map([
    [
      LOCAL_LLM_MANIFEST_KEY,
      { body: new TextEncoder().encode(JSON.stringify(FIXTURE_MANIFEST)), type: "application/json" },
    ],
  ]);
  const env = { LOCAL_LLM: mockR2(files) };
  const authorize = async () => ({ ok: true, userId: "1" });
  const url = new URL("https://example.com/api/local-llm/file/model-q4.gguf");
  const res = await localLlmFile(
    new Request(url, { headers: { Range: "bytes=0-8388607" } }),
    env,
    url,
    authorize
  );
  assert.equal(res.status, 404);
  const body = await res.json();
  assert.equal(body.error, "not found");
});

test("Range 回 206 且只切需要的位元組", async () => {
  const bytes = new Uint8Array([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
  const files = new Map([
    [
      LOCAL_LLM_MANIFEST_KEY,
      { body: new TextEncoder().encode(JSON.stringify(FIXTURE_MANIFEST)), type: "application/json" },
    ],
    ["model-q4.gguf", { body: bytes }],
  ]);
  const env = { LOCAL_LLM: mockR2(files) };
  const authorize = async () => ({ ok: true, userId: "1" });
  const url = new URL("https://example.com/api/local-llm/file/model-q4.gguf");
  const res = await localLlmFile(
    new Request(url, { headers: { Range: "bytes=2-5" } }),
    env,
    url,
    authorize
  );
  assert.equal(res.status, 206);
  assert.equal(res.headers.get("content-range"), "bytes 2-5/16");
  const buf = new Uint8Array(await res.arrayBuffer());
  assert.deepEqual([...buf], [2, 3, 4, 5]);
});

test("獨立 LOCAL_LLM binding，不寫 SHARES／DOWNLOADS", () => {
  const src = readFileSync(new URL("../src/local-llm.mjs", import.meta.url), "utf8");
  assert.match(src, /env\?\.LOCAL_LLM/);
  assert.doesNotMatch(src, /env\.SHARES/);
  assert.doesNotMatch(src, /env\.DOWNLOADS/);
  const wrangler = readFileSync(new URL("../wrangler.toml", import.meta.url), "utf8");
  assert.match(wrangler, /binding = "LOCAL_LLM"/);
  assert.match(wrangler, /bucket_name = "modpack-i18n-local-llm"/);
  assert.doesNotMatch(wrangler, /LATEST_VERSION = "1\.0\.9"/);
});

test("localLlmBound 看 binding 與 manifest 是否存在", async () => {
  assert.equal(await localLlmBound({}), false);
  assert.equal(await localLlmBound({ LOCAL_LLM: mockR2(new Map()) }), false);
  const withManifest = mockR2(
    new Map([[LOCAL_LLM_MANIFEST_KEY, { body: new TextEncoder().encode("{}"), type: "application/json" }]])
  );
  assert.equal(await localLlmBound({ LOCAL_LLM: withManifest }), true);
});
