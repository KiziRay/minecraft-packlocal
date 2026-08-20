// modpack-i18n Cloudflare Worker
//
// 主要職責：
//  1. GET  /api/desktop/latest   → 桌面版更新檢查（回最新版本 + 下載連結）
//  2. GET/POST /turnstile        → Cloudflare 真人驗證與短效憑證
//  3. POST /v1/chat/completions  → 驗證 Discord + Turnstile 後代理 AI
//  4. /download、/tm、/glossary  → R2 免安裝 EXE 與共享翻譯資料
//
// 為什麼要代理而不是把金鑰編進 exe：
//  - 金鑰若進 exe，任何人反編譯就能抽出，開發者的免費額度幾天內被刷爆。
//  - 代理讓金鑰只存在 Worker secret，且可限流／隨時切換／統計用量。
//
// 客戶端在使用者「沒有自填金鑰」時走這裡；使用者自填金鑰則直連上游，不經本 Worker。

import {
  completeTurnstile,
  renderTurnstile,
  startTurnstile,
  turnstileConfigured,
  turnstileMissingNames,
  turnstileStatus,
} from "./turnstile.mjs";
import {
  cleanupShares,
  shareDownload,
  shareMpuComplete,
  shareMpuCreate,
  shareMpuPart,
  shareOgImage,
  shareUpload,
} from "./share.mjs";
import { GLOSSARY_MAX_ZH_LEN, TM_MAX_ZH_LEN, tmCanUse, tmMerge, tmZhAcceptable } from "./tm.mjs";
import {
  cleanupReports,
  reportDownload,
  reportMpuComplete,
  reportMpuCreate,
  reportMpuPart,
} from "./report.mjs";

import { submitFeedback } from "./feedback.mjs";
import { corsHeaders } from "./cors.mjs";
import { isSafeOutboundUrl } from "./security.mjs";

const JSON_HEADERS = { "content-type": "application/json; charset=utf-8" };
const SHARED_USAGE_TTL = 604800;
const PERSONAL_USAGE_TTL = 172800;
const CONTRIBUTE_DAILY_LIMIT = 60;

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    // CORS 預檢（WebView 內其實同源，但保險起見）
    if (request.method === "OPTIONS") {
      return new Response(null, { headers: corsHeaders(request) });
    }

    if (url.pathname === "/api/desktop/latest" && request.method === "GET") {
      return latest(env);
    }

    // 免安裝 EXE 下載：直接從 R2 串流。/download/<檔名>
    if (url.pathname.startsWith("/download/") && (request.method === "GET" || request.method === "HEAD")) {
      return download(url, env, request.method === "HEAD", request);
    }

    if (url.pathname === "/turnstile" && request.method === "GET") {
      return renderTurnstile(request, env);
    }
    if (url.pathname === "/api/turnstile/start" && request.method === "POST") {
      const access = await authorizeManagedIdentity(request, env);
      if (!access.ok) return access.response;
      return startTurnstile(request, env, access.userId);
    }
    if (url.pathname === "/api/turnstile/verify" && request.method === "POST") {
      return completeTurnstile(request, env);
    }

    if (url.pathname === "/api/managed/usage" && request.method === "GET") {
      return managedUsage(request, env);
    }
    if (url.pathname === "/api/managed/gp-reward" && request.method === "POST") {
      return managedGpReward(request, env);
    }
    if (url.pathname === "/api/feedback/submit" && request.method === "POST") {
      return submitFeedback(request, env);
    }

    if (url.pathname === "/v1/chat/completions" && request.method === "POST") {
      return proxyChat(request, env);
    }

    // 共享翻譯記憶（社群）：keyed by (模組, key, 原文) 的雜湊，存 R2、依模組分片。
    if (url.pathname === "/tm/lookup" && request.method === "POST") {
      return tmLookup(request, env);
    }
    if (url.pathname === "/tm/contribute" && request.method === "POST") {
      return gatedContribute(request, env, tmContribute);
    }
    if (url.pathname === "/glossary/lookup" && request.method === "POST") {
      return glossaryLookup(request, env);
    }
    if (url.pathname === "/glossary/contribute" && request.method === "POST") {
      return gatedContribute(request, env, glossaryContribute);
    }

    if (url.pathname === "/api/report/mpu-create" && request.method === "POST") {
      return gatedShare(request, env, reportMpuCreate);
    }
    if (url.pathname === "/api/report/mpu-part" && request.method === "PUT") {
      return gatedShare(request, env, (req, workerEnv, _userId) => reportMpuPart(req, workerEnv, url));
    }
    if (url.pathname === "/api/report/mpu-complete" && request.method === "POST") {
      return gatedShare(request, env, reportMpuComplete);
    }
    if (url.pathname.startsWith("/report/") && (request.method === "GET" || request.method === "HEAD")) {
      return reportDownload(url, env, request.method === "HEAD");
    }

    // 分享檔使用獨立的 SHARES R2 bucket，不會寫入安裝檔或翻譯記憶。
    if (url.pathname === "/api/share/upload" && request.method === "POST") {
      return gatedShare(request, env, shareUpload);
    }
    if (url.pathname === "/api/share/mpu-create" && request.method === "POST") {
      return gatedShare(request, env, shareMpuCreate);
    }
    if (url.pathname === "/api/share/mpu-part" && request.method === "PUT") {
      return gatedShare(request, env, (req, workerEnv, _userId) => shareMpuPart(req, workerEnv, url));
    }
    if (url.pathname === "/api/share/mpu-complete" && request.method === "POST") {
      return gatedShare(request, env, shareMpuComplete);
    }
    if (url.pathname.startsWith("/s/") && (request.method === "GET" || request.method === "HEAD")) {
      return shareDownload(url, env, request.method === "HEAD");
    }
    if (url.pathname === "/share-og.png" && (request.method === "GET" || request.method === "HEAD")) {
      return shareOgImage(request.method === "HEAD");
    }

    // 健康檢查
    if (url.pathname === "/" || url.pathname === "/health") {
      // hasKey：代管金鑰是否已正確設定（只回布林，不洩漏值）
      // turnstile*：保留欄位供舊版相容；P0 起代管閘門不再依賴 Turnstile。
      const turnstile = turnstileStatus(env);
      return json({
        ok: true,
        service: "modpack-i18n",
        version: env.LATEST_VERSION,
        hasKey: !!(env.DEEPSEEK_KEY && String(env.DEEPSEEK_KEY).trim()),
        usageBound: !!env.USAGE,
        reportNotifyConfigured: !!(env.DISCORD_REPORT_WEBHOOK && String(env.DISCORD_REPORT_WEBHOOK).trim()),
        feedbackNotifyConfigured: !!(env.DISCORD_FEEDBACK_WEBHOOK && String(env.DISCORD_FEEDBACK_WEBHOOK).trim()),
        toolUpdateNotifyConfigured: !!(env.DISCORD_TOOL_UPDATE_WEBHOOK && String(env.DISCORD_TOOL_UPDATE_WEBHOOK).trim()),
        joinNotifyConfigured: !!(env.DISCORD_JOIN_WEBHOOK && String(env.DISCORD_JOIN_WEBHOOK).trim()),
        authGate: "discord",
        turnstileReady: turnstileConfigured(env),
        turnstile: { ...turnstile, enforced: false },
        turnstileMissing: turnstileMissingNames(env),
        translationsBound: await estimateTranslationsBound(env),
      });
    }

    return json({ error: "not found" }, 404);
  },
  async scheduled(_event, env) {
    await maybeNotifyToolUpdateOncePerVersion(env);
    await cleanupShares(env);
    await cleanupReports(env);
  },
};

// ───────────────────────── 更新端點 ─────────────────────────

function latest(env) {
  return json({
    version: env.LATEST_VERSION || "0.0.0",
    url: env.DOWNLOAD_URL || "",
    notes: env.RELEASE_NOTES || "",
    sha256: env.UPDATE_SHA256 || env.INSTALLER_SHA256 || "",
  });
}

function nextUtcMidnightIso() {
  const d = new Date();
  const next = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate() + 1, 0, 0, 0));
  return next.toISOString();
}

/** ISO 8601 週（UTC、週一為週首）→ `YYYY-Www`。 */
export function utcIsoWeek(date = new Date()) {
  const tmp = new Date(Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate()));
  tmp.setUTCDate(tmp.getUTCDate() + 4 - (tmp.getUTCDay() || 7));
  const yearStart = new Date(Date.UTC(tmp.getUTCFullYear(), 0, 1));
  const weekNo = Math.ceil(((tmp - yearStart) / 86400000 + 1) / 7);
  return `${tmp.getUTCFullYear()}-W${String(weekNo).padStart(2, "0")}`;
}

/** 下週一 00:00:00.000Z（共享額度重置時刻）。 */
export function nextUtcWeekStartIso(date = new Date()) {
  const d = new Date(Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate()));
  const isoDow = d.getUTCDay() === 0 ? 7 : d.getUTCDay();
  const daysToAdd = 8 - isoDow;
  return new Date(
    Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate() + daysToAdd, 0, 0, 0, 0)
  ).toISOString();
}

export function sharedUsageKey(week = utcIsoWeek()) {
  return `usage:shared:${week}`;
}

export function isSharedWeeklyQuotaExhausted(spent, budget) {
  return budget > 0 && spent >= budget;
}

/**
 * KV 讀改寫（非真正 CAS）：寫前再讀一次，降低並行 double-spend。
 * @returns {{ ok: true, spent: number } | { ok: false, spent: number }}
 */
export async function tryIncrementUsageKv(kv, key, delta, ttl, maxTotal = 0) {
  const readSpent = async () => parseInt((await kv.get(key)) || "0", 10);
  let spent = await readSpent();
  let next = spent + delta;
  if (maxTotal > 0 && next > maxTotal) {
    return { ok: false, spent };
  }
  const again = await readSpent();
  if (again !== spent) {
    spent = again;
    next = spent + delta;
    if (maxTotal > 0 && next > maxTotal) {
      return { ok: false, spent };
    }
  }
  await kv.put(key, String(next), { expirationTtl: ttl });
  return { ok: true, spent: next };
}

function splitReleaseNotes(notes) {
  const raw = String(notes || "").trim();
  if (!raw) return [];
  return raw
    .split(/[\n;；]+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

/** Discord webhook embed payload（版本更新公告）。 */
export function buildToolUpdateDiscordPayload(version, releaseNotes, downloadUrl) {
  const items = splitReleaseNotes(releaseNotes);
  if (!items.length) return null;
  const v = String(version || "").trim();
  if (!v) return null;

  const url = String(downloadUrl || "").trim();
  const description = items.map((s) => `• ${s}`).join("\n").slice(0, 4000);

  /** @type {Record<string, unknown>} */
  const embed = {
    title: `MCPL v${v} 更新`,
    description,
    color: 0x35c5c9,
    footer: { text: "模組包翻譯工具 · ZeitFrei" },
    timestamp: new Date().toISOString(),
  };

  if (url) {
    embed.url = url;
    embed.fields = [
      {
        name: "下載",
        value: `[MCPL-${v}.exe](${url})`,
        inline: false,
      },
    ];
  }

  return { embeds: [embed] };
}

async function maybeNotifyToolUpdateOncePerVersion(env) {
  const hook = env?.DISCORD_TOOL_UPDATE_WEBHOOK && String(env.DISCORD_TOOL_UPDATE_WEBHOOK).trim();
  if (!hook) return;
  if (!env?.USAGE) return;

  const version = String(env.LATEST_VERSION || "").trim();
  if (!version) return;

  const key = `tool_update_notify:${version}`;
  const hit = await env.USAGE.get(key);
  if (hit) return;

  const payload = buildToolUpdateDiscordPayload(version, env.RELEASE_NOTES, env.DOWNLOAD_URL);
  if (!payload) return;

  let resp;
  try {
    if (!isSafeOutboundUrl(hook)) return;
    resp = await fetch(hook, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
      signal: AbortSignal.timeout(8000),
    });
  } catch (_) {
    return;
  }
  if (!resp?.ok) return;
  try {
    await env.USAGE.put(key, "1", { expirationTtl: 180 * 24 * 60 * 60 });
  } catch (_) {
    /* ignore */
  }
}

async function managedUsage(request, env) {
  if (!env?.USAGE) return json({ ok: false, error: "usage not configured" }, 503, request);
  const access = await authorizeManagedIdentity(request, env);
  if (!access.ok) return access.response;

  const day = utcDay();
  const week = utcIsoWeek();
  const sharedKey = sharedUsageKey(week);
  const sharedSpent = parseInt((await env.USAGE.get(sharedKey)) || "0", 10);
  const userSpent = parseInt((await env.USAGE.get(`usage:user:${day}:${access.userId}`)) || "0", 10);

  const sharedBudget = parseInt(env.WEEKLY_SHARED_TOKEN_BUDGET || "0", 10);
  const userBudget = await effectiveUserBudget(env, access.userId);

  return json(
    {
      ok: true,
      day,
      sharedSpent,
      sharedBudget,
      sharedPeriod: "week",
      sharedWeek: week,
      sharedResetAtUtc: nextUtcWeekStartIso(),
      userSpent,
      userBudget,
      userPeriod: "day",
      resetAtUtc: nextUtcMidnightIso(),
    },
    200,
    request
  );
}

/** 個人今日總額度 = 基礎上限 +（已領 GP 加成）。 */
export async function effectiveUserBudget(env, userId) {
  const base = parseInt(env.PER_USER_DAILY_TOKEN_BUDGET || "0", 10);
  if (!env?.USAGE || !userId) return base;
  const bonus = parseInt(env.GP_REWARD_BONUS || "0", 10);
  if (bonus <= 0) return base;
  const claimed = await env.USAGE.get(`gp_reward:${userId}`);
  return claimed ? base + bonus : base;
}

/** Discord join 公告 embed／純文字（供 webhook 與測試）。 */
export function renderDiscordJoinContent(userId, displayName) {
  const id = String(userId || "").trim();
  const name = String(displayName || id || "使用者")
    .replace(/[\n\r@<>]/g, "")
    .trim()
    .slice(0, 80);
  if (!/^\d{5,25}$/.test(id)) return null;
  return `<https://discord.com/users/${id}|${name || id}> 通過官方伺服器驗證，開始使用 MCPL 代管功能。`;
}

/** 會員驗證成功後，每 user／日最多通知一次（需 USAGE KV + secret）。 */
export async function maybeNotifyDiscordJoinOncePerDay(userId, displayName, env) {
  const hook = env?.DISCORD_JOIN_WEBHOOK && String(env.DISCORD_JOIN_WEBHOOK).trim();
  if (!hook || !env?.USAGE) return;
  if (!isSafeOutboundUrl(hook)) return;

  const content = renderDiscordJoinContent(userId, displayName);
  if (!content) return;

  const day = utcDay();
  const key = `join_notify:${day}:${userId}`;
  if (await env.USAGE.get(key)) return;

  let resp;
  try {
    resp = await fetch(hook, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ content }),
      signal: AbortSignal.timeout(8000),
    });
  } catch (_) {
    return;
  }
  if (!resp?.ok) return;
  try {
    await env.USAGE.put(key, "1", { expirationTtl: PERSONAL_USAGE_TTL });
  } catch (_) {
    /* ignore */
  }
}

async function recordContributeAttempt(env, userId) {
  if (!env?.USAGE || !userId) return { ok: true };
  const day = utcDay();
  const key = `contribute:day:${day}:${userId}`;
  const count = parseInt((await env.USAGE.get(key)) || "0", 10);
  if (count >= CONTRIBUTE_DAILY_LIMIT) {
    return { ok: false, error: "contribute rate limited" };
  }
  await env.USAGE.put(key, String(count + 1), { expirationTtl: PERSONAL_USAGE_TTL });
  return { ok: true };
}

/** proxyChat 個人上限判斷（spent 已達 effectiveBudget 即擋）。 */
export function isUserDailyQuotaExhausted(spent, effectiveBudget) {
  return effectiveBudget > 0 && spent >= effectiveBudget;
}

async function gatedContribute(request, env, handler) {
  const access = await authorizeManagedIdentity(request, env);
  if (!access.ok) return access.response;
  const limited = await recordContributeAttempt(env, access.userId);
  if (!limited.ok) {
    return json({ error: limited.error, type: "rate_limited" }, 429, request);
  }
  return handler(request, env);
}

async function managedGpReward(request, env) {
  if (!env?.USAGE) return json({ ok: false, error: "usage not configured" }, 503, request);
  const access = await authorizeManagedIdentity(request, env);
  if (!access.ok) return access.response;

  const userId = access.userId;
  const gpKey = `gp_reward:${userId}`;
  const already = await env.USAGE.get(gpKey);
  if (already) {
    return json({ ok: false, error: "already_claimed" }, 200);
  }

  const granted = parseInt(env.GP_REWARD_BONUS || "500000", 10);
  try {
    await env.USAGE.put(gpKey, "1");
  } catch (_) {
    return json({ ok: false, error: "gp_reward write failed" }, 500);
  }

  return json({ ok: true, granted });
}

// ───────────────────────── 共享翻譯記憶（R2，依模組分片）─────────────────────────
//
// 儲存：TRANSLATIONS R2 的 tm/v1/<namespace>.json.gz 是精確鍵，
// tm/v2/global.json.gz 是跨模組候選；不與更新檔 DOWNLOADS 混用。
// 多數決命中：跨包 ≥2 票、同包或 pack.* ≥1；不再永久 conflict 凍結。
// 只存匿名文字與語境，不存本機路徑、Discord 身分或整合包檔案。

const TM_MAX_ITEMS = 5000;
const TM_SHARD_CAP = 200000; // 單模組分片最多條數（防惡意灌爆）
const TM_GLOBAL_CAP = 300000;
const GLOSSARY_MAX_ITEMS = 5000;
const GLOSSARY_CAP = 300000;

/** /health 用：粗估共享 TM 分片數（R2 list，失敗回 null） */
async function estimateTranslationsBound(env) {
  try {
    if (!env.TRANSLATIONS) return null;
    let cursor;
    let count = 0;
    do {
      const listed = await env.TRANSLATIONS.list({
        prefix: "tm/",
        limit: 1000,
        cursor,
      });
      count += (listed.objects || []).length;
      cursor = listed.truncated ? listed.cursor : undefined;
      if (count >= 5000) break; // 健康檢查上限，避免掃太久
    } while (cursor);
    return count;
  } catch {
    return null;
  }
}

function tmShardKey(ns) {
  return `tm/v1/${ns}.json.gz`;
}

// gzip 壓縮／解壓（省 R2 容量：繁中 JSON 通常縮到 1/3 以下）
async function gzipBytes(str) {
  const cs = new CompressionStream("gzip");
  const w = cs.writable.getWriter();
  w.write(new TextEncoder().encode(str));
  w.close();
  return new Uint8Array(await new Response(cs.readable).arrayBuffer());
}
async function gunzipToStr(buf) {
  const ds = new DecompressionStream("gzip");
  const w = ds.writable.getWriter();
  w.write(new Uint8Array(buf));
  w.close();
  return new TextDecoder().decode(await new Response(ds.readable).arrayBuffer());
}
function tmValidNs(s) {
  return typeof s === "string" && s.length >= 1 && s.length <= 64 && /^[a-z0-9_.\-]+$/.test(s);
}
function tmValidKh(s) {
  return typeof s === "string" && /^[0-9a-f]{16,64}$/.test(s);
}
async function tmReadShard(env, ns) {
  const obj = await env.TRANSLATIONS?.get(tmShardKey(ns));
  if (!obj) return null;
  try {
    const buf = await obj.arrayBuffer();
    return JSON.parse(await gunzipToStr(buf));
  } catch (_) {
    return null;
  }
}

async function tmReadGlobal(env) {
  const obj = await env.TRANSLATIONS?.get("tm/v2/global.json.gz");
  if (!obj) return {};
  try {
    const buf = await obj.arrayBuffer();
    return JSON.parse(await gunzipToStr(buf));
  } catch (_) {
    return {};
  }
}

async function tmLookup(request, env) {
  if (!env.TRANSLATIONS) return json({ hits: {} });
  let body;
  try {
    body = await request.json();
  } catch (_) {
    return json({ error: "bad json" }, 400);
  }
  const items = Array.isArray(body.items) ? body.items.slice(0, TM_MAX_ITEMS) : [];
  const byNs = new Map();
  const queries = new Map();
  for (const it of items) {
    if (!it || !tmValidNs(it.ns) || !tmValidKh(it.kh)) continue;
    const ctx = typeof it.ctx === "string" ? it.ctx.slice(0, 64) : "";
    const sk = tmValidKh(it.sk) ? it.sk : "";
    const pk = validPackKey(it.pk) ? it.pk : "";
    if (!byNs.has(it.ns)) byNs.set(it.ns, new Map());
    byNs.get(it.ns).set(it.kh, { ctx, sk, pk });
    queries.set(it.kh, { ctx, sk, pk, ns: it.ns });
  }
  const hits = {};
  const nss = [...byNs.keys()];
  const CONC = 8;
  for (let i = 0; i < nss.length; i += CONC) {
    await Promise.all(
      nss.slice(i, i + CONC).map(async (ns) => {
        const shard = await tmReadShard(env, ns);
        if (!shard) return;
        for (const [kh, query] of byNs.get(ns)) {
          const zh = tmCanUse(shard[kh], query.ctx, query.pk, ns);
          if (zh) hits[kh] = zh;
        }
      })
    );
  }
  const missing = [...queries.entries()].filter(([kh]) => !hits[kh]);
  if (missing.length) {
    const global = await tmReadGlobal(env);
    for (const [kh, query] of missing) {
      if (!query.sk) continue;
      const zh = tmCanUse(global[query.sk], query.ctx, query.pk, query.ns);
      if (zh) hits[kh] = zh;
    }
  }
  return json({ hits });
}

async function tmContribute(request, env) {
  if (!env.TRANSLATIONS) return json({ ok: false, accepted: 0 });
  let body;
  try {
    body = await request.json();
  } catch (_) {
    return json({ error: "bad json" }, 400);
  }
  const items = Array.isArray(body.items) ? body.items.slice(0, TM_MAX_ITEMS) : [];
  const byNs = new Map();
  const globalEntries = new Map();
  for (const it of items) {
    if (!it || !tmValidNs(it.ns) || !tmValidKh(it.kh) || !tmValidKh(it.sk)) continue;
    const zh = typeof it.zh === "string" ? it.zh.trim() : "";
    if (!tmZhAcceptable(zh, TM_MAX_ZH_LEN)) continue;
    const record = {
      zh,
      ctx: typeof it.ctx === "string" ? it.ctx.slice(0, 64) : "",
      packs: validPackKey(it.pk) ? { [it.pk]: typeof it.pn === "string" ? it.pn.slice(0, 120) : "" } : {},
    };
    if (!byNs.has(it.ns)) byNs.set(it.ns, new Map());
    byNs.get(it.ns).set(it.kh, record);
    tmMerge(globalEntries, it.sk, record);
  }
  let accepted = 0;
  let conflicts = 0;
  for (const [ns, entries] of byNs) {
    const shard = (await tmReadShard(env, ns)) || {};
    let changed = false;
    for (const [kh, next] of entries) {
      if (Object.keys(shard).length >= TM_SHARD_CAP && !(kh in shard)) continue;
      const result = tmMerge(shard, kh, next);
      if (result === "accepted") {
        changed = true;
        accepted++;
      } else if (result === "variant") {
        changed = true;
        conflicts++;
      }
    }
    // 只有真的有新條目才寫（避免重複寫入）；寫的是 gzip 後的位元組（省容量）
    if (changed) {
      const gz = await gzipBytes(JSON.stringify(shard));
      await env.TRANSLATIONS.put(tmShardKey(ns), gz, {
        httpMetadata: { contentType: "application/gzip" },
      });
    }
  }
  if (globalEntries.size) {
    const global = await tmReadGlobal(env);
    let changed = false;
    for (const [sk, next] of globalEntries) {
      if (Object.keys(global).length >= TM_GLOBAL_CAP && !(sk in global)) continue;
      const result = tmMerge(global, sk, next);
      if (result === "accepted") {
        accepted++;
        changed = true;
      } else if (result === "variant") {
        conflicts++;
        changed = true;
      }
    }
    if (changed) {
      const gz = await gzipBytes(JSON.stringify(global));
      await env.TRANSLATIONS.put("tm/v2/global.json.gz", gz, {
        httpMetadata: { contentType: "application/gzip" },
      });
    }
  }
  return json({ ok: true, accepted, conflicts });
}

// ───────────────────────── 共享術語表 ─────────────────────────
// 與 TM 共用多數決：跨包 ≥2 票、同包 1 票；不再永久 conflict 凍結。
function glossaryKey() {
  return "glossary/v1/global.json.gz";
}

async function readGlossary(env) {
  const object = await env.TRANSLATIONS?.get(glossaryKey());
  if (!object) return {};
  try {
    return JSON.parse(await gunzipToStr(await object.arrayBuffer()));
  } catch (_) {
    return {};
  }
}

function validGlossaryHash(value) {
  return typeof value === "string" && /^[0-9a-f]{16,64}$/.test(value);
}

function validPackKey(value) {
  return typeof value === "string" && /^[0-9a-f]{16,64}$/.test(value);
}

async function glossaryLookup(request, env) {
  if (!env.TRANSLATIONS) return json({ hits: {} });
  let body;
  try { body = await request.json(); } catch (_) { return json({ error: "bad json" }, 400); }
  const items = Array.isArray(body.items) ? body.items.slice(0, GLOSSARY_MAX_ITEMS) : [];
  const queries = new Map();
  for (const item of items) {
    if (!item || !validGlossaryHash(item.gh)) continue;
    const pk = validPackKey(item.pk) ? item.pk : "";
    queries.set(item.gh, { pk, ctx: typeof item.ctx === "string" ? item.ctx.slice(0, 64) : "" });
  }
  const glossary = await readGlossary(env);
  const hits = {};
  for (const [gh, query] of queries) {
    const zh = tmCanUse(glossary[gh], query.ctx, query.pk, "");
    if (zh) hits[gh] = zh;
  }
  return json({ hits });
}

async function glossaryContribute(request, env) {
  if (!env.TRANSLATIONS) return json({ ok: false, accepted: 0, conflicts: 0 });
  let body;
  try { body = await request.json(); } catch (_) { return json({ error: "bad json" }, 400); }
  const items = Array.isArray(body.items) ? body.items.slice(0, GLOSSARY_MAX_ITEMS) : [];
  const glossary = await readGlossary(env);
  let accepted = 0;
  let conflicts = 0;
  let changed = false;
  for (const item of items) {
    if (!item || !validGlossaryHash(item.gh) || !validPackKey(item.pk)) continue;
    const zh = typeof item.zh === "string" ? item.zh.trim() : "";
    const pn = typeof item.pn === "string" ? item.pn.trim().slice(0, 120) : "";
    if (!tmZhAcceptable(zh, GLOSSARY_MAX_ZH_LEN) || !pn) continue;
    const result = tmMerge(glossary, item.gh, {
      zh,
      ctx: typeof item.ctx === "string" ? item.ctx.slice(0, 64) : "",
      packs: { [item.pk]: pn },
    });
    if (result === "accepted") {
      accepted++;
      changed = true;
    } else if (result === "variant") {
      conflicts++;
      changed = true;
    }
  }
  if (changed) {
    const gz = await gzipBytes(JSON.stringify(glossary));
    await env.TRANSLATIONS.put(glossaryKey(), gz, { httpMetadata: { contentType: "application/gzip" } });
  }
  return json({ ok: true, accepted, conflicts });
}

// ───────────────────────── 免安裝 EXE 下載（R2）─────────────────────────

async function download(url, env, headOnly, request) {
  if (!env.DOWNLOADS) {
    return json({ error: "downloads not configured" }, 503);
  }
  // 只允許取檔名，擋掉 ../ 之類的路徑穿越。
  const name = decodeURIComponent(url.pathname.slice("/download/".length));
  if (!name || name.includes("/") || name.includes("..") || name.includes("\\")) {
    return json({ error: "bad object name" }, 400);
  }
  const obj = await env.DOWNLOADS.get(name);
  if (!obj) {
    return json({ error: "not found" }, 404);
  }
  const headers = new Headers();
  obj.writeHttpMetadata(headers);
  headers.set("etag", obj.httpEtag);
  headers.set("content-length", String(obj.size));
  // 讓瀏覽器／下載器視為附件，用原檔名。
  headers.set("content-disposition", `attachment; filename*=UTF-8''${encodeURIComponent(name)}`);
  if (!headers.has("content-type")) {
    headers.set("content-type", "application/octet-stream");
  }
  Object.assign(headers, corsHeaders(request));
  return new Response(headOnly ? null : obj.body, { headers });
}

// ───────────────────────── 一日分享檔（獨立 R2）─────────────────────────

async function gatedShare(request, env, fn) {
  if (!env.SHARES) return json({ error: "share storage not configured" }, 503);
  const access = await authorizeManagedAi(request, env);
  if (!access.ok) return access.response;
  return fn(request, env, access.userId);
}

// ───────────────────────── AI 代理 ─────────────────────────

/** 只允許 json_object；其他 shape 一律忽略。 */
export function normalizeResponseFormat(value) {
  if (value === "json_object") {
    return { type: "json_object" };
  }
  if (
    value &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    Object.keys(value).length === 1 &&
    value.type === "json_object"
  ) {
    return { type: "json_object" };
  }
  return undefined;
}

/** 只接受有限 number，夾在 1..8192。 */
export function clampCompletionTokens(value) {
  if (typeof value !== "number" || !Number.isFinite(value)) return undefined;
  return Math.min(8192, Math.max(1, Math.floor(value)));
}

/** DeepSeek 思考模式：enabled／disabled；非法則 undefined。 */
export function normalizeThinking(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const type = value.type;
  if (type === "enabled" || type === "disabled") return { type };
  return undefined;
}

/** 組裝轉發上游的聊天補全 body（白名單欄位）。 */
export function buildChatForwardBody(body, env) {
  const forward = {
    model: env.UPSTREAM_MODEL || "deepseek-v4-flash",
    messages: body.messages,
    temperature: typeof body.temperature === "number" ? body.temperature : 0.1,
  };
  const responseFormat = normalizeResponseFormat(body.response_format);
  if (responseFormat) forward.response_format = responseFormat;
  const maxTokens = clampCompletionTokens(body.max_tokens);
  if (maxTokens !== undefined) forward.max_tokens = maxTokens;
  const maxCompletionTokens = clampCompletionTokens(body.max_completion_tokens);
  if (maxCompletionTokens !== undefined) forward.max_completion_tokens = maxCompletionTokens;
  // 強制關閉思考模式；忽略客戶端 thinking（防貴模型／長推理）。
  forward.thinking = { type: "disabled" };
  return forward;
}

async function proxyChat(request, env) {
  const access = await authorizeManagedAi(request, env);
  if (!access.ok) return access.response;

  // trim：擋掉空字串／只有換行的 secret（貼進遮罩提示常見的坑），也避免結尾換行害上游 401。
  const key = env.DEEPSEEK_KEY && String(env.DEEPSEEK_KEY).trim();
  if (!key) {
    // secret 未設或值為空：明確告訴客戶端這是「服務端未就緒」，不是使用者金鑰問題。
    return json(
      { error: { message: "managed translation not configured", type: "server_not_ready" } },
      503
    );
  }

  // 共享週總量 + 個人日上限：超過就回 429。
  const sharedBudget = parseInt(env.WEEKLY_SHARED_TOKEN_BUDGET || "0", 10);
  const userBudget = parseInt(env.PER_USER_DAILY_TOKEN_BUDGET || "0", 10);
  const week = utcIsoWeek();
  const sharedKey = sharedUsageKey(week);
  if (sharedBudget > 0 && env.USAGE) {
    const sharedSpent = parseInt((await env.USAGE.get(sharedKey)) || "0", 10);
    if (isSharedWeeklyQuotaExhausted(sharedSpent, sharedBudget)) {
      return json(
        {
          error: {
            message: "managed shared weekly quota exhausted",
            type: "insufficient_quota",
          },
        },
        429,
        request
      );
    }
  }
  if (userBudget > 0 && env.USAGE) {
    const userDayKey = `usage:user:${utcDay()}:${access.userId}`;
    const spent = parseInt((await env.USAGE.get(userDayKey)) || "0", 10);
    const effectiveBudget = await effectiveUserBudget(env, access.userId);
    if (isUserDailyQuotaExhausted(spent, effectiveBudget)) {
      return json(
        {
          error: {
            message: "managed personal daily quota exhausted; use custom API or disable AI",
            type: "insufficient_quota",
          },
        },
        429,
        request
      );
    }
  }

  let body;
  try {
    const declaredSize = parseInt(request.headers.get("content-length") || "0", 10);
    if (declaredSize > 250000) {
      return json({ error: { message: "request too large", type: "invalid_request" } }, 413);
    }
    const raw = await request.text();
    if (raw.length > 250000) {
      return json({ error: { message: "request too large", type: "invalid_request" } }, 413);
    }
    body = JSON.parse(raw);
  } catch (_) {
    return json({ error: { message: "invalid json body" } }, 400);
  }

  if (!validTranslationMessages(body.messages)) {
    return json({ error: { message: "invalid translation messages", type: "invalid_request" } }, 400);
  }

  // 只允許聊天補全所需欄位轉發，並鎖定模型（避免被拿去打別的昂貴模型）。
  const forward = buildChatForwardBody(body, env);

  const upstream = (env.UPSTREAM_BASE || "https://api.deepseek.com").replace(/\/+$/, "");
  let resp;
  try {
    resp = await fetch(upstream + "/v1/chat/completions", {
      method: "POST",
      headers: {
        Authorization: "Bearer " + key,
        "content-type": "application/json",
      },
      body: JSON.stringify(forward),
    });
  } catch (e) {
    return json({ error: { message: "upstream unreachable" } }, 502);
  }

  const text = await resp.text();

  // 記帳（成功才計；用量以 usage.total_tokens 為準，取不到就估）。
  if (resp.ok && env.USAGE && (sharedBudget > 0 || userBudget > 0)) {
    try {
      const used = estimateTokens(text, forward);
      if (sharedBudget > 0) {
        await tryIncrementUsageKv(env.USAGE, sharedKey, used, SHARED_USAGE_TTL, sharedBudget);
      }
      if (userBudget > 0) {
        const userDayKey = `usage:user:${utcDay()}:${access.userId}`;
        await tryIncrementUsageKv(env.USAGE, userDayKey, used, PERSONAL_USAGE_TTL);
      }
    } catch (_) {
      /* 記帳失敗不影響翻譯 */
    }
  }

  // 原樣回傳上游狀態與內容，客戶端既有的 402/429 判斷即可運作。
  return new Response(text, {
    status: resp.status,
    headers: { ...JSON_HEADERS, ...corsHeaders(request) },
  });
}

async function authorizeManagedAi(request, env) {
  // P0：Turnstile 整體多餘 → 僅 Discord 會員門檻；真人驗證不再擋代管 AI／分享。
  return authorizeManagedIdentity(request, env);
}

async function authorizeManagedIdentity(request, env) {
  const expectedProtocol = String(env.MANAGED_AI_PROTOCOL || "3");
  if (request.headers.get("x-zeitfrei-ai-protocol") !== expectedProtocol) {
    return {
      ok: false,
      response: json(
        { error: { message: "client upgrade required", type: "client_upgrade_required" } },
        426
      ),
    };
  }

  const session = String(request.headers.get("x-zeitfrei-session") || "").trim();
  if (!session || session.length < 40 || session.length > 8192 || !/^[A-Za-z0-9+/=_-]+$/.test(session)) {
    return {
      ok: false,
      response: json({ error: { message: "discord login required", type: "login_required" } }, 401),
    };
  }

  const authBase = String(env.AUTH_BASE_URL || "https://cloud.zeitfrei.uk").replace(/\/+$/, "");
  let account;
  try {
    const response = await fetch(`${authBase}/api/check-upload`, {
      headers: { Cookie: `cf_storage_v3_session=${session}` },
      signal: AbortSignal.timeout(8000),
    });
    if (response.status === 401) {
      return {
        ok: false,
        response: json({ error: { message: "discord login expired", type: "login_required" } }, 401),
      };
    }
    if (!response.ok) throw new Error("account check failed");
    account = await response.json();
  } catch (_) {
    return {
      ok: false,
      response: json({ error: { message: "login verification unavailable", type: "auth_unavailable" } }, 503),
    };
  }

  const userId = String((account && account.user_id) || "");
  if (!/^\d{5,25}$/.test(userId)) {
    return {
      ok: false,
      response: json({ error: { message: "invalid discord session", type: "login_required" } }, 401),
    };
  }

  const displayName = String(
    (account && (account.nickname || account.username || account.display_name || account.name)) || ""
  )
    .trim()
    .slice(0, 80);

  const guild = await verifyGuildMembership(userId, authBase, env);
  if (!guild.ok) {
    if (guild.type === "guild_required") {
      return {
        ok: false,
        response: json(
          { error: { message: "official discord membership required", type: "guild_required" } },
          403,
          request
        ),
      };
    }
    return {
      ok: false,
      response: json(
        { error: { message: "membership verification unavailable", type: "auth_unavailable" } },
        503,
        request
      ),
    };
  }

  await maybeNotifyDiscordJoinOncePerDay(userId, displayName, env);

  return { ok: true, userId, displayName };
}

/** 正向會員快取 TTL（秒）。略過重複 member-tier，減輕高並行閃斷。 */
export const GUILD_OK_TTL_SECONDS = 900;

export function guildOkKvKey(userId) {
  return `guild_ok:${userId}`;
}

export function guildOkCacheRequest(userId, authBase) {
  const base = String(authBase || "https://cloud.zeitfrei.uk").replace(/\/+$/, "");
  return new Request(`${base}/__guild_ok_cache/${encodeURIComponent(userId)}`);
}

/** 讀正向會員快取：優先 USAGE KV，否則 caches.default。 */
export async function readGuildOkCached(userId, env, authBase) {
  if (env && env.USAGE) {
    try {
      const hit = await env.USAGE.get(guildOkKvKey(userId));
      return hit === "1";
    } catch (_) {
      return false;
    }
  }
  try {
    if (typeof caches !== "undefined" && caches.default) {
      const cached = await caches.default.match(guildOkCacheRequest(userId, authBase));
      return !!(cached && cached.ok);
    }
  } catch (_) {
    /* ignore */
  }
  return false;
}

/** 寫入正向會員快取（僅 inGuild===true 時呼叫）。 */
export async function writeGuildOkCached(userId, env, authBase) {
  if (env && env.USAGE) {
    try {
      await env.USAGE.put(guildOkKvKey(userId), "1", { expirationTtl: GUILD_OK_TTL_SECONDS });
    } catch (_) {
      /* ignore */
    }
    return;
  }
  try {
    if (typeof caches !== "undefined" && caches.default) {
      const resp = new Response("1", {
        status: 200,
        headers: { "cache-control": `public, max-age=${GUILD_OK_TTL_SECONDS}` },
      });
      await caches.default.put(guildOkCacheRequest(userId, authBase), resp);
    }
  } catch (_) {
    /* ignore */
  }
}

/**
 * 查 Discord 會員。快取命中略過 fetch；明確非會員軟重試 1 次；
 * HTTP／JSON 失敗 → auth_unavailable（勿誤標 guild_required）。
 * @returns {{ ok: true } | { ok: false, type: "guild_required"|"auth_unavailable" }}
 */
export async function verifyGuildMembership(userId, authBase, env, fetchImpl = fetch) {
  if (await readGuildOkCached(userId, env, authBase)) {
    return { ok: true };
  }

  async function fetchMembershipOnce() {
    const response = await fetchImpl(
      `${String(authBase).replace(/\/+$/, "")}/api/member-tier/${encodeURIComponent(userId)}`,
      { signal: AbortSignal.timeout(8000) }
    );
    if (!response.ok) throw new Error("membership check failed");
    return await response.json();
  }

  try {
    let membership = await fetchMembershipOnce();
    if (!membership || membership.inGuild !== true) {
      membership = await fetchMembershipOnce();
    }
    if (!membership || membership.inGuild !== true) {
      return { ok: false, type: "guild_required" };
    }
    await writeGuildOkCached(userId, env, authBase);
    return { ok: true };
  } catch (_) {
    return { ok: false, type: "auth_unavailable" };
  }
}

function validTranslationMessages(messages) {
  if (!Array.isArray(messages) || messages.length < 1 || messages.length > 4) return false;
  let total = 0;
  for (const message of messages) {
    if (!message || !["system", "user"].includes(message.role) || typeof message.content !== "string") {
      return false;
    }
    total += message.content.length;
    if (total > 180000) return false;
  }
  return messages.some((message) => message.role === "user");
}

function estimateTokens(text, forward) {
  try {
    const v = JSON.parse(text);
    if (v && v.usage && typeof v.usage.total_tokens === "number") {
      return v.usage.total_tokens;
    }
  } catch (_) {
    /* fall through */
  }
  // 粗估：輸入字元數 / 3
  const chars = JSON.stringify(forward.messages || "").length;
  return Math.ceil(chars / 3);
}

// ───────────────────────── 工具 ─────────────────────────

function utcDay() {
  return new Date().toISOString().slice(0, 10); // YYYY-MM-DD (UTC)
}

function json(obj, status = 200, request) {
  return new Response(JSON.stringify(obj), {
    status,
    // no-store：版本檢查等 API 一定要拿到最新值，不能被邊緣或客戶端快取住舊版本資訊。
    headers: { ...JSON_HEADERS, "cache-control": "no-store", ...corsHeaders(request) },
  });
}
