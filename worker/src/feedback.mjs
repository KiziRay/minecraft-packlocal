import { corsHeaders } from "./cors.mjs";
import { isSafeOutboundUrl } from "./security.mjs";

const JSON_HEADERS = { "content-type": "application/json; charset=utf-8" };

function json(obj, status = 200, request) {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { ...JSON_HEADERS, "cache-control": "no-store", ...corsHeaders(request) },
  });
}

function utcDay() {
  return new Date().toISOString().slice(0, 10); // YYYY-MM-DD (UTC)
}

function nowMs() {
  return Date.now();
}

function validClientId(s) {
  return typeof s === "string" && /^[A-Za-z0-9_-]{8,64}$/.test(s);
}

function parseIntSafe(v) {
  const n = typeof v === "number" ? v : parseInt(String(v), 10);
  return Number.isFinite(n) ? n : null;
}

function sanitizeNote(note, limitChars = 800) {
  if (note == null) return "";
  const s = String(note).replace(/\0/g, "").trim();
  if (!s) return "";
  return s.length > limitChars ? s.slice(0, limitChars) : s;
}

async function notifyDiscordFeedback(env, content) {
  const hook = env?.DISCORD_FEEDBACK_WEBHOOK && String(env.DISCORD_FEEDBACK_WEBHOOK).trim();
  if (!hook) return { ok: false, error: "feedback webhook not configured" };

  let resp;
  try {
    if (!isSafeOutboundUrl(hook)) return { ok: false, error: "feedback webhook blocked" };
    resp = await fetch(hook, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ content: String(content || "").slice(0, 1800) }),
      signal: AbortSignal.timeout(8000),
    });
  } catch (_) {
    return { ok: false, error: "feedback notify failed" };
  }
  if (!resp.ok) return { ok: false, error: "feedback notify failed" };
  return { ok: true };
}

const FEEDBACK_PAIN_LABELS = {
  incomplete: "翻不乾淨",
  apply_find: "不好套用或找不到結果",
  ai_slow: "AI 慢或不穩",
  ui_hard: "介面難用（字小／找不到設定）",
  other: "其他",
  none: "沒問題",
};

const FEEDBACK_WISH_LABELS = {
  more_mods: "更多模組翻得到",
  quests: "任務／書本更準",
  faster: "更快少額度",
  docs: "說明更清楚",
  other: "其他",
};

function feedbackLabel(map, key) {
  const k = String(key || "").trim();
  return map[k] || k || "—";
}

export async function submitFeedback(request, env) {
  if (!env?.USAGE) return json({ ok: false, error: "usage not configured" }, 503);
  if (!env?.DISCORD_FEEDBACK_WEBHOOK) return json({ ok: false, error: "feedback webhook not configured" }, 503);

  let body;
  try {
    body = await request.json();
  } catch (_) {
    return json({ ok: false, error: "bad json" }, 400);
  }

  const clientId = String(body?.clientId || "").trim();
  if (!validClientId(clientId)) return json({ ok: false, error: "clientId invalid" }, 400);

  const rating = parseIntSafe(body?.rating);
  if (rating != null && (rating < 1 || rating > 5)) return json({ ok: false, error: "rating invalid" }, 400);

  const note = sanitizeNote(body?.note);
  const painPoint = String(body?.painPoint || "").trim().slice(0, 48);
  const wish = String(body?.wish || "").trim().slice(0, 48);
  const source = String(body?.source || "").trim().slice(0, 40);
  const toolVersion = String(body?.toolVersion || "").trim().slice(0, 24);

  // rate limit: 每 clientId 每天 <= 1，且至少間隔 >= 7 天（防濫用重送）
  const day = utcDay();
  const dailyKey = `feedback:day:${day}:${clientId}`;
  const lastKey = `feedback:last:${clientId}`;

  const dailyHit = await env.USAGE.get(dailyKey);
  if (dailyHit) return json({ ok: false, error: "rate_limited_day" }, 429);

  const lastRaw = await env.USAGE.get(lastKey);
  const last = parseIntSafe(lastRaw);
  const minIntervalMs = 7 * 24 * 60 * 60 * 1000;
  if (last != null && nowMs() - last < minIntervalMs) return json({ ok: false, error: "rate_limited_interval" }, 429);

  // 寫入限流（KV 無法保證 CAS；此處接受少量 race，但仍可因 dailyHit 擋住多數重送）
  try {
    await env.USAGE.put(dailyKey, "1", { expirationTtl: 172800 });
    await env.USAGE.put(lastKey, String(nowMs()), { expirationTtl: 45 * 24 * 60 * 60 });
  } catch (_) {
    // KV 寫入失敗仍允許送 webhook，但不讓它變成 spam relay
  }

  const shortClient = clientId.length > 10 ? clientId.slice(0, 10) + "…" : clientId;
  const content = [
    "MCPL 使用回饋（匿名）",
    `clientId: ${shortClient}`,
    painPoint ? `pain: ${feedbackLabel(FEEDBACK_PAIN_LABELS, painPoint)} (${painPoint})` : null,
    wish ? `wish: ${feedbackLabel(FEEDBACK_WISH_LABELS, wish)} (${wish})` : null,
    rating != null ? `rating: ${rating}/5` : null,
    note ? `note: ${note}` : null,
    source ? `source: ${source}` : null,
    toolVersion ? `toolVersion: ${toolVersion}` : null,
  ]
    .filter(Boolean)
    .join("\n");

  const notify = await notifyDiscordFeedback(env, content);
  if (!notify.ok) return json({ ok: false, error: notify.error }, 503);

  return json({ ok: true });
}

