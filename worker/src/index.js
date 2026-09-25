// modpack-i18n Cloudflare Worker
//
// 主要職責：
//  1. GET  /api/desktop/latest   → 桌面版更新檢查（回最新版本 + 下載連結）
//  2. GET/POST /turnstile        → Cloudflare 真人驗證與短效憑證
//  3. /download、/tm、/glossary  → R2 免安裝 EXE 與共享翻譯資料
//  4. /api/share、/api/report    → 24h 分享與診斷回報（獨立 SHARES bucket）
//
// AI 翻譯不再由本 Worker 代管；客戶端走自訂 API、GPT 或本機模型。
// Discord 會籍仍用於分享、診斷回報與（之後）本地模型檔下載閘門。

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
import { issueThreadHealth, submitIssueThread } from "./issue-thread.mjs";
import { corsHeaders } from "./cors.mjs";
import { isSafeOutboundUrl } from "./security.mjs";
import { localLlmBound, localLlmFile, localLlmManifest } from "./local-llm.mjs";

const JSON_HEADERS = { "content-type": "application/json; charset=utf-8" };
const SHARED_USAGE_TTL = 604800;
const PERSONAL_USAGE_TTL = 172800;
const DEFAULT_CONTRIBUTE_USER_DAILY_LIMIT = 120;
const DEFAULT_CONTRIBUTE_IP_DAILY_LIMIT = 40;
const DEFAULT_LOOKUP_IP_MINUTE_LIMIT = 240;
const LOOKUP_USAGE_TTL = 120;

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    // CORS 預檢（WebView 內其實同源，但保險起見）
    if (request.method === "OPTIONS") {
      return new Response(null, { headers: corsHeaders(request) });
    }

    if (url.pathname === "/api/desktop/latest" && request.method === "GET") {
      return latest(env, request);
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

    if (url.pathname === "/api/feedback/submit" && request.method === "POST") {
      return submitFeedback(request, env);
    }
    // 設定健康檢查：部署後用瀏覽器打開就知道 secret 有沒有設對，不回傳任何密鑰
    if (url.pathname === "/api/issue-thread/health" && request.method === "GET") {
      return issueThreadHealth(env);
    }
    if (url.pathname === "/api/issue-thread" && request.method === "POST") {
      const access = await authorizeManagedIdentity(request, env);
      if (!access.ok) return access.response;
      return submitIssueThread(request, env, access);
    }
    if (url.pathname === "/api/local-llm/manifest" && request.method === "GET") {
      return localLlmManifest(request, env, authorizeManagedIdentity);
    }
    if (url.pathname.startsWith("/api/local-llm/file/") && (request.method === "GET" || request.method === "HEAD")) {
      return localLlmFile(request, env, url, authorizeManagedIdentity);
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
      return gatedShare(request, env, (req, workerEnv, userId) => reportMpuPart(req, workerEnv, url, userId));
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
      return gatedShare(request, env, (req, workerEnv, userId) => shareMpuPart(req, workerEnv, url, userId));
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
      // 部署後打 /health 即觸發版本更新 Discord 公告（每版 KV 防重一次）。
      await maybeNotifyToolUpdateOncePerVersion(env);
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
        tmGlobal: await estimateTmGlobalHealth(env),
        localLlmBound: await localLlmBound(env),
      });
    }

    return json({ error: "not found" }, 404);
  },
  async scheduled(_event, env) {
    await cleanupShares(env);
    await cleanupReports(env);
  },
};

// ───────────────────────── 更新端點 ─────────────────────────

async function latest(env, request) {
  // 版本一上線：首次被查詢時發 Discord（非 hourly cron；與 /health 共用 KV 防重）
  try {
    await maybeNotifyToolUpdateOncePerVersion(env);
  } catch (_) {
    /* ignore */
  }
  const baseVersion = String(env.LATEST_VERSION || "0.0.0").trim();
  const requestedBuild = new URL(request.url).searchParams.get("build") || "";
  const currentBuild = String(env.UPDATE_BUILD_ID || "").trim();
  const payload = {
    version: desktopUpdateVersionForBuild(baseVersion, requestedBuild, currentBuild),
    // 舊版更新器只接受 MCPL-<major>.<minor>.<patch>.exe（恰好三段）；不可回四段檔名。
    url: resolveDesktopDownloadUrl(env.DOWNLOAD_URL, env.LEGACY_DOWNLOAD_URL),
        notes: [env.RELEASE_NOTES, env.RELEASE_NOTES_EXTRA].filter((value) => value && String(value).trim()).join("；"),
    sha256: env.UPDATE_SHA256 || env.INSTALLER_SHA256 || "",
  };
  // 設定齊備才附上 manifest。缺任何一項就整個不輸出，維持舊行為——
  // 半套的 manifest 比沒有更危險（客戶端會拿它當信任依據）。
  const manifest = buildReleaseManifest(env);
  if (manifest) payload.manifest = manifest;
  return json(payload);
}

/**
 * 由設定組出 release manifest（桌面端 `engine/release_manifest.rs` 的對應合約）。
 *
 * 回 `null` 代表「這個部署還沒設定 manifest」，`latest()` 就完全不輸出該欄位。
 * 舊客戶端本來就忽略未知欄位，新客戶端看不到 manifest 時沿用舊版本比對路徑。
 *
 * 注意：這裡**不做簽章**。私鑰永遠不進 Worker secret，簽章是離線人工步驟；
 * 未簽章的 manifest 在客戶端只會被 test 之類的非正式通道接受，stable 一律拒絕。
 */
export function buildReleaseManifest(env) {
  const channel = String(env.RELEASE_CHANNEL || "").trim().toLowerCase();
  const version = String(env.MANIFEST_VERSION || "").trim();
  const buildId = String(env.MANIFEST_BUILD_ID || "").trim();
  const releasedAt = String(env.MANIFEST_RELEASED_AT || "").trim();
  const commit = String(env.MANIFEST_COMMIT || "").trim();
  const buildTime = String(env.MANIFEST_BUILD_TIME || "").trim();
  const bytes = Number(env.MANIFEST_ARTIFACT_BYTES || 0);
  const sha256 = String(env.UPDATE_SHA256 || env.INSTALLER_SHA256 || "").trim();
  const url = resolveDesktopDownloadUrl(env.DOWNLOAD_URL, env.LEGACY_DOWNLOAD_URL);

  if (!["stable", "beta", "canary", "test"].includes(channel)) return null;
  if (!/^\d+\.\d+\.\d+$/.test(version)) return null;
  if (!buildId || !releasedAt || !commit || !buildTime) return null;
  if (!/^[0-9a-f]{64}$/i.test(sha256)) return null;
  if (!Number.isInteger(bytes) || bytes <= 0) return null;
  if (!url || !url.toLowerCase().startsWith("https://")) return null;

  const manifest = {
    schemaVersion: 1,
    channel,
    version,
    buildId,
    releasedAt,
    provenance: {
      commit,
      // 沒有明說乾淨就當 dirty：失效安全方向朝「擋掉 stable 發布」。
      dirty: String(env.MANIFEST_DIRTY || "true").trim().toLowerCase() !== "false",
      buildTime,
      builder: String(env.MANIFEST_BUILDER || "local").trim(),
    },
    artifact: { name: url.split("/").pop(), url, sha256, bytes },
    compat: { minClientVersion: String(env.MANIFEST_MIN_CLIENT || "0.0.0").trim() },
  };

  const signature = String(env.MANIFEST_SIGNATURE || "").trim();
  const keyId = String(env.MANIFEST_KEY_ID || "").trim();
  if (signature && keyId) {
    manifest.trust = { algorithm: "ed25519-v1", keyId, signature };
  }
  return manifest;
}

/** 同版維護更新：新建置帶 build ID，舊建置收到可比較的維護版號。跨版（build 不符）直接回 LATEST。 */
export function desktopUpdateVersionForBuild(baseVersion, requestedBuild, currentBuild) {
  const base = String(baseVersion || "0.0.0").trim() || "0.0.0";
  const requested = String(requestedBuild || "").trim();
  const current = String(currentBuild || "").trim();
  if (current && requested === current) return base;
  if (!requested) return base;
  if (current && requested !== current) return base;
  const segments = base.split(".").filter(Boolean);
  return segments.length >= 4 ? base : `${base}.1`;
}

/** 舊版桌面更新器只接受 MCPL-x.y.z.exe（恰好三段數字）。 */
export function isLegacyCompatibleMcplDownloadUrl(url) {
  const name = String(url || "").split("/").pop()?.toLowerCase() || "";
  return /^mcpl-\d+\.\d+\.\d+\.exe$/.test(name);
}

/** 永遠回傳舊版更新器可接受的直連；四段檔名一律改走 LEGACY。 */
export function resolveDesktopDownloadUrl(downloadUrl, legacyDownloadUrl) {
  const legacy = String(legacyDownloadUrl || "").trim();
  const primary = String(downloadUrl || "").trim();
  if (legacy && isLegacyCompatibleMcplDownloadUrl(legacy)) return legacy;
  if (primary && isLegacyCompatibleMcplDownloadUrl(primary)) return primary;
  return legacy || primary;
}

export function desktopDownloadUrlForBuild(downloadUrl, legacyDownloadUrl, requestedBuild, currentBuild) {
  const compatible = resolveDesktopDownloadUrl(downloadUrl, legacyDownloadUrl);
  const requested = String(requestedBuild || "").trim();
  const current = String(currentBuild || "").trim();
  if (!requested) return compatible;
  if (current && requested === current) return compatible;
  if (current && requested !== current) return compatible;
  return compatible;
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


/** 個人今日總額度 = 基礎上限 +（已領 GP 加成）。 */
export async function effectiveUserBudget(env, userId) {
  const base = parseInt(env.PER_USER_DAILY_TOKEN_BUDGET || "0", 10);
  if (!env?.USAGE || !userId) return base;
  const bonus = parseInt(env.GP_REWARD_BONUS || "0", 10);
  if (bonus <= 0) return base;
  const claimed = await env.USAGE.get(`gp_reward:${userId}`);
  return claimed ? base + bonus : base;
}

/** Discord 頭像 CDN；無 hash 時用預設頭像。 */
export function discordAvatarUrl(userId, avatarHash) {
  const id = String(userId || "").trim();
  if (!/^\d{5,25}$/.test(id)) return null;
  const hash = String(avatarHash || "").trim();
  if (/^[a-fA-F0-9_]{16,128}$/.test(hash)) {
    const ext = hash.startsWith("a_") ? "gif" : "png";
    return `https://cdn.discordapp.com/avatars/${id}/${hash}.${ext}?size=128`;
  }
  let index = 0;
  try {
    index = Number((BigInt(id) >> 22n) % 6n);
  } catch (_) {
    index = 0;
  }
  return `https://cdn.discordapp.com/embed/avatars/${index}.png`;
}

/**
 * Discord join 公告 webhook payload（embed，無 content 以免 URL unfurl）。
 * 成員資訊放 author＋fields（勿只靠 description markdown 使用者連結，客戶端常不顯示）。
 * @param {string} userId
 * @param {string} [displayName]
 * @param {string} [avatarHash]
 */
export function buildDiscordJoinPayload(userId, displayName, avatarHash) {
  const id = String(userId || "").trim();
  const name = String(displayName || "")
    .replace(/[\n\r@<>]/g, "")
    .trim()
    .slice(0, 80);
  if (!/^\d{5,25}$/.test(id)) return null;
  const label = name || id;
  const profileUrl = `https://discord.com/users/${id}`;
  const icon = discordAvatarUrl(id, avatarHash);
  /** @type {Record<string, unknown>} */
  const embed = {
    author: {
      name: label.slice(0, 256),
      url: profileUrl,
      ...(icon ? { icon_url: icon } : {}),
    },
    title: "通過官方伺服器驗證",
    description: `${label} 開始使用 MCPL。`,
    color: 0x35c5c9,
    fields: [
      { name: "成員", value: label.slice(0, 256), inline: true },
      { name: "Discord ID", value: `\`${id}\``, inline: true },
    ],
    footer: { text: "模組包翻譯工具 · ZeitFrei" },
    timestamp: new Date().toISOString(),
  };
  if (icon) embed.thumbnail = { url: icon };
  return { embeds: [embed] };
}

/** @deprecated 測試／相容：回傳 join embed 的 description。 */
export function renderDiscordJoinContent(userId, displayName, avatarHash) {
  const payload = buildDiscordJoinPayload(userId, displayName, avatarHash);
  return payload?.embeds?.[0]?.description || null;
}

/** 會員驗證成功後，每 user／日最多通知一次（需 USAGE KV + secret）。 */
export async function maybeNotifyDiscordJoinOncePerDay(userId, displayName, env, avatarHash) {
  const hook = env?.DISCORD_JOIN_WEBHOOK && String(env.DISCORD_JOIN_WEBHOOK).trim();
  if (!hook || !env?.USAGE) return;
  if (!isSafeOutboundUrl(hook)) return;

  const payload = buildDiscordJoinPayload(userId, displayName, avatarHash);
  if (!payload) return;

  const day = utcDay();
  const key = `join_notify:${day}:${userId}`;
  if (await env.USAGE.get(key)) return;

  let resp;
  try {
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
    await env.USAGE.put(key, "1", { expirationTtl: PERSONAL_USAGE_TTL });
  } catch (_) {
    /* ignore */
  }
}

function parsePositiveInt(value, fallback) {
  const parsed = parseInt(String(value ?? ""), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function clientIpBucket(request) {
  const ip = String(request.headers.get("CF-Connecting-IP") || "").trim().slice(0, 128);
  return `ip:${ip || "unknown"}`;
}

async function recordContributeAttempt(env, bucketId, limit) {
  if (!env?.USAGE || !bucketId) return { ok: true };
  const day = utcDay();
  const key = `contribute:day:${day}:${bucketId}`;
  const count = parseInt((await env.USAGE.get(key)) || "0", 10);
  if (count >= limit) {
    return { ok: false, error: "contribute rate limited" };
  }
  await env.USAGE.put(key, String(count + 1), { expirationTtl: PERSONAL_USAGE_TTL });
  return { ok: true };
}

async function recordLookupAttempt(env, request, scope) {
  if (!env?.USAGE) return { ok: true };
  const limit = parsePositiveInt(env.LOOKUP_IP_MINUTE_LIMIT, DEFAULT_LOOKUP_IP_MINUTE_LIMIT);
  const minute = Math.floor(Date.now() / 60000);
  const key = `lookup:minute:${minute}:${scope}:${clientIpBucket(request)}`;
  const count = parseInt((await env.USAGE.get(key)) || "0", 10);
  if (count >= limit) return { ok: false, error: "lookup rate limited" };
  await env.USAGE.put(key, String(count + 1), { expirationTtl: LOOKUP_USAGE_TTL });
  return { ok: true };
}

/** 個人日額度是否已用盡（spent 已達 effectiveBudget）。 */
export function isUserDailyQuotaExhausted(spent, effectiveBudget) {
  return effectiveBudget > 0 && spent >= effectiveBudget;
}

async function gatedContribute(request, env, handler) {
  const protocol = requireManagedProtocol(request, env);
  if (!protocol.ok) return protocol.response;
  let bucketId = clientIpBucket(request);
  let limit = parsePositiveInt(env.CONTRIBUTE_IP_DAILY_LIMIT, DEFAULT_CONTRIBUTE_IP_DAILY_LIMIT);
  if (hasContributeSession(request)) {
    const access = await authorizeManagedIdentity(request, env);
    if (access.ok) {
      bucketId = `user:${access.userId}`;
      limit = parsePositiveInt(env.CONTRIBUTE_USER_DAILY_LIMIT, DEFAULT_CONTRIBUTE_USER_DAILY_LIMIT);
    }
  }
  const limited = await recordContributeAttempt(env, bucketId, limit);
  if (!limited.ok) {
    return json({ error: limited.error, type: "rate_limited" }, 429, request);
  }
  return handler(request, env);
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
  } catch (err) {
    await recordTranslationReadError(env, tmShardKey(ns), err);
    return null;
  }
}

async function tmReadGlobal(env) {
  const obj = await env.TRANSLATIONS?.get("tm/v2/global.json.gz");
  if (!obj) return {};
  try {
    const buf = await obj.arrayBuffer();
    return JSON.parse(await gunzipToStr(buf));
  } catch (err) {
    await recordTranslationReadError(env, "tm/v2/global.json.gz", err);
    return {};
  }
}

async function recordTranslationReadError(env, key, err) {
  const message = String(err?.message || err || "unknown").slice(0, 160);
  console.warn(`translation shard parse failed: ${key}: ${message}`);
  if (!env?.USAGE) return;
  try {
    await env.USAGE.put(
      `translations:parse_error:${utcDay()}:${key}`,
      JSON.stringify({ key, message, at: new Date().toISOString() }).slice(0, 500),
      { expirationTtl: PERSONAL_USAGE_TTL }
    );
  } catch (_) {
    /* logging must not break lookup/contribute */
  }
}

function tmGlobalSoftCap() {
  return Math.floor(TM_GLOBAL_CAP * 0.8);
}

function tmGlobalSoftCapExceeded(global) {
  return Object.keys(global || {}).length >= tmGlobalSoftCap();
}

async function estimateTmGlobalHealth(env) {
  try {
    if (!env.TRANSLATIONS) return null;
    const global = await tmReadGlobal(env);
    const items = Object.keys(global || {}).length;
    return {
      items,
      softCap: tmGlobalSoftCap(),
      hardCap: TM_GLOBAL_CAP,
      softCapExceeded: items >= tmGlobalSoftCap(),
    };
  } catch (_) {
    return null;
  }
}

async function tmLookup(request, env) {
  const limited = await recordLookupAttempt(env, request, "tm");
  if (!limited.ok) return json({ error: limited.error, type: "rate_limited" }, 429, request);
  if (!env.TRANSLATIONS) return lookupJson({ hits: {} }, 200, request);
  let body;
  try {
    body = await request.json();
  } catch (_) {
    return lookupJson({ error: "bad json" }, 400, request);
  }
  const items = Array.isArray(body.items) ? body.items.slice(0, TM_MAX_ITEMS) : [];
  const byNs = new Map();
  const queries = new Map();
  for (const it of items) {
    if (!it || !tmValidNs(it.ns) || !tmValidKh(it.kh)) continue;
    const ctx = typeof it.ctx === "string" ? it.ctx.slice(0, 64) : "";
    const sk = tmValidKh(it.sk) ? it.sk : "";
    const pk = validPackKey(it.pk) ? it.pk : "";
    const pks = Array.isArray(it.pks)
      ? it.pks.filter((value) => validPackKey(value)).slice(0, 16)
      : pk
        ? [pk]
        : [];
    // mv＝提供這個 namespace 的 mod 檔（含版本）。有帶的話，
    // 「不同整合包但同一個模組的同一版本」一票就能採用（見 tm.mjs tmCanUse）。
    const mv = validModIdentity(it.mv) ? it.mv : "";
    if (!byNs.has(it.ns)) byNs.set(it.ns, new Map());
    byNs.get(it.ns).set(it.kh, { ctx, sk, pk, pks, mv });
    queries.set(it.kh, { ctx, sk, pk, pks, mv, ns: it.ns });
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
          const zh = tmCanUse(
            shard[kh],
            query.ctx,
            query.pks.length ? query.pks : query.pk,
            ns,
            query.mv
          );
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
      const zh = tmCanUse(
        global[query.sk],
        query.ctx,
        query.pks.length ? query.pks : query.pk,
        query.ns,
        query.mv
      );
      if (zh) hits[kh] = zh;
    }
  }
  return lookupJson({ hits }, 200, request);
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
      // 記下這一票是在哪個 mod 版本下貢獻的，供跨整合包重用判定
      mods: validModIdentity(it.mv) ? { [it.mv]: 1 } : {},
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
    if (tmGlobalSoftCapExceeded(global)) {
      return json({ ok: true, accepted, conflicts, globalSkipped: true });
    }
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
  } catch (err) {
    await recordTranslationReadError(env, glossaryKey(), err);
    return {};
  }
}

function validGlossaryHash(value) {
  return typeof value === "string" && /^[0-9a-f]{16,64}$/.test(value);
}

function validPackKey(value) {
  return typeof value === "string" && /^[0-9a-f]{16,64}$/.test(value);
}

/**
 * mod 檔識別（含版本），例如 `create-1.20.1-0.5.1.f`。
 *
 * 這是客戶端送來的自由字串，會被存進 R2，所以在邊界收斂：
 * 只收小寫英數與 `. _ - +`，長度上限 120。不合格的一律當成沒有帶——
 * 那只會退回原本的兩票門檻，不會出錯。
 */
function validModIdentity(value) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 120 &&
    /^[0-9a-z._+-]+$/.test(value)
  );
}

async function glossaryLookup(request, env) {
  const limited = await recordLookupAttempt(env, request, "glossary");
  if (!limited.ok) return json({ error: limited.error, type: "rate_limited" }, 429, request);
  if (!env.TRANSLATIONS) return lookupJson({ hits: {} }, 200, request);
  let body;
  try { body = await request.json(); } catch (_) { return lookupJson({ error: "bad json" }, 400, request); }
  const items = Array.isArray(body.items) ? body.items.slice(0, GLOSSARY_MAX_ITEMS) : [];
  const queries = new Map();
  for (const item of items) {
    if (!item || !validGlossaryHash(item.gh)) continue;
    const pk = validPackKey(item.pk) ? item.pk : "";
    const pks = Array.isArray(item.pks)
      ? item.pks.filter((value) => validPackKey(value)).slice(0, 16)
      : pk
        ? [pk]
        : [];
    queries.set(item.gh, { pk, pks, ctx: typeof item.ctx === "string" ? item.ctx.slice(0, 64) : "" });
  }
  const glossary = await readGlossary(env);
  const hits = {};
  for (const [gh, query] of queries) {
    const zh = tmCanUse(glossary[gh], query.ctx, query.pks.length ? query.pks : query.pk, "");
    if (zh) hits[gh] = zh;
  }
  return lookupJson({ hits }, 200, request);
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
    if (Object.keys(glossary).length >= GLOSSARY_CAP && !(item.gh in glossary)) continue;
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
  const access = await authorizeManagedIdentity(request, env);
  if (!access.ok) return access.response;
  return fn(request, env, access.userId);
}


async function authorizeManagedAi(request, env) {
  // Discord 會籍閘門（分享／回報）；不再代理免費代管 AI。
  return authorizeManagedIdentity(request, env);
}

function requireManagedProtocol(request, env) {
  const expectedProtocol = String(env.MANAGED_AI_PROTOCOL || "3");
  if (request.headers.get("x-zeitfrei-ai-protocol") !== expectedProtocol) {
    return {
      ok: false,
      response: json(
        { error: { message: "client upgrade required", type: "client_upgrade_required" } },
        426,
        request
      ),
    };
  }
  return { ok: true };
}

function hasContributeSession(request) {
  const session = String(request.headers.get("x-zeitfrei-session") || "").trim();
  return !!session && session.length >= 40 && session.length <= 8192 && /^[A-Za-z0-9+/=_-]+$/.test(session);
}

/**
 * `x-zeitfrei-session` → 帳號資訊的短效快取。
 *
 * 這支函式原本每次呼叫都對 `AUTH_BASE_URL` 做一次跨站 `fetch`——單一使用者下載一次
 * 7.38 GB 的本地模型（32 MB 一段）要切成約 230 個 Range 請求，等於每次下載對外
 * 打 230 次帳號驗證，每次量到 150–250ms。這是本地模型「網速夠快卻只有個位數 MB/s」
 * 的主因之一：不是頻寬不夠，是每個分段開頭都要先付一次跨站握手的時間。
 *
 * key 用 session 的雜湊而非原始值：KV 鍵有 512 bytes 上限，session cookie 本身
 * 可以到 8192 字元；雜湊也避免把使用者的 session 明碼存進 KV。
 */
export const AUTH_SESSION_CACHE_TTL_SECONDS = 60;

export async function sessionCacheKey(session) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(session));
  const hex = [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
  return `session_ok:${hex}`;
}

export async function readSessionCached(session, env) {
  if (!env?.USAGE) return null;
  try {
    const raw = await env.USAGE.get(await sessionCacheKey(session));
    return raw ? JSON.parse(raw) : null;
  } catch (_) {
    return null;
  }
}

export async function writeSessionCached(session, account, env) {
  if (!env?.USAGE) return;
  try {
    await env.USAGE.put(await sessionCacheKey(session), JSON.stringify(account), {
      expirationTtl: AUTH_SESSION_CACHE_TTL_SECONDS,
    });
  } catch (_) {
    /* 快取寫入失敗不影響本次請求，下次照樣會走一次完整驗證 */
  }
}

async function authorizeManagedIdentity(request, env) {
  const protocol = requireManagedProtocol(request, env);
  if (!protocol.ok) return protocol;

  const session = String(request.headers.get("x-zeitfrei-session") || "").trim();
  if (!session || session.length < 40 || session.length > 8192 || !/^[A-Za-z0-9+/=_-]+$/.test(session)) {
    return {
      ok: false,
      response: json({ error: { message: "discord login required", type: "login_required" } }, 401),
    };
  }

  const authBase = String(env.AUTH_BASE_URL || "https://cloud.zeitfrei.uk").replace(/\/+$/, "");
  let account = await readSessionCached(session, env);
  if (!account) {
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
    await writeSessionCached(session, account, env);
  }

  const userId = String((account && account.user_id) || "");
  if (!/^\d{5,25}$/.test(userId)) {
    return {
      ok: false,
      response: json({ error: { message: "invalid discord session", type: "login_required" } }, 401),
    };
  }

  const displayName = String(
    (account &&
      (account.nickname ||
        account.global_name ||
        account.username ||
        account.display_name ||
        account.name ||
        account.user)) ||
      ""
  )
    .trim()
    .slice(0, 80);

  const avatarHash = String(
    (account && (account.avatar || account.avatar_hash || account.avatarHash)) || ""
  )
    .trim()
    .slice(0, 128);

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

  await maybeNotifyDiscordJoinOncePerDay(userId, displayName, env, avatarHash);

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

function lookupJson(obj, status = 200, request) {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { ...JSON_HEADERS, "cache-control": "private, max-age=60", ...corsHeaders(request) },
  });
}
