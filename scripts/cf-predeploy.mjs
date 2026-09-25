// Cloudflare Worker 部署前／部署後檢查。
//
//   node scripts/cf-predeploy.mjs            部署前：語法與測試、凍結設定比對、線上與原始碼的版本比對
//   node scripts/cf-predeploy.mjs --post     部署後：用舊版客戶端的請求做線上冒煙測試
//   node scripts/cf-predeploy.mjs --release  只在正式發版（B10）使用：允許更新相關設定改變
//
// 規則來源：2.0.0 批次計畫「批次紀律」第 6 點。更新相關設定在正式發版前禁止變更，
// 否則仍在用舊版的玩家會被推送到錯的版本。
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const args = new Set(process.argv.slice(2));
const POST = args.has("--post");
const RELEASE = args.has("--release");
const BASE = "https://modpack-i18n.jolin34563.workers.dev";
const FREEZE_FILE = "worker/release-freeze.json";
const WRANGLER_FILE = "worker/wrangler.toml";
const TIMEOUT_MS = 15000;

const problems = [];
const fail = (msg) => problems.push(msg);

function readWranglerVars() {
  const text = readFileSync(WRANGLER_FILE, "utf8").replace(/^﻿/, "");
  const vars = {};
  let inVars = false;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (line.startsWith("[")) {
      inVars = line === "[vars]";
      continue;
    }
    if (!inVars || !line || line.startsWith("#")) continue;
    const m = line.match(/^([A-Z0-9_]+)\s*=\s*"(.*)"\s*$/);
    if (m) vars[m[1]] = m[2];
  }
  return vars;
}

function run(label, command) {
  const r = spawnSync(command, { encoding: "utf8", shell: true });
  if (r.status !== 0) fail(`${label} 失敗：\n${(r.stdout || "") + (r.stderr || "")}`.slice(0, 4000));
  else console.log(`ok  ${label}`);
}

async function getJson(path, init = {}) {
  const res = await fetch(BASE + path, { ...init, signal: AbortSignal.timeout(TIMEOUT_MS) });
  const type = res.headers.get("content-type") || "";
  const body = type.includes("json") ? await res.json() : await res.text();
  return { status: res.status, type, body };
}

async function preDeploy() {
  run("check:worker", "npm run check:worker");
  run("test:worker", "npm run test:worker");

  const vars = readWranglerVars();
  const freeze = JSON.parse(readFileSync(FREEZE_FILE, "utf8"));
  for (const key of freeze.frozenKeys) {
    const expected = freeze.values[key];
    if (vars[key] !== expected) {
      if (RELEASE) console.log(`注意 ${key} 已變更（--release 允許）：${expected} → ${vars[key]}`);
      else fail(`${key} 在正式發版前禁止變更：凍結值「${expected}」，目前「${vars[key]}」`);
    } else {
      console.log(`ok  凍結設定 ${key}`);
    }
  }

  try {
    const live = await getJson("/api/desktop/latest");
    if (live.status !== 200 || typeof live.body !== "object") {
      fail(`線上 /api/desktop/latest 回應異常：HTTP ${live.status}`);
    } else if (!RELEASE && live.body.version !== freeze.values.LATEST_VERSION) {
      fail(`線上版本 ${live.body.version} 與凍結值 ${freeze.values.LATEST_VERSION} 不一致，先查清楚線上是誰改的`);
    } else {
      console.log(`ok  線上版本 ${live.body.version}`);
    }
  } catch (e) {
    fail(`無法連到線上 Worker：${e.message}`);
  }
}

async function postDeploy() {
  const freeze = JSON.parse(readFileSync(FREEZE_FILE, "utf8"));
  try {
    const health = await getJson("/health");
    if (health.status !== 200) fail(`/health HTTP ${health.status}`);
    else console.log("ok  /health");

    // 舊版（1.0.8.11）客戶端的更新檢查請求。
    const latest = await getJson("/api/desktop/latest?build=1.0.8.11");
    if (latest.status !== 200 || typeof latest.body !== "object" || !latest.body.version) {
      fail(`舊版更新檢查異常：HTTP ${latest.status}`);
    } else if (!RELEASE && latest.body.version !== freeze.values.LATEST_VERSION) {
      fail(`部署後更新檢查回 ${latest.body.version}，應維持 ${freeze.values.LATEST_VERSION}`);
    } else {
      console.log(`ok  舊版更新檢查 → ${latest.body.version}`);
    }

    // 舊版客戶端的共享庫查詢格式（空查詢也必須回 JSON，不可回 5xx 或 HTML 錯誤頁）。
    const lookup = await getJson("/tm/lookup", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ items: [] }),
    });
    if (lookup.status >= 500 || !lookup.type.includes("json")) {
      fail(`舊版共享庫查詢異常：HTTP ${lookup.status}（${lookup.type}）`);
    } else {
      console.log(`ok  舊版共享庫查詢 HTTP ${lookup.status}`);
    }
  } catch (e) {
    fail(`部署後冒煙測試無法完成：${e.message}`);
  }
}

if (POST) await postDeploy();
else await preDeploy();

if (problems.length) {
  console.error(`\n檢查未通過（${problems.length} 項）：`);
  for (const p of problems) console.error(`- ${p}`);
  process.exit(1);
}
console.log(POST ? "\n部署後冒煙測試通過" : "\n部署前檢查通過");
