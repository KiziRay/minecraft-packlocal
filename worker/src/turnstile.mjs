// Cloudflare Turnstile：桌面版代管 AI 的真人驗證。
//
// 原始 Turnstile token 只在本 Worker 送往 Siteverify，驗證後立即失效。
// 桌面端收到的是 HMAC 簽章、綁定 Discord user id 的短效通行憑證，
// 可供同一次翻譯的多個平行批次使用，不會把 Turnstile Secret 放進 exe。

const TURNSTILE_ACTION = "managed-ai";
const CHALLENGE_TTL_SECONDS = 5 * 60;
const ACCESS_TTL_SECONDS = 2 * 60 * 60;
const CALLBACK_PORT_MIN = 19431;
const CALLBACK_PORT_MAX = 19440;
const SITEVERIFY_URL = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

const JSON_HEADERS = {
  "content-type": "application/json; charset=utf-8",
  "cache-control": "no-store",
};

export function turnstileConfigured(env) {
  return !!(
    clean(env.TURNSTILE_SITE_KEY) &&
    clean(env.TURNSTILE_SECRET_KEY) &&
    clean(env.TURNSTILE_PROOF_SECRET).length >= 32
  );
}

export function isAllowedLoopbackCallback(value) {
  try {
    const url = new URL(String(value || ""));
    const port = Number(url.port);
    return (
      url.protocol === "http:" &&
      url.hostname === "127.0.0.1" &&
      port >= CALLBACK_PORT_MIN &&
      port <= CALLBACK_PORT_MAX &&
      url.pathname === "/turnstile-callback" &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash
    );
  } catch (_) {
    return false;
  }
}

export async function startTurnstile(request, env, userId) {
  if (!turnstileConfigured(env)) {
    return apiError("安全驗證尚未完成服務端設定", "turnstile_unavailable", 503);
  }

  let body;
  try {
    body = await request.json();
  } catch (_) {
    return apiError("安全驗證請求格式錯誤", "invalid_request", 400);
  }
  const callback = clean(body && body.callback);
  if (!isAllowedLoopbackCallback(callback)) {
    return apiError("本機 callback 不在允許範圍", "invalid_callback", 400);
  }

  const now = epochSeconds();
  const state = await signPayload(
    {
      v: 1,
      kind: "challenge",
      uid: String(userId),
      callback,
      nonce: crypto.randomUUID(),
      exp: now + CHALLENGE_TTL_SECONDS,
    },
    env.TURNSTILE_PROOF_SECRET
  );
  const origin = new URL(request.url).origin;
  return json({
    ok: true,
    url: `${origin}/turnstile?state=${encodeURIComponent(state)}`,
    expiresAt: now + CHALLENGE_TTL_SECONDS,
  });
}

export async function renderTurnstile(request, env) {
  if (!turnstileConfigured(env)) {
    return htmlPage("安全驗證暫時無法使用", "服務端尚未完成設定，請稍後再試。", 503);
  }
  const state = clean(new URL(request.url).searchParams.get("state"));
  const payload = await verifySignedPayload(state, env.TURNSTILE_PROOF_SECRET, "challenge");
  if (!validChallengePayload(payload)) {
    return htmlPage("驗證連結已失效", "請回到翻譯工具重新開始安全驗證。", 400);
  }

  const siteKey = escapeHtml(clean(env.TURNSTILE_SITE_KEY));
  const safeState = escapeHtml(state);
  const page = `<!doctype html>
<html lang="zh-Hant-TW"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>安全驗證｜模組包翻譯工具</title>
<script src="https://challenges.cloudflare.com/turnstile/v0/api.js" async defer></script>
<style>:root{color-scheme:dark}*{box-sizing:border-box}body{margin:0;min-height:100vh;display:grid;place-items:center;background:#111315;color:#eeece6;font:16px/1.65 system-ui,"Microsoft JhengHei",sans-serif}.card{width:min(92vw,460px);padding:30px;border:1px solid #34383d;background:#191c1f}p{color:#aeb3b7;margin:8px 0 22px}button{width:100%;margin-top:18px;padding:12px 16px;border:0;background:#d97842;color:#fff;font:inherit;font-weight:700;cursor:pointer}.note{font-size:13px;color:#7f858a;margin:15px 0 0}</style></head>
<body><main class="card"><h1>完成安全驗證</h1><p>這項驗證只用來保護開發者提供的翻譯額度。</p>
<form method="post" action="/api/turnstile/verify"><input type="hidden" name="state" value="${safeState}">
<div class="cf-turnstile" data-sitekey="${siteKey}" data-action="${TURNSTILE_ACTION}" data-theme="dark"></div>
<button type="submit">完成並回到工具</button></form><p class="note">驗證結果不會取代 Discord 會員檢查。</p></main></body></html>`;

  return new Response(page, {
    status: 200,
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
      "referrer-policy": "no-referrer",
      "x-content-type-options": "nosniff",
      "content-security-policy": "default-src 'none'; script-src https://challenges.cloudflare.com; frame-src https://challenges.cloudflare.com; connect-src https://challenges.cloudflare.com; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
    },
  });
}

export async function completeTurnstile(request, env) {
  if (!turnstileConfigured(env)) {
    return htmlPage("安全驗證暫時無法使用", "服務端尚未完成設定，請稍後再試。", 503);
  }

  let form;
  try {
    form = await request.formData();
  } catch (_) {
    return htmlPage("驗證資料無法讀取", "請回到翻譯工具重新驗證。", 400);
  }
  const state = clean(form.get("state"));
  const token = clean(form.get("cf-turnstile-response"));
  const payload = await verifySignedPayload(state, env.TURNSTILE_PROOF_SECRET, "challenge");
  if (!validChallengePayload(payload) || token.length < 1 || token.length > 2048) {
    return htmlPage("驗證未完成", "請回到翻譯工具重新驗證。", 400);
  }

  const verifyBody = new FormData();
  verifyBody.set("secret", clean(env.TURNSTILE_SECRET_KEY));
  verifyBody.set("response", token);
  verifyBody.set("idempotency_key", crypto.randomUUID());
  const remoteIp = clean(request.headers.get("CF-Connecting-IP"));
  if (remoteIp) verifyBody.set("remoteip", remoteIp);

  let outcome;
  try {
    const response = await fetch(SITEVERIFY_URL, {
      method: "POST",
      body: verifyBody,
      signal: AbortSignal.timeout(8000),
    });
    if (!response.ok) throw new Error("siteverify unavailable");
    outcome = await response.json();
  } catch (_) {
    return htmlPage("驗證服務暫時無法使用", "請稍後回到翻譯工具重新驗證。", 503);
  }

  const expectedHostname = clean(env.TURNSTILE_HOSTNAME) || new URL(request.url).hostname;
  if (
    outcome.success !== true ||
    outcome.action !== TURNSTILE_ACTION ||
    outcome.hostname !== expectedHostname
  ) {
    return htmlPage("安全驗證失敗", "驗證已過期或無效，請回到翻譯工具重試。", 403);
  }

  const now = epochSeconds();
  const expiresAt = now + ACCESS_TTL_SECONDS;
  const proof = await signPayload(
    {
      v: 1,
      kind: "access",
      uid: payload.uid,
      nonce: crypto.randomUUID(),
      exp: expiresAt,
    },
    env.TURNSTILE_PROOF_SECRET
  );
  return callbackPage(payload.callback, proof, expiresAt);
}

export async function verifyTurnstileAccess(proof, env, userId) {
  if (!turnstileConfigured(env)) {
    return { ok: false, type: "turnstile_unavailable" };
  }
  const payload = await verifySignedPayload(
    clean(proof),
    env.TURNSTILE_PROOF_SECRET,
    "access"
  );
  if (!payload || payload.uid !== String(userId) || !validExpiry(payload.exp)) {
    return { ok: false, type: "turnstile_required" };
  }
  return { ok: true, expiresAt: payload.exp };
}

export async function signPayload(payload, secret) {
  const body = base64UrlEncode(new TextEncoder().encode(JSON.stringify(payload)));
  const key = await importHmacKey(secret, ["sign"]);
  const signature = await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(body));
  return `${body}.${base64UrlEncode(new Uint8Array(signature))}`;
}

export async function verifySignedPayload(token, secret, expectedKind) {
  if (typeof token !== "string" || token.length < 20 || token.length > 4096) return null;
  const parts = token.split(".");
  if (parts.length !== 2 || !parts[0] || !parts[1]) return null;
  try {
    const key = await importHmacKey(secret, ["verify"]);
    const valid = await crypto.subtle.verify(
      "HMAC",
      key,
      base64UrlDecode(parts[1]),
      new TextEncoder().encode(parts[0])
    );
    if (!valid) return null;
    const payload = JSON.parse(new TextDecoder().decode(base64UrlDecode(parts[0])));
    if (!payload || payload.v !== 1 || payload.kind !== expectedKind || !validExpiry(payload.exp)) {
      return null;
    }
    return payload;
  } catch (_) {
    return null;
  }
}

function validChallengePayload(payload) {
  return !!(
    payload &&
    /^\d{5,25}$/.test(String(payload.uid || "")) &&
    typeof payload.nonce === "string" &&
    payload.nonce.length >= 16 &&
    isAllowedLoopbackCallback(payload.callback) &&
    validExpiry(payload.exp)
  );
}

function validExpiry(exp) {
  return Number.isInteger(exp) && exp > epochSeconds();
}

function callbackPage(callback, proof, expiresAt) {
  const nonce = crypto.randomUUID().replaceAll("-", "");
  const callbackJson = scriptJson(callback);
  const proofJson = scriptJson(proof);
  const page = `<!doctype html><html lang="zh-Hant-TW"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>驗證完成</title>
<style>:root{color-scheme:dark}body{margin:0;min-height:100vh;display:grid;place-items:center;background:#111315;color:#eeece6;font:16px/1.6 system-ui,"Microsoft JhengHei",sans-serif;text-align:center}main{padding:32px}p{color:#aeb3b7}</style></head>
<body><main><h1 id="title">驗證完成</h1><p id="note">正在通知翻譯工具…</p></main>
<script nonce="${nonce}">(async()=>{try{const r=await fetch(${callbackJson},{method:"POST",headers:{"content-type":"application/json"},body:JSON.stringify({proof:${proofJson},expiresAt:${expiresAt}})});if(!r.ok)throw new Error();document.getElementById("note").textContent="可以關閉這個分頁，回到翻譯工具。";setTimeout(()=>window.close(),1200)}catch(_){document.getElementById("title").textContent="無法通知工具";document.getElementById("note").textContent="請確認工具仍開啟，再重新驗證。"}})();</script></body></html>`;
  return new Response(page, {
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
      "referrer-policy": "no-referrer",
      "x-content-type-options": "nosniff",
      "content-security-policy": `default-src 'none'; script-src 'nonce-${nonce}'; style-src 'unsafe-inline'; connect-src http://127.0.0.1:*; base-uri 'none'; frame-ancestors 'none'`,
    },
  });
}

function htmlPage(title, message, status) {
  const page = `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${escapeHtml(title)}</title><body style="margin:0;min-height:100vh;display:grid;place-items:center;background:#111315;color:#eeece6;font:16px/1.6 system-ui,'Microsoft JhengHei',sans-serif;text-align:center"><main><h1>${escapeHtml(title)}</h1><p style="color:#aeb3b7">${escapeHtml(message)}</p></main></body>`;
  return new Response(page, {
    status,
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
      "referrer-policy": "no-referrer",
      "x-content-type-options": "nosniff",
      "content-security-policy": "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'",
    },
  });
}

function apiError(message, type, status) {
  return json({ error: { message, type } }, status);
}

function json(value, status = 200) {
  return new Response(JSON.stringify(value), { status, headers: JSON_HEADERS });
}

async function importHmacKey(secret, usages) {
  return crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(clean(secret)),
    { name: "HMAC", hash: "SHA-256" },
    false,
    usages
  );
}

function base64UrlEncode(bytes) {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/g, "");
}

function base64UrlDecode(value) {
  const normalized = value.replaceAll("-", "+").replaceAll("_", "/");
  const binary = atob(normalized + "=".repeat((4 - (normalized.length % 4)) % 4));
  return Uint8Array.from(binary, (char) => char.charCodeAt(0));
}

function scriptJson(value) {
  return JSON.stringify(String(value)).replaceAll("<", "\\u003c");
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function clean(value) {
  return typeof value === "string" ? value.trim() : "";
}

function epochSeconds() {
  return Math.floor(Date.now() / 1000);
}
