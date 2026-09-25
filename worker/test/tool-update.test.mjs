import test from "node:test";
import assert from "node:assert/strict";

import {
  buildToolUpdateDiscordPayload,
  desktopDownloadUrlForBuild,
  desktopUpdateVersionForBuild,
  isLegacyCompatibleMcplDownloadUrl,
  resolveDesktopDownloadUrl,
} from "../src/index.js";

test("desktopUpdateVersionForBuild 讓舊版取得同版維護更新且新版不循環", () => {
  assert.equal(desktopUpdateVersionForBuild("1.0.7", "1.0.7-guide-reader", "1.0.7-guide-reader"), "1.0.7");
  assert.equal(desktopUpdateVersionForBuild("1.0.7", "old-build", "1.0.7-guide-reader"), "1.0.7");
  assert.equal(desktopUpdateVersionForBuild("1.0.7", "", "1.0.7-guide-reader"), "1.0.7");
  assert.equal(desktopUpdateVersionForBuild("", "wrong", "1.0.7-guide-reader"), "0.0.0");
  assert.equal(desktopUpdateVersionForBuild("1.0.7.2", "", "1.0.7.2-quality-deferred"), "1.0.7.2");
  assert.equal(
    desktopUpdateVersionForBuild("1.0.7.2", "1.0.7.2-quality-deferred", "1.0.7.2-quality-deferred"),
    "1.0.7.2"
  );
  assert.equal(
    desktopUpdateVersionForBuild("1.0.8", "1.0.7.2-quality-deferred", "1.0.8"),
    "1.0.8"
  );
});

test("desktopDownloadUrlForBuild 讓舊版使用三段式相容別名", () => {
  const primary = "https://example.test/download/MCPL-1.0.7.2.exe";
  const legacy = "https://example.test/download/MCPL-1.0.7.exe";
  assert.equal(desktopDownloadUrlForBuild(primary, legacy, "", "1.0.7.2-quality-deferred"), legacy);
  assert.equal(
    desktopDownloadUrlForBuild(primary, legacy, "1.0.7.2-quality-deferred", "1.0.7.2-quality-deferred"),
    legacy
  );
  assert.equal(
    desktopDownloadUrlForBuild(primary, legacy, "1.0.7.2-quality-deferred", "1.0.8"),
    legacy
  );
  assert.equal(desktopDownloadUrlForBuild(primary, "", "old-build", "new-build"), primary);
});

test("resolveDesktopDownloadUrl 拒絕四段檔名改走 LEGACY", () => {
  const four = "https://example.test/download/MCPL-1.0.8.1.exe";
  const three = "https://example.test/download/MCPL-1.0.8.exe";
  assert.equal(isLegacyCompatibleMcplDownloadUrl(four), false);
  assert.equal(isLegacyCompatibleMcplDownloadUrl(three), true);
  assert.equal(resolveDesktopDownloadUrl(four, three), three);
  assert.equal(resolveDesktopDownloadUrl(four, ""), four);
});

test("buildToolUpdateDiscordPayload 產生 embed 而非純 content", () => {
  const payload = buildToolUpdateDiscordPayload(
    "1.0.3",
    "更新自動重開；額度指示",
    "https://example.test/download/MCPL-1.0.3.exe"
  );
  assert.ok(payload);
  assert.ok(Array.isArray(payload.embeds));
  assert.equal(payload.embeds.length, 1);
  assert.equal(payload.content, undefined);
  assert.equal(payload.embeds[0].title, "MCPL v1.0.3 更新");
  assert.match(payload.embeds[0].description, /更新自動重開/);
  assert.match(payload.embeds[0].description, /額度指示/);
  assert.equal(payload.embeds[0].url, "https://example.test/download/MCPL-1.0.3.exe");
  assert.equal(payload.embeds[0].fields[0].name, "下載");
});

test("buildToolUpdateDiscordPayload 無 notes 回 null", () => {
  assert.equal(buildToolUpdateDiscordPayload("1.0.3", "", "https://x"), null);
});
