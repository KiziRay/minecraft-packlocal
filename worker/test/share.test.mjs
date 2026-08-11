import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const source = await readFile(new URL("../src/index.js", import.meta.url), "utf8");

test("分享檔使用獨立 SHARES bucket，不寫入 DOWNLOADS", () => {
  const start = source.indexOf("async function shareUpload");
  const end = source.indexOf("async function shareDownload", start);
  assert.ok(start >= 0 && end > start);
  const upload = source.slice(start, end);
  assert.match(upload, /env\.SHARES\.put/);
  assert.doesNotMatch(upload, /env\.DOWNLOADS/);
});

test("分享下載會檢查期限並讓過期物件失效", () => {
  const start = source.indexOf("async function shareDownload");
  const end = source.indexOf("function randomShareToken", start);
  const download = source.slice(start, end);
  assert.match(download, /expiresAt/);
  assert.match(download, /share expired/);
  assert.match(download, /cache-control/);
});

test("分享連結先顯示可嵌入的介紹頁，下載需明確指定", () => {
  const start = source.indexOf("async function shareDownload");
  const end = source.indexOf("function randomShareToken", start);
  const download = source.slice(start, end);
  assert.match(download, /renderShareLanding/);
  assert.match(download, /downloadRequested/);
  assert.match(download, /og:title/);
  assert.match(download, /cloud\.zeitfrei\.uk\/zeitfreitool/);
  assert.match(download, /frame-ancestors \\*/);
});

test("共享翻譯記憶保留上下文並標記衝突，不覆蓋舊譯文", () => {
  assert.match(source, /tm\/v2\/global\.json\.gz/);
  assert.match(source, /function tmMerge/);
  assert.match(source, /conflict: true/);
  assert.match(source, /ctx/);
});

test("強制 Turnstile 設定不完整時不會退化成只檢查 Discord", () => {
  const start = source.indexOf("async function authorizeManagedAi");
  const end = source.indexOf("async function authorizeManagedIdentity", start);
  const auth = source.slice(start, end);
  assert.match(auth, /turnstileEnforced && !turnstileConfigured\(env\)/);
  assert.match(auth, /type: "turnstile_unavailable"/);
});
