/**
 * B5a-1 問題回報：遊戲內顯示問題＋子分類、送出前預覽、案件編號或如實說明。不改 Worker。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import {
  DETAIL_MAX,
  DISPLAY_CATEGORY,
  DISPLAY_KINDS,
  SERVER_SUMMARIES,
  buildIssuePayload,
  describeIssueResult,
} from "./issue-report.js";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel) => readFileSync(join(here, rel), "utf8").replace(/\r\n/g, "\n");
const info = { toolVersion: "1.1.1", mcVersion: "1.20.1", packName: "ATM10" };

test("送出的概要一定在 Worker 與後端的清單裡（這一批不改 Worker）", async () => {
  const worker = await import("../../worker/src/issue-thread.mjs");
  assert.deepEqual([...SERVER_SUMMARIES], worker.ISSUE_SUMMARIES);
  const rust = read("../../src-tauri/src/engine/issue_report.rs");
  for (const s of SERVER_SUMMARIES) assert.ok(rust.includes(`"${s}"`), `後端缺 ${s}`);
  for (const kind of DISPLAY_KINDS) {
    const out = buildIssuePayload({ summary: DISPLAY_CATEGORY, displayKind: kind, cause: "不確定", info });
    assert.ok(worker.ISSUE_SUMMARIES.includes(out.summary), `${kind} 對映到不存在的概要`);
    assert.equal(worker.parseIssueBody({ summary: out.summary, cause: out.cause, detail: out.detail }).ok, true, `${kind} 會被 Worker 退回`);
  }
});

test("遊戲內顯示問題：子分類寫進說明開頭；「某些字沒翻」對映到翻譯結果", () => {
  const box = buildIssuePayload({ summary: DISPLAY_CATEGORY, displayKind: "方框", cause: "操作後立刻發生", info });
  assert.equal(box.ok, true);
  assert.equal(box.summary, "套用後遊戲異常");
  assert.match(box.detail, /^【遊戲內顯示問題：方框】/);
  const missing = buildIssuePayload({ summary: DISPLAY_CATEGORY, displayKind: "某些字沒翻", cause: "不確定" });
  assert.equal(missing.summary, "翻譯結果不對或沒翻到");
  const noKind = buildIssuePayload({ summary: DISPLAY_CATEGORY, cause: "不確定" });
  assert.equal(noKind.ok, false);
  assert.match(noKind.error, /哪一種顯示問題/);
});

test("附帶資料：MC 版本預設附上、包名預設不附；取消就不送；工具版本後端一律附上，預覽照實寫", () => {
  const out = buildIssuePayload({
    summary: "介面或縮放",
    cause: "不確定",
    attach: { mc: true, pack: false },
    info,
  });
  assert.match(out.detail, /附帶：Minecraft 1\.20\.1$/);
  assert.ok(!out.detail.includes("ATM10"));
  assert.match(out.preview, /工具版本 1\.1\.1/);
  const none = buildIssuePayload({ summary: "介面或縮放", cause: "不確定", attach: {}, info });
  assert.equal(none.detail, null);
  const html = read("../index.html");
  assert.ok(!html.includes('id="issue-attach-tool"'), "關不掉的就不給勾選");
  assert.match(html, /id="issue-attach-mc" checked/);
  assert.match(html, /id="issue-attach-pack" \/>/);
});

test("送出前預覽＝實際送出的內容", () => {
  const out = buildIssuePayload({
    summary: DISPLAY_CATEGORY,
    displayKind: "閃退",
    cause: "操作後立刻發生",
    detail: "開世界的時候直接關掉遊戲",
    attach: { mc: true },
    info,
  });
  assert.ok(out.preview.includes(`問題概要：${out.summary}`));
  assert.ok(out.preview.includes(`詳細說明：${out.detail}`));
  assert.match(out.preview, /Discord 帳號/);
});

test("說明太短照舊擋下；總長不超過 Worker 上限", () => {
  assert.match(buildIssuePayload({ summary: "其他", cause: "不確定", detail: "太短" }).error, /超過十個字/);
  const long = buildIssuePayload({ summary: DISPLAY_CATEGORY, displayKind: "亂碼", cause: "不確定", detail: "字".repeat(800), attach: { mc: true, pack: true }, info });
  assert.ok(Array.from(long.detail).length <= DETAIL_MAX);
  assert.match(long.detail, /附帶：/);
});

test("結果就地說明：成功寫案件編號；失敗如實說明並提供開啟 Discord", () => {
  const ok = describeIssueResult({ ok: true, caseId: "MCPL-42" });
  assert.equal(ok.text, "已送出，案件編號：MCPL-42。");
  assert.equal(ok.showDiscord, false);
  const fail = describeIssueResult({ ok: false, message: "站長聯絡通道暫時離線" }, { copied: true });
  assert.match(fail.text, /^沒有送出：站長聯絡通道暫時離線/);
  assert.match(fail.text, /已複製/);
  assert.equal(fail.showDiscord, true);
});

test("浮層不再用「已送出」「無法自動送出」對話框", () => {
  const app = read("../app.js");
  assert.ok(!app.includes('title: "已送出"'));
  assert.ok(!app.includes('title: "無法自動送出"'));
  assert.ok(app.includes("buildIssuePayload("));
  assert.ok(app.includes("describeIssueResult("));
});
