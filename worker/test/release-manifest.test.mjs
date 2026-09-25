import test from "node:test";
import assert from "node:assert/strict";

import { buildReleaseManifest, resolveDesktopDownloadUrl } from "../src/index.js";

const SHA = "c4c33ca3586f9e2a0ced727d3bd88f3b8890b8af7615c443cd7630caaae560cf";
const HOST = "https://modpack-i18n.jolin34563.workers.dev";

function env(overrides = {}) {
  return {
    DOWNLOAD_URL: `${HOST}/download/MCPL-1.1.0.exe`,
    LEGACY_DOWNLOAD_URL: `${HOST}/download/MCPL-1.1.0.exe`,
    UPDATE_SHA256: SHA,
    RELEASE_CHANNEL: "stable",
    MANIFEST_VERSION: "1.1.0",
    MANIFEST_BUILD_ID: "1.1.0-e66c7999b843",
    MANIFEST_RELEASED_AT: "2026-09-04T00:00:00Z",
    MANIFEST_COMMIT: "e66c7999b8432b2a4f370a62d78ef6abc211c8aa",
    MANIFEST_BUILD_TIME: "2026-09-04T00:00:00Z",
    MANIFEST_DIRTY: "false",
    MANIFEST_BUILDER: "local",
    MANIFEST_ARTIFACT_BYTES: 27686912,
    ...overrides,
  };
}

test("完整設定時輸出符合合約的 manifest", () => {
  const m = buildReleaseManifest(env());
  assert.equal(m.schemaVersion, 1);
  assert.equal(m.channel, "stable");
  assert.equal(m.version, "1.1.0");
  assert.equal(m.provenance.dirty, false);
  assert.equal(m.provenance.commit.length, 40);
  assert.equal(m.artifact.name, "MCPL-1.1.0.exe");
  assert.ok(m.artifact.url.startsWith("https://"));
  assert.equal(m.artifact.sha256, SHA);
  assert.equal(m.artifact.bytes, 27686912);
});

test("缺任何必要設定就整個不輸出 manifest（半套比沒有更危險）", () => {
  for (const missing of [
    "RELEASE_CHANNEL",
    "MANIFEST_VERSION",
    "MANIFEST_BUILD_ID",
    "MANIFEST_RELEASED_AT",
    "MANIFEST_COMMIT",
    "MANIFEST_BUILD_TIME",
    "UPDATE_SHA256",
    "MANIFEST_ARTIFACT_BYTES",
  ]) {
    const e = env();
    delete e[missing];
    assert.equal(buildReleaseManifest(e), null, `缺 ${missing} 時應回 null`);
  }
});

test("未知通道與四段版本號一律拒絕", () => {
  assert.equal(buildReleaseManifest(env({ RELEASE_CHANNEL: "production" })), null);
  // 四段維護號屬於 buildId，不可放進 version
  assert.equal(buildReleaseManifest(env({ MANIFEST_VERSION: "1.0.8.11" })), null);
});

test("http 下載連結與壞 sha256 一律拒絕", () => {
  const insecure = env({
    DOWNLOAD_URL: "http://example.invalid/download/MCPL-1.1.0.exe",
    LEGACY_DOWNLOAD_URL: "http://example.invalid/download/MCPL-1.1.0.exe",
  });
  assert.equal(buildReleaseManifest(insecure), null);
  assert.equal(buildReleaseManifest(env({ UPDATE_SHA256: "xyz" })), null);
});

test("dirty 預設為 true：沒明說乾淨就不得被當成 stable 候選", () => {
  const e = env();
  delete e.MANIFEST_DIRTY;
  assert.equal(buildReleaseManifest(e).provenance.dirty, true);
  assert.equal(buildReleaseManifest(env({ MANIFEST_DIRTY: "" })).provenance.dirty, true);
  assert.equal(buildReleaseManifest(env({ MANIFEST_DIRTY: "false" })).provenance.dirty, false);
});

test("沒有簽章設定就不輸出 trust 欄位（不自稱已簽）", () => {
  assert.equal(buildReleaseManifest(env()).trust, undefined);
  // 只有 keyId 沒有 signature 也不算
  assert.equal(buildReleaseManifest(env({ MANIFEST_KEY_ID: "k1" })).trust, undefined);
  const signed = buildReleaseManifest(env({ MANIFEST_KEY_ID: "k1", MANIFEST_SIGNATURE: "sig" }));
  assert.deepEqual(signed.trust, { algorithm: "ed25519-v1", keyId: "k1", signature: "sig" });
});

test("manifest 的下載檔名沿用三段式規則（1.0.8.1 事故回歸）", () => {
  // DOWNLOAD_URL 是四段時，resolveDesktopDownloadUrl 會改用 LEGACY 的三段式
  const e = env({
    DOWNLOAD_URL: `${HOST}/download/MCPL-1.1.0.1.exe`,
    LEGACY_DOWNLOAD_URL: `${HOST}/download/MCPL-1.1.0.exe`,
  });
  assert.equal(resolveDesktopDownloadUrl(e.DOWNLOAD_URL, e.LEGACY_DOWNLOAD_URL), `${HOST}/download/MCPL-1.1.0.exe`);
  assert.equal(buildReleaseManifest(e).artifact.name, "MCPL-1.1.0.exe");
});

test("目前線上設定（1.0.8.11／未設 manifest 變數）不會輸出 manifest", () => {
  // 這就是部署當下的 wrangler.toml 狀態：新欄位不存在，舊客戶端行為完全不變。
  const live = {
    LATEST_VERSION: "1.0.8.11",
    DOWNLOAD_URL: `${HOST}/download/MCPL-1.0.8.exe`,
    LEGACY_DOWNLOAD_URL: `${HOST}/download/MCPL-1.0.8.exe`,
    UPDATE_SHA256: SHA,
    UPDATE_BUILD_ID: "1.0.8.11",
  };
  assert.equal(buildReleaseManifest(live), null);
});
