// 診斷回報：SHARES 前綴 reports/v1/、Discord 會籍、100MB、MPU、webhook 只送連結、3 天清理。
// DISCORD_REPORT_WEBHOOK 只從 env secret 讀取，永不寫入回應或日誌。

import { parseJson } from "./share.mjs";
import { isSafeOutboundUrl } from "./security.mjs";

const JSON_HEADERS = { "content-type": "application/json; charset=utf-8" };

export const REPORT_PREFIX = "reports/v1/";
export const REPORT_TTL_SECONDS = 3 * 24 * 60 * 60;
export const REPORT_MAX_BYTES = 100 * 1024 * 1024;
export const REPORT_DAILY_LIMIT = 3;
export const REPORT_ACTIVE_LIMIT = 5;
export const REPORT_BURST_SECONDS = 60;
export const REPORT_MPU_STALE_SECONDS = 60 * 60;
export const REPORT_MPU_MIN_PART = 5 * 1024 * 1024;
export const REPORT_NOTE_MAX = 2048;

export const REPORT_CATEGORIES = new Set([
  "crash_after_apply",
  "crash_on_world",
  "crash_on_book_quest",
  "ui_mojibake",
  "ui_tofu",
  "still_english",
  "bad_translation",
  "shared_lib_suspect",
  "placeholder_broken",
  "pack_unsupported",
  "pack_partial_support",
  "loader_version",
  "missing_source",
  "tool_crash",
  "tool_one_click_fail",
  "tool_apply_fail",
  "tool_restore_fail",
  "tool_update_fail",
  "tool_share_fail",
  "tool_ai_managed",
  "tool_ai_custom",
  "tool_ui",
  "other_feature",
  "other_docs",
  "other_privacy",
  "other",
]);

export const REPORT_ALLOWED_NAMES = new Set([
  "manifest.json",
  "user_note.txt",
  "diagnosis.json",
  "crash.txt",
  "latest.log",
  "翻譯工作階段.json",
  "覆蓋範圍說明.txt",
  "翻譯錯誤日誌.txt",
]);

export function isReportCategory(value) {
  return REPORT_CATEGORIES.has(String(value || ""));
}

export function isAllowedReportName(name) {
  const value = String(name || "");
  if (!value || value.includes("/") || value.includes("\\") || value.includes("..")) return false;
  if (REPORT_ALLOWED_NAMES.has(value)) return true;
  return /^crash[-_].+\.txt$/i.test(value);
}

export function reportIdKey(token) {
  return `report:id:${token}`;
}

export function reportDayKey(userId, day) {
  return `report:day:${userId}:${day}`;
}

export function reportActiveKey(userId) {
  return `report:active:${userId}`;
}

export function reportMpuKey(uploadId) {
  return `report:mpu:${uploadId}`;
}

export function reportLastKey(userId) {
  return `report:last:${userId}`;
}

export function utcDayFromSec(nowSec) {
  return new Date(Number(nowSec) * 1000).toISOString().slice(0, 10);
}

export function randomReportToken() {
  const bytes = new Uint8Array(24);
  crypto.getRandomValues(bytes);
  return btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=/g, "");
}

export function isReportToken(token) {
  return /^[A-Za-z0-9_-]{32,48}$/.test(String(token || ""));
}

export function reportObjectKey(token) {
  return `${REPORT_PREFIX}${token}.zip`;
}

export function reportQuotaDecision({ dailyCount, activeCount, lastAt, nowSec }) {
  if (Number(dailyCount) >= REPORT_DAILY_LIMIT) {
    return { ok: false, status: 429, error: "report daily limit reached" };
  }
  if (Number(activeCount) >= REPORT_ACTIVE_LIMIT) {
    return { ok: false, status: 429, error: "report active limit reached" };
  }
  if (lastAt && Number(nowSec) - Number(lastAt) < REPORT_BURST_SECONDS) {
    return { ok: false, status: 429, error: "report rate limited" };
  }
  return { ok: true };
}

export function pruneActive(list, nowSec) {
  if (!Array.isArray(list)) return [];
  return list.filter((item) => Number(item?.expiresAt || 0) > nowSec);
}

export function looksLikeZipMagic(bytes) {
  if (!bytes || bytes.length < 4) return false;
  return bytes[0] === 0x50 && bytes[1] === 0x4b && (bytes[2] === 0x03 || bytes[2] === 0x05 || bytes[2] === 0x07);
}

export function sanitizePackLabel(name, unrelated) {
  if (unrelated) return "與包無關";
  const cleaned = String(name || "")
    .replace(/[\\/\u0000-\u001f\u007f]/g, "")
    .trim()
    .slice(0, 120);
  return cleaned || "未命名整合包";
}

export function buildWebhookText({ category, packLabel, packVersion, errorCode, toolVersion, url }) {
  const ver = String(packVersion || "-").slice(0, 40);
  const code = String(errorCode || "-").slice(0, 64);
  const tool = String(toolVersion || "-").slice(0, 24);
  return `[${category}] ${packLabel} ${ver} | ${code} | 工具 ${tool} | ${url}`;
}

function reportJson(obj, status = 200) {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { ...JSON_HEADERS, "cache-control": "no-store" },
  });
}

function publicBase(env, request) {
  return String(env?.SHARE_PUBLIC_URL || new URL(request.url).origin).replace(/\/+$/, "");
}

export function publicReportUrl(base, token) {
  return `${String(base || "").replace(/\/+$/, "")}/report/${token}`;
}

async function reserveReportSlot(env, userId, nowSec, pending) {
  if (!env?.USAGE) return { ok: false, status: 503, error: "usage not configured" };
  const decided = reportQuotaDecision({
    dailyCount: parseInt((await env.USAGE.get(reportDayKey(userId, utcDayFromSec(nowSec)))) || "0", 10) || 0,
    activeCount: pruneActive(parseJson(await env.USAGE.get(reportActiveKey(userId)), []), nowSec).length,
    lastAt: parseInt((await env.USAGE.get(reportLastKey(userId))) || "0", 10) || 0,
    nowSec,
  });
  if (!decided.ok) return decided;
  const activeKey = reportActiveKey(userId);
  const active = pruneActive(parseJson(await env.USAGE.get(activeKey), []), nowSec);
  active.push(pending);
  await env.USAGE.put(activeKey, JSON.stringify(active), { expirationTtl: REPORT_TTL_SECONDS + 3600 });
  return { ok: true };
}

async function finalizeReportQuota(env, userId, nowSec, entry) {
  if (!env?.USAGE) return;
  const dayKey = reportDayKey(userId, utcDayFromSec(nowSec));
  const daily = parseInt((await env.USAGE.get(dayKey)) || "0", 10) || 0;
  await env.USAGE.put(dayKey, String(daily + 1), { expirationTtl: 172800 });
  await env.USAGE.put(reportLastKey(userId), String(nowSec), { expirationTtl: 3600 });
  const activeKey = reportActiveKey(userId);
  const active = pruneActive(parseJson(await env.USAGE.get(activeKey), []), nowSec);
  const idx = active.findIndex(
    (item) =>
      (entry.uploadId && item.uploadId === entry.uploadId) ||
      (entry.token && item.token === entry.token)
  );
  if (idx >= 0) active[idx] = { ...active[idx], ...entry, kind: "ready" };
  else active.push({ ...entry, kind: "ready" });
  await env.USAGE.put(activeKey, JSON.stringify(active), { expirationTtl: REPORT_TTL_SECONDS + 3600 });
  if (entry.token) {
    await env.USAGE.put(
      reportIdKey(entry.token),
      JSON.stringify({
        token: entry.token,
        key: entry.key,
        userId,
        expiresAt: entry.expiresAt,
        category: entry.category || "",
      }),
      { expirationTtl: REPORT_TTL_SECONDS }
    );
  }
}

async function releasePendingReport(env, userId, nowSec, pending) {
  if (!env?.USAGE) return;
  const activeKey = reportActiveKey(userId);
  const active = pruneActive(parseJson(await env.USAGE.get(activeKey), []), nowSec).filter(
    (item) =>
      !(
        (pending.uploadId && item.uploadId === pending.uploadId) ||
        (pending.token && item.token === pending.token)
      )
  );
  await env.USAGE.put(activeKey, JSON.stringify(active), { expirationTtl: REPORT_TTL_SECONDS + 3600 });
}

async function notifyDiscord(env, text) {
  const hook = env?.DISCORD_REPORT_WEBHOOK && String(env.DISCORD_REPORT_WEBHOOK).trim();
  if (!hook) return { ok: false, error: "report notify not configured" };
  if (!isSafeOutboundUrl(hook)) return { ok: false, error: "report webhook blocked" };
  let resp;
  try {
    resp = await fetch(hook, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ content: String(text || "").slice(0, 1800) }),
    });
  } catch (_) {
    return { ok: false, error: "report notify failed" };
  }
  if (!resp.ok) return { ok: false, error: "report notify failed" };
  return { ok: true };
}

function parseReportMeta(body, headers) {
  const category = String(body?.reportCategory || headers?.get?.("x-zeitfrei-report-category") || "");
  const packUnrelated = body?.packUnrelated === true || headers?.get?.("x-zeitfrei-pack-unrelated") === "1";
  const packName = String(body?.packName || headers?.get?.("x-zeitfrei-pack-name") || "");
  const packVersion = String(body?.packVersion || "").slice(0, 40);
  const errorCode = String(body?.errorCode || "").slice(0, 64);
  const toolVersion = String(body?.toolVersion || headers?.get?.("x-zeitfrei-client-version") || "").slice(0, 24);
  const size = Number(body?.size || 0);
  return { category, packUnrelated, packName, packVersion, errorCode, toolVersion, size };
}

export async function reportMpuCreate(request, env, userId, nowSec = Math.floor(Date.now() / 1000)) {
  if (!env.SHARES) return reportJson({ error: "report storage not configured" }, 503);
  if (!env.USAGE) return reportJson({ error: "usage not configured" }, 503);
  if (!env.DISCORD_REPORT_WEBHOOK) return reportJson({ error: "report notify not configured" }, 503);
  let body = {};
  try {
    body = await request.json();
  } catch (_) {
    body = {};
  }
  const meta = parseReportMeta(body, request.headers);
  if (!isReportCategory(meta.category)) return reportJson({ error: "reportCategory required" }, 400);
  if (!meta.packUnrelated && !String(meta.packName).trim()) {
    return reportJson({ error: "pack name required" }, 400);
  }
  if (!Number.isFinite(meta.size) || meta.size <= 0) return reportJson({ error: "content length required" }, 411);
  if (meta.size > REPORT_MAX_BYTES) return reportJson({ error: "report file too large" }, 413);

  const token = randomReportToken();
  const key = reportObjectKey(token);
  const expiresAt = nowSec + REPORT_TTL_SECONDS;
  const pending = {
    key,
    token,
    kind: "pending",
    createdAt: nowSec,
    expiresAt,
    category: meta.category,
    packName: meta.packName,
    packUnrelated: meta.packUnrelated,
    packVersion: meta.packVersion,
    errorCode: meta.errorCode,
    toolVersion: meta.toolVersion,
  };
  const reserved = await reserveReportSlot(env, userId, nowSec, pending);
  if (!reserved.ok) return reportJson({ error: reserved.error }, reserved.status);

  let multipart;
  try {
    multipart = await env.SHARES.createMultipartUpload(key, {
      httpMetadata: { contentType: "application/zip", cacheControl: "no-store" },
      customMetadata: {
        expiresAt: String(expiresAt),
        service: "packlocal-report",
        category: meta.category,
        packUnrelated: meta.packUnrelated ? "1" : "0",
      },
    });
  } catch (err) {
    await releasePendingReport(env, userId, nowSec, pending);
    return reportJson(
      { error: "multipart create failed", detail: String(err && err.message ? err.message : err).slice(0, 200) },
      400
    );
  }
  pending.uploadId = multipart.uploadId;
  await env.USAGE.put(
    reportMpuKey(multipart.uploadId),
    JSON.stringify({ ...pending, userId, uploadId: multipart.uploadId }),
    { expirationTtl: REPORT_MPU_STALE_SECONDS + 60 }
  );
  const activeKey = reportActiveKey(userId);
  const active = pruneActive(parseJson(await env.USAGE.get(activeKey), []), nowSec);
  const idx = active.findIndex((item) => item.token === token);
  if (idx >= 0) active[idx] = { ...active[idx], uploadId: multipart.uploadId };
  await env.USAGE.put(activeKey, JSON.stringify(active), { expirationTtl: REPORT_TTL_SECONDS + 3600 });
  return reportJson({
    token,
    key,
    uploadId: multipart.uploadId,
    partSize: 8 * 1024 * 1024,
    expiresAt,
  });
}

export async function reportMpuPart(request, env, url) {
  if (!env.SHARES) return reportJson({ error: "report storage not configured" }, 503);
  const key = String(url.searchParams.get("key") || "");
  const uploadId = String(url.searchParams.get("uploadId") || "");
  const partNumber = Number(url.searchParams.get("partNumber") || 0);
  if (!key.startsWith(REPORT_PREFIX) || key.includes("..") || !uploadId || !Number.isFinite(partNumber) || partNumber < 1) {
    return reportJson({ error: "bad multipart part" }, 400);
  }
  const declared = parseInt(request.headers.get("content-length") || "0", 10);
  if (!Number.isFinite(declared) || declared <= 0 || declared > REPORT_MAX_BYTES) {
    return reportJson({ error: "report part too large" }, 413);
  }
  try {
    const upload = env.SHARES.resumeMultipartUpload(key, uploadId);
    const part = await upload.uploadPart(partNumber, request.body);
    return reportJson({ partNumber, etag: part.etag });
  } catch (err) {
    return reportJson(
      { error: "multipart part failed", detail: String(err && err.message ? err.message : err).slice(0, 200) },
      400
    );
  }
}

export async function reportMpuComplete(request, env, userId, nowSec = Math.floor(Date.now() / 1000)) {
  if (!env.SHARES) return reportJson({ error: "report storage not configured" }, 503);
  let body;
  try {
    body = await request.json();
  } catch (_) {
    return reportJson({ error: "bad json" }, 400);
  }
  const token = String(body.token || "");
  const key = String(body.key || "");
  const uploadId = String(body.uploadId || "");
  const parts = Array.isArray(body.parts) ? body.parts : [];
  if (!isReportToken(token) || key !== reportObjectKey(token) || !uploadId || !parts.length) {
    return reportJson({ error: "invalid complete payload" }, 400);
  }
  const tracked = env.USAGE ? parseJson(await env.USAGE.get(reportMpuKey(uploadId)), null) : null;
  if (tracked && tracked.userId && tracked.userId !== userId) {
    return reportJson({ error: "forbidden" }, 403);
  }
  const normalized = [];
  for (const p of parts) {
    const partNumber = Number(p.partNumber || p.PartNumber || 0);
    const etag = String(p.etag || p.ETag || "");
    if (!Number.isFinite(partNumber) || partNumber < 1 || !etag) {
      return reportJson({ error: "invalid part etag" }, 400);
    }
    normalized.push({ partNumber, etag });
  }
  normalized.sort((a, b) => a.partNumber - b.partNumber);
  let object;
  try {
    const upload = env.SHARES.resumeMultipartUpload(key, uploadId);
    object = await upload.complete(normalized);
  } catch (err) {
    try {
      const upload = env.SHARES.resumeMultipartUpload(key, uploadId);
      await upload.abort();
    } catch (_) {}
    await releasePendingReport(env, userId, nowSec, { uploadId, token });
    return reportJson(
      { error: "multipart complete failed", detail: String(err && err.message ? err.message : err).slice(0, 200) },
      400
    );
  }
  if (!object) {
    await releasePendingReport(env, userId, nowSec, { uploadId, token });
    return reportJson({ error: "report upload failed" }, 500);
  }
  if (object.size != null && object.size > REPORT_MAX_BYTES) {
    try {
      await env.SHARES.delete(key);
    } catch (_) {}
    await releasePendingReport(env, userId, nowSec, { uploadId, token });
    return reportJson({ error: "report file too large" }, 413);
  }
  try {
    const head = await env.SHARES.get(key, { range: { offset: 0, length: 4 } });
    const buf = head ? new Uint8Array(await head.arrayBuffer()) : new Uint8Array();
    if (!looksLikeZipMagic(buf)) {
      await env.SHARES.delete(key);
      await releasePendingReport(env, userId, nowSec, { uploadId, token });
      return reportJson({ error: "zip magic required" }, 415);
    }
  } catch (_) {
    /* range 失敗仍允許：R2 mock／部分實作可能不支援 range */
  }

  const expiresAt = Number(object.customMetadata?.expiresAt || 0) || nowSec + REPORT_TTL_SECONDS;
  const category = String(tracked?.category || object.customMetadata?.category || "other");
  const packLabel = sanitizePackLabel(tracked?.packName, tracked?.packUnrelated === true);
  const downloadUrl = publicReportUrl(publicBase(env, request), token);
  const text = buildWebhookText({
    category,
    packLabel,
    packVersion: tracked?.packVersion,
    errorCode: tracked?.errorCode,
    toolVersion: tracked?.toolVersion,
    url: downloadUrl,
  });
  const notified = await notifyDiscord(env, text);
  if (!notified.ok) {
    try {
      await env.SHARES.delete(key);
    } catch (_) {}
    await releasePendingReport(env, userId, nowSec, { uploadId, token });
    return reportJson({ error: notified.error }, 502);
  }
  await finalizeReportQuota(env, userId, nowSec, {
    token,
    key,
    uploadId,
    expiresAt,
    category,
  });
  if (env.USAGE) {
    try {
      await env.USAGE.delete(reportMpuKey(uploadId));
    } catch (_) {}
  }
  return reportJson({
    ok: true,
    expiresAt,
    message: "已送出（資料 3 天內刪除）",
  });
}

export async function reportDownload(url, env, headOnly, nowSec = Math.floor(Date.now() / 1000)) {
  const token = decodeURIComponent(url.pathname.slice("/report/".length).replace(/\/$/, ""));
  if (!isReportToken(token) || !env.SHARES) return reportJson({ error: "not found" }, 404);
  const key = reportObjectKey(token);
  const object = await env.SHARES.get(key);
  if (!object) return reportJson({ error: "not found" }, 404);
  const expiresAt = Number(object.customMetadata?.expiresAt || 0);
  if (expiresAt && expiresAt <= nowSec) {
    try {
      await env.SHARES.delete(key);
    } catch (_) {}
    return reportJson({ error: "not found" }, 404);
  }
  const headers = new Headers();
  if (typeof object.writeHttpMetadata === "function") object.writeHttpMetadata(headers);
  headers.set("content-type", "application/zip");
  if (object.size != null) headers.set("content-length", String(object.size));
  headers.set("cache-control", "no-store, max-age=0");
  headers.set("content-disposition", `attachment; filename="diagnose-${token.slice(0, 8)}.zip"`);
  return new Response(headOnly ? null : object.body, { headers });
}

export async function cleanupReports(env, nowSec = Math.floor(Date.now() / 1000)) {
  if (env?.SHARES) {
    const listing = await env.SHARES.list({ prefix: REPORT_PREFIX, limit: 1000 });
    await Promise.all(
      (listing.objects || []).map(async (object) => {
        const head = await env.SHARES.head(object.key);
        if (!head || Number(head.customMetadata?.expiresAt || 0) > nowSec) return;
        try {
          await env.SHARES.delete(object.key);
        } catch (_) {}
        const token = String(object.key || "")
          .slice(REPORT_PREFIX.length)
          .replace(/\.zip$/i, "");
        if (token && env.USAGE) {
          try {
            await env.USAGE.delete(reportIdKey(token));
          } catch (_) {}
        }
      })
    );
  }
  if (!env?.USAGE || !env?.SHARES) return;
  let cursor;
  do {
    const page = await env.USAGE.list({ prefix: "report:mpu:", limit: 1000, cursor });
    for (const entry of page.keys || []) {
      const rec = parseJson(await env.USAGE.get(entry.name), null);
      if (!rec) continue;
      if (nowSec - Number(rec.createdAt || 0) < REPORT_MPU_STALE_SECONDS) continue;
      try {
        const upload = env.SHARES.resumeMultipartUpload(rec.key, rec.uploadId);
        await upload.abort();
      } catch (_) {}
      try {
        await env.USAGE.delete(entry.name);
      } catch (_) {}
      await releasePendingReport(env, rec.userId, nowSec, rec);
    }
    cursor = page.list_complete ? null : page.cursor;
  } while (cursor);
}
