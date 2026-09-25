// 把目前工作分支的「檔案樹」發佈成公開分支上的一個 commit，並推送到 GitHub。
//
//   node scripts/publish-batch.mjs "batch 0: 基準、設定視窗、舊資料相容"          只檢查並建立本機 commit
//   node scripts/publish-batch.mjs "batch 0: 基準、設定視窗、舊資料相容" --push   建立後推送
//
// 為什麼用檔案樹而不是直接推工作分支：工作分支的舊歷史含有不公開的內部文件，
// 直接推會把它們帶進公開 repo。這裡只拿當下的樹，公開分支因此只有乾淨的批次 commit。
// 規則來源：2.0.0 批次計畫「批次紀律」第 5 點。
import { spawnSync } from "node:child_process";

const REMOTE = "mc";
const PUBLIC_BRANCH = "public/2.0.0";
const REMOTE_BRANCH = "release/2.0.0";
const BASE = `${REMOTE}/main`;

const message = process.argv[2];
const PUSH = process.argv.includes("--push");
if (!message || message.startsWith("--")) {
  console.error('用法：node scripts/publish-batch.mjs "batch N: 主題" [--push]');
  process.exit(2);
}

function git(args, { allowFail = false } = {}) {
  const r = spawnSync("git", args, { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
  if (r.status !== 0 && !allowFail) {
    console.error(`git ${args.join(" ")} 失敗：\n${r.stderr}`);
    process.exit(1);
  }
  return { ok: r.status === 0, out: (r.stdout || "").trim() };
}

// 1. 工作樹必須乾淨，確保發佈的就是已提交的內容。
if (git(["status", "--porcelain"]).out) {
  console.error("工作樹有未提交的修改，先提交再發佈。");
  process.exit(1);
}

const tree = git(["rev-parse", "HEAD^{tree}"]).out;
const head = git(["rev-parse", "--short", "HEAD"]).out;

// 2. 禁傳檔案：內部工作區、日誌、執行檔、本機使用者資料、標為不公開的文件。
const forbidden = [
  /^\.claudecode\//,
  /^logs\//,
  /\.exe$/i,
  /^modpack-i18n-data\//,
  /^ARCHITECTURE\.md$/,
  /^AGENTS\.md$/,
  /^CLAUDE\.md$/,
  /^docs\/(DEVELOPMENT|AI-HANDOFF|SEARCH-MAP|LOCALIZE|AUDIT|EXTENDING|HARDENING|API-COMMANDS)/,
  /^docs\/COMMUNITY\.md$/,
  /^docs\/MCPL-/,
  /(^|\/)\.env(\.|$)/,
];
const files = git(["-c", "core.quotepath=off", "ls-tree", "-r", "--name-only", tree]).out.split("\n");
const badFiles = files.filter((f) => forbidden.some((re) => re.test(f)));

// 3. 機密掃描（在要發佈的樹上跑 git grep）。排除本腳本自己：它的原始碼含有要找的樣式。
const secretPatterns = [
  "ghp_[A-Za-z0-9]{30,}",
  "github_pat_[A-Za-z0-9_]{30,}",
  "(^|[^A-Za-z0-9_-])sk-[A-Za-z0-9_-]{20,}",
  "-----BEGIN [A-Z ]*PRIVATE KEY-----",
  "xox[baprs]-[A-Za-z0-9-]{10,}",
  "discord(app)?\\.com/api/webhooks/[0-9]+/[A-Za-z0-9_-]{20,}",
  "\\.mcpl-keys",
];
const hits = [];
for (const pattern of secretPatterns) {
  const r = git(["grep", "-I", "-n", "-E", "-e", pattern, tree, "--", ".", ":(exclude)scripts/publish-batch.mjs"], { allowFail: true });
  if (r.out) hits.push(`【${pattern}】\n${r.out.split("\n").slice(0, 10).join("\n")}`);
}

if (badFiles.length || hits.length) {
  if (badFiles.length) console.error(`禁傳檔案 ${badFiles.length} 個：\n${badFiles.slice(0, 30).join("\n")}`);
  if (hits.length) console.error(`疑似機密：\n${hits.join("\n\n")}`);
  console.error("\n發佈中止，沒有建立任何 commit。");
  process.exit(1);
}
console.log(`ok  禁傳檔案與機密掃描（${files.length} 個檔案）`);

// 4. 在公開分支上建立 commit（第一次從 mc/main 起算）。
git(["fetch", REMOTE, "main", REMOTE_BRANCH], { allowFail: true });
let parent = git(["rev-parse", "--verify", "-q", PUBLIC_BRANCH], { allowFail: true }).out;
if (!parent) {
  parent = git(["rev-parse", "--verify", "-q", `${REMOTE}/${REMOTE_BRANCH}`], { allowFail: true }).out
    || git(["rev-parse", BASE]).out;
}
if (git(["rev-parse", `${parent}^{tree}`]).out === tree) {
  console.log("公開分支的內容已經和目前相同，不需要新 commit。");
} else {
  const body = `${message}\n\n來源：工作分支 ${head}\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`;
  const commit = spawnSync("git", ["commit-tree", tree, "-p", parent, "-F", "-"], {
    input: body,
    encoding: "utf8",
  });
  if (commit.status !== 0) {
    console.error(commit.stderr);
    process.exit(1);
  }
  const sha = commit.stdout.trim();
  git(["update-ref", `refs/heads/${PUBLIC_BRANCH}`, sha]);
  console.log(`ok  ${PUBLIC_BRANCH} → ${sha.slice(0, 10)}`);
}

// 5. 推送（只推公開分支，不推工作分支、不用 force）。
if (PUSH) {
  git(["push", REMOTE, `${PUBLIC_BRANCH}:refs/heads/${REMOTE_BRANCH}`]);
  console.log(`ok  已推送到 ${REMOTE}/${REMOTE_BRANCH}`);
} else {
  console.log("（未推送；加上 --push 才會推送）");
}
