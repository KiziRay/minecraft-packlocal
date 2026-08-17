// 24 小時自解檔分享：短碼、落地頁、USAGE KV 配額與過期清理。
// 沒有 env.USAGE 時行為與舊版相容（長 token URL、不記次數）。

const JSON_HEADERS = { "content-type": "application/json; charset=utf-8" };

export const SHARE_PREFIX = "v1/";
export const SHARE_TTL_SECONDS = 24 * 60 * 60;
export const SHARE_DEFAULT_MAX_BYTES = 4 * 1024 * 1024 * 1024;
export const SHARE_DEFAULT_DAILY_LIMIT = 3;
export const SHARE_DEFAULT_ACTIVE_LIMIT = 2;
export const SHARE_DEFAULT_MPU_STALE_SECONDS = 60 * 60;
export const SHARE_SFX_DOWNLOAD_NAME = "模組包繁中翻譯自解檔.exe";
export const SHARE_ZIP_DOWNLOAD_NAME = "模組包繁中翻譯.zip";
export const SHARE_OG_TITLE = "繁體中文模組包翻譯工具";
export const SHARE_OG_DESCRIPTION = "讓模組包翻譯不再困難";
const SHORT_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

export function shareMaxBytes(env) {
  return Math.max(
    1,
    parseInt(env?.SHARE_MAX_BYTES || String(SHARE_DEFAULT_MAX_BYTES), 10) || SHARE_DEFAULT_MAX_BYTES
  );
}

export function shareDailyLimit(env) {
  return Math.max(
    1,
    parseInt(env?.SHARE_DAILY_LIMIT || String(SHARE_DEFAULT_DAILY_LIMIT), 10) || SHARE_DEFAULT_DAILY_LIMIT
  );
}

export function shareActiveLimit(env) {
  return Math.max(
    1,
    parseInt(env?.SHARE_ACTIVE_LIMIT || String(SHARE_DEFAULT_ACTIVE_LIMIT), 10) || SHARE_DEFAULT_ACTIVE_LIMIT
  );
}

export function shareMpuStaleSeconds(env) {
  return Math.max(
    60,
    parseInt(env?.SHARE_MPU_STALE_SECONDS || String(SHARE_DEFAULT_MPU_STALE_SECONDS), 10) ||
      SHARE_DEFAULT_MPU_STALE_SECONDS
  );
}

export function isLongShareToken(token) {
  return /^[A-Za-z0-9_-]{32,128}$/.test(String(token || ""));
}

export function isShortShareCode(token) {
  return /^[A-Za-z0-9]{8}$/.test(String(token || ""));
}

export function shareIdKey(code) {
  return `share:id:${code}`;
}

export function shareDayKey(userId, day) {
  return `share:day:${userId}:${day}`;
}

export function shareActiveKey(userId) {
  return `share:active:${userId}`;
}

export function shareMpuKey(uploadId) {
  return `share:mpu:${uploadId}`;
}

export function utcDayFromSec(nowSec) {
  return new Date(Number(nowSec) * 1000).toISOString().slice(0, 10);
}

export function publicShareUrl(base, id) {
  return `${String(base || "").replace(/\/+$/, "")}/s/${id}`;
}

export function randomShareToken() {
  const bytes = new Uint8Array(24);
  crypto.getRandomValues(bytes);
  return btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=/g, "");
}

export function randomShortShareCode() {
  const bytes = new Uint8Array(8);
  crypto.getRandomValues(bytes);
  let out = "";
  for (const b of bytes) out += SHORT_ALPHABET[b % SHORT_ALPHABET.length];
  return out;
}

export function shareContentDisposition(isExe) {
  const utf8Name = isExe ? SHARE_SFX_DOWNLOAD_NAME : SHARE_ZIP_DOWNLOAD_NAME;
  const asciiName = isExe ? "modpack-zh-tw-sfx.exe" : "modpack-zh-tw.zip";
  return `attachment; filename="${asciiName}"; filename*=UTF-8''${encodeURIComponent(utf8Name)}`;
}

export function sharePackNameFromHeader(request) {
  const rawName = String(request?.headers?.get?.("x-zeitfrei-pack-name") || "");
  let packName = "Minecraft 模組翻譯資源包";
  try {
    const decoded = decodeURIComponent(rawName);
    const cleaned = decoded.replace(/[\\/\u0000-\u001f\u007f]/g, "").trim().slice(0, 120);
    if (cleaned) packName = cleaned;
  } catch (_) {}
  return packName;
}

export function shareIsExe(kind, type) {
  const k = String(kind || "").toLowerCase();
  const t = String(type || "").toLowerCase();
  return k.includes("sfx") || t.includes("executable") || t.includes("msdownload") || t === "application/octet-stream";
}

export function parseJson(raw, fallback) {
  if (raw == null || raw === "") return fallback;
  try {
    return JSON.parse(raw);
  } catch (_) {
    return fallback;
  }
}

export function pruneActive(list, nowSec) {
  if (!Array.isArray(list)) return [];
  return list.filter((item) => Number(item?.expiresAt || 0) > nowSec);
}

export function quotaDecision({ dailyCount, activeCount, dailyLimit, activeLimit }) {
  if (Number(dailyCount) >= Number(dailyLimit)) {
    return { ok: false, status: 429, error: "share daily limit reached" };
  }
  if (Number(activeCount) >= Number(activeLimit)) {
    return { ok: false, status: 429, error: "share active limit reached" };
  }
  return { ok: true };
}

export async function allocateShortCode(env) {
  if (!env?.USAGE) return "";
  for (let i = 0; i < 8; i++) {
    const code = randomShortShareCode();
    const key = shareIdKey(code);
    const existing = await env.USAGE.get(key);
    if (!existing) {
      await env.USAGE.put(key, JSON.stringify({ reserved: true }), {
        expirationTtl: SHARE_TTL_SECONDS,
      });
      return code;
    }
  }
  return "";
}

export async function dropShortCode(env, shortCode) {
  if (!env?.USAGE || !shortCode) return;
  try {
    await env.USAGE.delete(shareIdKey(shortCode));
  } catch (_) {}
}

export async function putShortCode(env, shortCode, payload, expiresAt, nowSec) {
  if (!env?.USAGE || !shortCode) return;
  const ttl = Math.max(60, Math.min(SHARE_TTL_SECONDS, Number(expiresAt) - nowSec));
  await env.USAGE.put(shareIdKey(shortCode), JSON.stringify(payload), { expirationTtl: ttl });
}

export async function reserveShareSlot(env, userId, nowSec, pendingEntry) {
  if (!env?.USAGE) return { ok: true, skipped: true };
  const decided = quotaDecision({
    dailyCount: parseInt((await env.USAGE.get(shareDayKey(userId, utcDayFromSec(nowSec)))) || "0", 10) || 0,
    activeCount: pruneActive(parseJson(await env.USAGE.get(shareActiveKey(userId)), []), nowSec).length,
    dailyLimit: shareDailyLimit(env),
    activeLimit: shareActiveLimit(env),
  });
  if (!decided.ok) return decided;
  if (pendingEntry) {
    const activeKey = shareActiveKey(userId);
    const active = pruneActive(parseJson(await env.USAGE.get(activeKey), []), nowSec);
    active.push(pendingEntry);
    await env.USAGE.put(activeKey, JSON.stringify(active), {
      expirationTtl: SHARE_TTL_SECONDS + 3600,
    });
  }
  return { ok: true };
}

export async function finalizeShareQuota(env, userId, nowSec, entry) {
  if (!env?.USAGE) return;
  const dayKey = shareDayKey(userId, utcDayFromSec(nowSec));
  const daily = parseInt((await env.USAGE.get(dayKey)) || "0", 10) || 0;
  await env.USAGE.put(dayKey, String(daily + 1), { expirationTtl: 172800 });
  const activeKey = shareActiveKey(userId);
  const active = pruneActive(parseJson(await env.USAGE.get(activeKey), []), nowSec);
  const idx = active.findIndex(
    (item) =>
      (entry.uploadId && item.uploadId === entry.uploadId) ||
      (entry.key && item.key === entry.key)
  );
  const rec = {
    key: entry.key,
    shortCode: entry.shortCode || "",
    expiresAt: entry.expiresAt,
    kind: "ready",
    createdAt: nowSec,
    uploadId: entry.uploadId || "",
  };
  if (idx >= 0) active[idx] = { ...active[idx], ...rec };
  else active.push(rec);
  await env.USAGE.put(activeKey, JSON.stringify(active), {
    expirationTtl: SHARE_TTL_SECONDS + 3600,
  });
  if (entry.shortCode) {
    await putShortCode(
      env,
      entry.shortCode,
      { key: entry.key, expiresAt: entry.expiresAt },
      entry.expiresAt,
      nowSec
    );
  }
  if (entry.uploadId) {
    try {
      await env.USAGE.delete(shareMpuKey(entry.uploadId));
    } catch (_) {}
  }
}

export async function releasePendingShare(env, userId, nowSec, pending) {
  if (!env?.USAGE || !userId) return;
  const activeKey = shareActiveKey(userId);
  const active = pruneActive(parseJson(await env.USAGE.get(activeKey), []), nowSec).filter((item) => {
    if (pending?.uploadId && item.uploadId === pending.uploadId) return false;
    if (pending?.key && item.key === pending.key && item.kind === "pending") return false;
    return true;
  });
  await env.USAGE.put(activeKey, JSON.stringify(active), {
    expirationTtl: SHARE_TTL_SECONDS + 3600,
  });
}

export async function resolveShareObject(env, token, nowSec) {
  if (!env?.SHARES) return { error: "share storage not configured", status: 503 };
  const id = String(token || "");
  if (isShortShareCode(id)) {
    if (!env.USAGE) return { error: "share not found or expired", status: 404 };
    const raw = await env.USAGE.get(shareIdKey(id));
    if (!raw) return { error: "share not found or expired", status: 404 };
    const rec = parseJson(raw, null);
    const key = rec?.key;
    if (!key) return { error: "share not found or expired", status: 404 };
    const object = await env.SHARES.get(key);
    if (!object) {
      try {
        await env.USAGE.delete(shareIdKey(id));
      } catch (_) {}
      return { error: "share not found or expired", status: 404 };
    }
    const expiresAt = Number(object.customMetadata?.expiresAt || rec.expiresAt || 0);
    if (!expiresAt || expiresAt <= nowSec) {
      try {
        await env.SHARES.delete(key);
      } catch (_) {}
      try {
        await env.USAGE.delete(shareIdKey(id));
      } catch (_) {}
      return { error: "share expired", status: 404 };
    }
    return { object, key, expiresAt, publicId: id };
  }
  if (!isLongShareToken(id)) return { error: "bad share token", status: 400 };
  let object = await env.SHARES.get(SHARE_PREFIX + id + ".exe");
  let key = SHARE_PREFIX + id + ".exe";
  if (!object) {
    object = await env.SHARES.get(SHARE_PREFIX + id + ".zip");
    key = SHARE_PREFIX + id + ".zip";
  }
  if (!object) return { error: "share not found or expired", status: 404 };
  const expiresAt = Number(object.customMetadata?.expiresAt || 0);
  if (!expiresAt || expiresAt <= nowSec) {
    try {
      await env.SHARES.delete(key);
    } catch (_) {}
    const shortCode = object.customMetadata?.shortCode;
    if (shortCode && env.USAGE) {
      try {
        await env.USAGE.delete(shareIdKey(shortCode));
      } catch (_) {}
    }
    return { error: "share expired", status: 404 };
  }
  return { object, key, expiresAt, publicId: id };
}

function shareCorsHeaders() {
  return {
    "access-control-allow-origin": "*",
    "access-control-allow-methods": "GET, POST, OPTIONS",
    "access-control-allow-headers":
      "content-type, authorization, x-zeitfrei-ai-protocol, x-zeitfrei-client-version, x-zeitfrei-session, x-zeitfrei-turnstile",
  };
}

function shareJson(obj, status = 200) {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { ...JSON_HEADERS, "cache-control": "no-store", ...shareCorsHeaders() },
  });
}

function publicBase(env, request) {
  return String(env?.SHARE_PUBLIC_URL || new URL(request.url).origin).replace(/\/+$/, "");
}

function sanitizePackName(value, fallback = "Minecraft 模組翻譯資源包") {
  const cleaned = String(value || "")
    .replace(/[\\/\u0000-\u001f\u007f]/g, "")
    .trim()
    .slice(0, 120);
  return cleaned || fallback;
}

function shareCustomMetadata({ expiresAt, userId, packName, isExe, shortCode }) {
  return {
    expiresAt: String(expiresAt),
    uploader: userId,
    service: "packlocal-share",
    name: packName,
    kind: isExe ? "sfx-exe" : "zip",
    password: isExe ? "cloud.zeitfrei.uk" : "",
    shortCode: shortCode || "",
  };
}

export async function shareUpload(request, env, userId, nowSec = Math.floor(Date.now() / 1000)) {
  const type = String(request.headers.get("content-type") || "").split(";")[0].toLowerCase();
  const maxBytes = shareMaxBytes(env);
  const declared = parseInt(request.headers.get("content-length") || "0", 10);
  const allowedTypes = new Set([
    "application/zip",
    "application/octet-stream",
    "application/x-msdownload",
    "application/vnd.microsoft.portable-executable",
  ]);
  if (!allowedTypes.has(type)) return shareJson({ error: "exe or zip content type required" }, 415);
  if (!Number.isFinite(declared) || declared <= 0) return shareJson({ error: "content length required" }, 411);
  if (declared > maxBytes) return shareJson({ error: "share file too large" }, 413);

  const token = randomShareToken();
  const expiresAt = nowSec + SHARE_TTL_SECONDS;
  const kind = String(request.headers.get("x-zeitfrei-share-kind") || "").toLowerCase();
  const isExe = shareIsExe(kind, type);
  const ext = isExe ? ".exe" : ".zip";
  const key = SHARE_PREFIX + token + ext;
  const packName = sharePackNameFromHeader(request);
  const shortCode = await allocateShortCode(env);
  const pending = { key, kind: "pending", createdAt: nowSec, expiresAt, shortCode };
  const reserved = await reserveShareSlot(env, userId, nowSec, pending);
  if (!reserved.ok) {
    await dropShortCode(env, shortCode);
    return shareJson({ error: reserved.error }, reserved.status);
  }

  const object = await env.SHARES.put(key, request.body, {
    httpMetadata: {
      contentType: isExe ? "application/vnd.microsoft.portable-executable" : "application/zip",
      cacheControl: "no-store",
    },
    customMetadata: shareCustomMetadata({ expiresAt, userId, packName, isExe, shortCode }),
  });
  if (!object) {
    await releasePendingShare(env, userId, nowSec, pending);
    await dropShortCode(env, shortCode);
    return shareJson({ error: "share upload failed" }, 500);
  }
  await finalizeShareQuota(env, userId, nowSec, { key, shortCode, expiresAt });
  const publicId = shortCode || token;
  return shareJson({ url: publicShareUrl(publicBase(env, request), publicId), expiresAt });
}

export async function shareMpuCreate(request, env, userId, nowSec = Math.floor(Date.now() / 1000)) {
  let body = {};
  try {
    body = await request.json();
  } catch (_) {
    return shareJson({ error: "json body required" }, 400);
  }
  const size = Number(body.size || 0);
  const maxBytes = shareMaxBytes(env);
  if (!Number.isFinite(size) || size <= 0) return shareJson({ error: "size required" }, 400);
  if (size > maxBytes) return shareJson({ error: "share file too large" }, 413);

  const kind = String(body.kind || request.headers.get("x-zeitfrei-share-kind") || "").toLowerCase();
  const isExe = shareIsExe(kind, body.contentType || "");
  const token = randomShareToken();
  const expiresAt = nowSec + SHARE_TTL_SECONDS;
  const ext = isExe ? ".exe" : ".zip";
  const key = SHARE_PREFIX + token + ext;
  const packName =
    typeof body.name === "string" && body.name.trim()
      ? sanitizePackName(body.name)
      : sharePackNameFromHeader(request);
  const shortCode = await allocateShortCode(env);
  const pending = { key, kind: "pending", createdAt: nowSec, expiresAt, shortCode };
  const reserved = await reserveShareSlot(env, userId, nowSec, pending);
  if (!reserved.ok) {
    await dropShortCode(env, shortCode);
    return shareJson({ error: reserved.error }, reserved.status);
  }

  let multipart;
  try {
    multipart = await env.SHARES.createMultipartUpload(key, {
      httpMetadata: {
        contentType: isExe ? "application/vnd.microsoft.portable-executable" : "application/zip",
        cacheControl: "no-store",
      },
      customMetadata: shareCustomMetadata({ expiresAt, userId, packName, isExe, shortCode }),
    });
  } catch (err) {
    await releasePendingShare(env, userId, nowSec, pending);
    await dropShortCode(env, shortCode);
    return shareJson(
      { error: "multipart create failed", detail: String(err && err.message ? err.message : err).slice(0, 200) },
      400
    );
  }
  pending.uploadId = multipart.uploadId;
  if (env.USAGE) {
    const activeKey = shareActiveKey(userId);
    const active = pruneActive(parseJson(await env.USAGE.get(activeKey), []), nowSec);
    const idx = active.findIndex((item) => item.key === key && item.kind === "pending");
    if (idx >= 0) active[idx] = { ...active[idx], uploadId: multipart.uploadId };
    await env.USAGE.put(activeKey, JSON.stringify(active), {
      expirationTtl: SHARE_TTL_SECONDS + 3600,
    });
    await env.USAGE.put(
      shareMpuKey(multipart.uploadId),
      JSON.stringify({
        key,
        uploadId: multipart.uploadId,
        userId,
        createdAt: nowSec,
        shortCode,
        token,
        expiresAt,
      }),
      { expirationTtl: Math.max(shareMpuStaleSeconds(env) * 2, 7200) }
    );
  }
  return shareJson({
    token,
    key: multipart.key,
    uploadId: multipart.uploadId,
    expiresAt,
    partSize: 20 * 1024 * 1024,
  });
}

export async function shareMpuPart(request, env, url) {
  const key = String(url.searchParams.get("key") || "");
  const uploadId = String(url.searchParams.get("uploadId") || "");
  const partNumber = parseInt(url.searchParams.get("partNumber") || "0", 10);
  if (!key.startsWith(SHARE_PREFIX) || !uploadId || !Number.isFinite(partNumber) || partNumber < 1 || partNumber > 10000) {
    return shareJson({ error: "bad multipart part params" }, 400);
  }
  const declared = parseInt(request.headers.get("content-length") || "0", 10);
  if (!Number.isFinite(declared) || declared <= 0) return shareJson({ error: "content length required" }, 411);
  if (declared > 90 * 1024 * 1024) return shareJson({ error: "part too large" }, 413);

  try {
    const upload = env.SHARES.resumeMultipartUpload(key, uploadId);
    const uploaded = await upload.uploadPart(partNumber, request.body);
    return shareJson({ partNumber: uploaded.partNumber, etag: uploaded.etag });
  } catch (err) {
    return shareJson(
      { error: "multipart part failed", detail: String(err && err.message ? err.message : err).slice(0, 200) },
      400
    );
  }
}

export async function shareMpuComplete(request, env, userId, nowSec = Math.floor(Date.now() / 1000)) {
  let body = {};
  try {
    body = await request.json();
  } catch (_) {
    return shareJson({ error: "json body required" }, 400);
  }
  const key = String(body.key || "");
  const uploadId = String(body.uploadId || "");
  const token = String(body.token || "");
  const parts = Array.isArray(body.parts) ? body.parts : [];
  if (!key.startsWith(SHARE_PREFIX) || !uploadId || !isLongShareToken(token)) {
    return shareJson({ error: "bad multipart complete params" }, 400);
  }
  if (!key.includes(token)) return shareJson({ error: "token/key mismatch" }, 400);
  if (parts.length < 1) return shareJson({ error: "parts required" }, 400);
  const normalized = [];
  for (const p of parts) {
    const partNumber = Number(p.partNumber || p.PartNumber || 0);
    const etag = String(p.etag || p.ETag || "");
    if (!Number.isFinite(partNumber) || partNumber < 1 || !etag) {
      return shareJson({ error: "invalid part etag" }, 400);
    }
    normalized.push({ partNumber, etag });
  }
  normalized.sort((a, b) => a.partNumber - b.partNumber);
  try {
    const upload = env.SHARES.resumeMultipartUpload(key, uploadId);
    const object = await upload.complete(normalized);
    if (!object) return shareJson({ error: "share upload failed" }, 500);
    const expiresAt =
      Number(object.customMetadata?.expiresAt || 0) || nowSec + SHARE_TTL_SECONDS;
    let shortCode = String(object.customMetadata?.shortCode || "");
    if (env.USAGE) {
      const tracked = parseJson(await env.USAGE.get(shareMpuKey(uploadId)), null);
      if (tracked?.shortCode) shortCode = tracked.shortCode;
    }
    if (env.USAGE && !shortCode) shortCode = await allocateShortCode(env);
    await finalizeShareQuota(env, userId, nowSec, { key, shortCode, expiresAt, uploadId });
    const publicId = shortCode || token;
    return shareJson({ url: publicShareUrl(publicBase(env, request), publicId), expiresAt });
  } catch (err) {
    try {
      const upload = env.SHARES.resumeMultipartUpload(key, uploadId);
      await upload.abort();
    } catch (_) {}
    await releasePendingShare(env, userId, nowSec, { uploadId, key });
    await dropShortCode(env, parseJson(await env.USAGE?.get?.(shareMpuKey(uploadId)), null)?.shortCode);
    return shareJson(
      { error: "multipart complete failed", detail: String(err && err.message ? err.message : err).slice(0, 200) },
      400
    );
  }
}

export async function shareDownload(url, env, headOnly, nowSec = Math.floor(Date.now() / 1000)) {
  const token = decodeURIComponent(url.pathname.slice("/s/".length).replace(/\/download$/, ""));
  const found = await resolveShareObject(env, token, nowSec);
  if (found.error) return shareJson({ error: found.error }, found.status);
  const downloadRequested = headOnly || url.pathname.endsWith("/download") || url.searchParams.get("download") === "1";
  if (!downloadRequested) {
    return renderShareLanding(url, env, found.publicId, found.object, found.expiresAt);
  }
  const headers = new Headers();
  if (typeof found.object.writeHttpMetadata === "function") {
    found.object.writeHttpMetadata(headers);
  }
  const isExe =
    String(found.key || "").endsWith(".exe") || String(found.object.customMetadata?.kind || "").includes("sfx");
  headers.set(
    "content-type",
    isExe ? "application/vnd.microsoft.portable-executable" : "application/zip"
  );
  if (found.object.size != null) headers.set("content-length", String(found.object.size));
  headers.set("cache-control", "no-store, max-age=0");
  headers.set("content-disposition", shareContentDisposition(isExe));
  return new Response(headOnly ? null : found.object.body, { headers });
}

export function shareOgImage(headOnly) {
  const svg = `<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630">
  <rect width="1200" height="630" fill="#14161a"/>
  <rect x="64" y="64" width="1072" height="502" rx="28" fill="#191c20" stroke="#3d4147"/>
  <circle cx="180" cy="240" r="56" fill="#d88958"/>
  <text x="280" y="230" fill="#e8e5df" font-family="Segoe UI,sans-serif" font-size="48" font-weight="700">${SHARE_OG_TITLE}</text>
  <text x="280" y="300" fill="#b8b8b2" font-family="Segoe UI,sans-serif" font-size="32">${SHARE_OG_DESCRIPTION}</text>
  <text x="120" y="480" fill="#8e918d" font-family="Segoe UI,sans-serif" font-size="26">Discord discord.gg/zeitfrei · 支持開發 zeitfrei.bobaboba.me</text>
</svg>`;
  return new Response(headOnly ? null : svg, {
    headers: {
      "content-type": "image/svg+xml; charset=utf-8",
      "cache-control": "public, max-age=86400",
    },
  });
}

function escapeHtml(value) {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function renderShareLanding(url, env, token, object, expiresAt) {
  const name = String(object.customMetadata?.name || "Minecraft 模組翻譯資源包");
  const base = String(env.SHARE_PUBLIC_URL || url.origin).replace(/\/+$/, "");
  const canonical = base + "/s/" + token;
  const downloadUrl = canonical + "?download=1";
  const imageUrl = base + "/share-og.png";
  const expires = new Date(expiresAt * 1000).toLocaleString("zh-TW", { dateStyle: "medium", timeStyle: "short" });
  const isExe =
    String(object.customMetadata?.kind || "").includes("sfx") ||
    String(object.httpMetadata?.contentType || "").includes("executable");
  const password = String(object.customMetadata?.password || (isExe ? "cloud.zeitfrei.uk" : ""));
  const bodyCopy = isExe
    ? "<p>這是帶密碼的自解 exe。下載後執行，輸入下方密碼解壓，並<strong>選擇 Minecraft 遊戲資料夾</strong>，翻譯會自動套用（對齊工具套用流程）。套用後請重開遊戲，語言選繁體中文（台灣）並啟用資源包。</p>" +
      "<p>解壓密碼：<strong>" +
      escapeHtml(password) +
      "</strong></p>"
    : "<p>這是可直接覆蓋到 Minecraft 實例的翻譯資源包。連結只保留 24 小時，下載後請解壓到對應的遊戲資料夾。</p>";
  const html = [
    "<!doctype html><html lang=\"zh-Hant\"><head><meta charset=\"utf-8\">",
    "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    "<meta name=\"robots\" content=\"noindex,nofollow\">",
    "<meta property=\"og:type\" content=\"website\">",
    "<meta property=\"og:title\" content=\"" + escapeHtml(SHARE_OG_TITLE) + "\">",
    "<meta property=\"og:description\" content=\"" + escapeHtml(SHARE_OG_DESCRIPTION) + "\">",
    "<meta property=\"og:url\" content=\"" + escapeHtml(canonical) + "\">",
    "<meta property=\"og:image\" content=\"" + escapeHtml(imageUrl) + "\">",
    "<meta name=\"twitter:card\" content=\"summary_large_image\">",
    "<meta name=\"twitter:title\" content=\"" + escapeHtml(SHARE_OG_TITLE) + "\">",
    "<meta name=\"twitter:description\" content=\"" + escapeHtml(SHARE_OG_DESCRIPTION) + "\">",
    "<meta name=\"twitter:image\" content=\"" + escapeHtml(imageUrl) + "\">",
    "<meta name=\"theme-color\" content=\"#14161a\">",
    "<title>" + escapeHtml(SHARE_OG_TITLE) + "</title>",
    "<style>body{margin:0;background:#101214;color:#e8e5df;font:16px/1.6 system-ui,sans-serif}main{max-width:680px;margin:8vh auto;padding:28px}article{border:1px solid #3d4147;border-radius:14px;background:#191c20;padding:28px;box-shadow:0 20px 70px #0006}p{color:#b8b8b2}.tag{color:#e29a62;font-size:12px;letter-spacing:.14em;text-transform:uppercase}h1{margin:8px 0 6px;font-size:28px}.lead{color:#d8d4cc;font-size:18px;margin:0 0 16px}.pack{color:#8e918d;font-size:14px}a.button{display:inline-block;background:#dd8951;color:#17110d;text-decoration:none;font-weight:700;padding:11px 18px;border-radius:8px;margin:12px 0 8px}a.secondary{display:inline-block;margin:6px 14px 0 0;color:#d8a47d;text-decoration:none;font-size:14px}footer{margin-top:28px;font-size:13px;color:#8e918d;border-top:1px solid #2c3036;padding-top:16px}footer a{color:#d8a47d;margin-right:14px}code{background:#101214;padding:2px 6px;border-radius:4px}</style>",
    "</head><body><main><article><div class=\"tag\">MODPACK TRANSLATION SHARE</div>",
    "<h1>" + escapeHtml(SHARE_OG_TITLE) + "</h1>",
    "<p class=\"lead\">" + escapeHtml(SHARE_OG_DESCRIPTION) + "</p>",
    "<p class=\"pack\">包名：" + escapeHtml(name) + "</p>",
    bodyCopy,
    "<p>有效期限：<strong>" + escapeHtml(expires) + "</strong></p>",
    "<a class=\"button\" href=\"" + escapeHtml(downloadUrl) + "\">" + (isExe ? "下載自解翻譯檔" : "下載翻譯檔") + "</a>",
    "<div><a class=\"secondary\" href=\"https://discord.gg/zeitfrei\" target=\"_blank\" rel=\"noopener\">加入 Discord 官方伺服器</a>",
    "<a class=\"secondary\" href=\"https://zeitfrei.bobaboba.me\" target=\"_blank\" rel=\"noopener\">支持開發 · 讓免費 AI 持續運作</a>",
    "<a class=\"secondary\" href=\"https://cloud.zeitfrei.uk/\" target=\"_blank\" rel=\"noopener\">ZeitFrei 雲端</a></div>",
    "<footer>模組包翻譯工具 · 24 小時分享連結</footer>",
    "</article></main></body></html>",
  ].join("");
  return new Response(html, {
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store, max-age=0",
      "content-security-policy": "default-src 'none'; style-src 'unsafe-inline'; img-src https: data:; frame-ancestors *; base-uri 'none'",
      "x-content-type-options": "nosniff",
      "referrer-policy": "no-referrer",
    },
  });
}

export async function cleanupShares(env, nowSec = Math.floor(Date.now() / 1000)) {
  if (env?.SHARES) {
    const listing = await env.SHARES.list({ prefix: SHARE_PREFIX, limit: 1000 });
    const expired = await Promise.all(
      (listing.objects || []).map(async (object) => {
        const head = await env.SHARES.head(object.key);
        if (!head || Number(head.customMetadata?.expiresAt || 0) > nowSec) return null;
        return { key: object.key, shortCode: head.customMetadata?.shortCode || "" };
      })
    );
    await Promise.all(
      expired.filter(Boolean).map(async (item) => {
        try {
          await env.SHARES.delete(item.key);
        } catch (_) {}
        if (item.shortCode && env.USAGE) {
          try {
            await env.USAGE.delete(shareIdKey(item.shortCode));
          } catch (_) {}
        }
      })
    );
  }
  if (!env?.USAGE || !env?.SHARES) return;
  const stale = shareMpuStaleSeconds(env);
  let cursor;
  do {
    const page = await env.USAGE.list({ prefix: "share:mpu:", limit: 1000, cursor });
    for (const entry of page.keys || []) {
      const rec = parseJson(await env.USAGE.get(entry.name), null);
      if (!rec) continue;
      if (nowSec - Number(rec.createdAt || 0) < stale) continue;
      try {
        const upload = env.SHARES.resumeMultipartUpload(rec.key, rec.uploadId);
        await upload.abort();
      } catch (_) {}
      try {
        await env.USAGE.delete(entry.name);
      } catch (_) {}
      await releasePendingShare(env, rec.userId, nowSec, rec);
    }
    cursor = page.list_complete ? null : page.cursor;
  } while (cursor);
}
