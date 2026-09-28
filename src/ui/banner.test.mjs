/**
 * B5a-1 橫幅（規格 §3.4）：N-01 有新版改橫幅、不與同意頁疊、測試版不出現；N-02 設定檔損壞。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import {
  MAX_VISIBLE_BANNERS,
  parentFolder,
  settingsHealthBanner,
  updateBannerDecision,
  visibleBanners,
} from "./banner.js";

const here = dirname(fileURLToPath(import.meta.url));
const available = { ok: true, updateAvailable: true, latest: "1.1.2", current: "1.1.1" };

test("N-01：有新版、同意頁已完成、非翻譯中 → 顯示「有新版 X（目前 Y）」＋更新", () => {
  const out = updateBannerDecision({ info: available, consentDone: true });
  assert.equal(out.kind, "show");
  assert.equal(out.banner.id, "N-01");
  assert.equal(out.banner.text, "有新版 1.1.2（目前 1.1.1）");
  assert.equal(out.banner.actionLabel, "更新");
  assert.ok(Array.from(out.banner.text).length <= 40);
});

test("N-01：同意頁還沒完成或翻譯中 → 先不顯示（晚點再判斷），不與同意頁疊", () => {
  assert.equal(updateBannerDecision({ info: available, consentDone: false }).kind, "defer");
  assert.equal(updateBannerDecision({ info: available, consentDone: true, busy: true }).kind, "defer");
});

test("N-01：關閉後這個版本不再出現；更新的版本會再出現", () => {
  assert.equal(updateBannerDecision({ info: available, consentDone: true, dismissedVersion: "1.1.2" }).kind, "none");
  assert.equal(
    updateBannerDecision({ info: { ...available, latest: "1.1.3" }, consentDone: true, dismissedVersion: "1.1.2" }).kind,
    "show"
  );
});

test("G0.4：測試版就算誤帶 updateAvailable 也不出現更新橫幅；已是最新版、連不上也不出現", () => {
  assert.equal(updateBannerDecision({ info: { ...available, testBuild: true }, consentDone: true }).kind, "none");
  assert.equal(updateBannerDecision({ info: { ok: true, updateAvailable: false }, consentDone: true }).kind, "none");
  assert.equal(updateBannerDecision({ info: { ok: false }, consentDone: true }).kind, "none");
  assert.equal(updateBannerDecision({ info: null, consentDone: true }).kind, "none");
});

test("同時最多 2 則，依序號優先", () => {
  const out = visibleBanners([
    { id: "N-02", text: "b" },
    { id: "N-05", text: "c" },
    { id: "N-01", text: "a" },
  ]);
  assert.equal(MAX_VISIBLE_BANNERS, 2);
  assert.deepEqual(out.map((b) => b.id), ["N-01", "N-02"]);
});

test("N-02：原檔保留才寫「原檔已保留」，並提供開啟所在資料夾", () => {
  const kept = settingsHealthBanner({ backupKept: true, folder: "D:/MCPL/工具設定.json.broken" });
  assert.equal(kept.id, "N-02");
  assert.equal(kept.text, "設定檔讀不出來，已先用預設值（原檔已保留）");
  assert.equal(kept.actionLabel, "開啟所在資料夾");
  const lost = settingsHealthBanner({ backupKept: false, folder: "D:/MCPL/工具設定.json" });
  assert.equal(lost.text, "設定檔讀不出來，已先用預設值");
  assert.equal(settingsHealthBanner(null), null);
  assert.equal(parentFolder("D:\\MCPL\\工具設定.json"), "D:\\MCPL");
});

test("update.js：自動檢查有新版時交給橫幅，不再直接跳更新視窗；手動檢查才開更新浮層", () => {
  const src = readFileSync(join(here, "../core/update.js"), "utf8");
  assert.ok(src.includes("onUpdateAvailable"), "自動檢查要交給橫幅");
  assert.match(src, /pendingUpdateTarget\(\{ interactive, hasBanner: typeof onUpdateAvailable === "function" \}\) === "banner"/);
});
