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
  const status = turnstileStatus(env);
  return status.siteKey && status.siteSecret && status.proofSecret;
}

/** 回傳尚未就緒的 env 名稱（僅名稱，不含值）。 */
export function turnstileMissingNames(env) {
  const status = turnstileStatus(env);
  const missing = [];
  if (!status.siteKey) missing.push("TURNSTILE_SITE_KEY");
  if (!status.siteSecret) missing.push("TURNSTILE_SECRET_KEY");
  if (!status.proofSecret) missing.push("TURNSTILE_PROOF_SECRET");
  return missing;
}

function turnstileUnavailableMessage(env) {
  const missing = turnstileMissingNames(env);
  if (!missing.length) {
    return "安全驗證尚未完成服務端設定。請管理員檢查 Turnstile 設定後重新部署，使用者可改用自訂 API。";
  }
  return `Worker 缺少 ${missing.join("／")}。請管理員用 wrangler.toml [vars]（SITE_KEY）或 wrangler secret put（SECRET／PROOF）設定後確認 /health，使用者可改用自訂 API。`;
}

export { turnstileUnavailableMessage };

// Safe diagnostics only: never expose secret values or their lengths.
export function turnstileStatus(env) {
  return {
    siteKey: Boolean(clean(env.TURNSTILE_SITE_KEY)),
    siteSecret: Boolean(clean(env.TURNSTILE_SECRET_KEY)),
    proofSecret: clean(env.TURNSTILE_PROOF_SECRET).length >= 32,
    enforced: String(env.TURNSTILE_ENFORCED || "") === "1",
  };
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
    return apiError(turnstileUnavailableMessage(env), "turnstile_unavailable", 503);
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
    return htmlPage(
      "安全驗證尚未完成服務端設定",
      turnstileUnavailableMessage(env),
      503
    );
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
    return htmlPage(
      "安全驗證尚未完成服務端設定",
      turnstileUnavailableMessage(env),
      503
    );
  }

  let form;
  try {
    form = await request.formData();
  } catch (_) {
    return htmlPage("驗證資料無法讀取", "請回到翻譯工具重新驗證。", 400);
  }
  const state = clean(form.get("state"));
  const token = clean(form.get("cf-turnstile-response"));
  const secret = clean(env.TURNSTILE_SECRET_KEY);
  const payload = await verifySignedPayload(state, env.TURNSTILE_PROOF_SECRET, "challenge");
  if (!validChallengePayload(payload)) {
    return htmlPage("驗證未完成", "挑戰狀態無效或已過期。請回到翻譯工具重新驗證。", 400);
  }
  if (!secret) {
    return htmlPage(
      "安全驗證尚未完成服務端設定",
      "Worker 缺少 TURNSTILE_SECRET_KEY。請管理員用 wrangler secret put 設定後再試。",
      503
    );
  }
  if (token.length < 1) {
    return htmlPage(
      "缺少驗證 token",
      "請先在頁面完成 Cloudflare 勾選，再按「完成並回到工具」。",
      400
    );
  }
  if (token.length > 2048) {
    return htmlPage("驗證 token 異常", "token 長度超過上限。請回到翻譯工具重新驗證。", 400);
  }

  // Official Siteverify accepts application/x-www-form-urlencoded or JSON (not multipart).
  const params = new URLSearchParams();
  params.set("secret", secret);
  params.set("response", token);
  params.set("idempotency_key", crypto.randomUUID());
  const remoteIp = clean(request.headers.get("CF-Connecting-IP"));
  if (remoteIp) params.set("remoteip", remoteIp);

  const siteverify = await callSiteverify(params);
  if (!siteverify.ok) {
    return htmlPage(siteverify.title, siteverify.message, siteverify.status);
  }
  const outcome = siteverify.outcome;

  const expectedHostname = clean(env.TURNSTILE_HOSTNAME) || new URL(request.url).hostname;
  const failure = describeSiteverifyFailure(outcome, expectedHostname);
  if (failure) {
    return htmlPage(failure.title, failure.message, 403);
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

/**
 * Siteverify 上游呼叫。
 * Cloudflare 對 invalid-input-secret 等業務失敗常回 HTTP 400 + JSON；
 * 必須先解析 body，不可只看 status 就當傳輸異常。
 */
export async function callSiteverify(verifyBody, fetchImpl = fetch) {
  const prepared = prepareSiteverifyRequest(verifyBody);
  if (!prepared.ok) {
    return {
      ok: false,
      status: prepared.status,
      title: prepared.title,
      message: prepared.message,
    };
  }

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 8000);
  try {
    const response = await fetchImpl(SITEVERIFY_URL, {
      method: "POST",
      headers: prepared.headers,
      body: prepared.body,
      signal: controller.signal,
    });
    const rawText = await safeReadText(response);
    const outcome = tryParseSiteverifyJson(rawText);
    // Business failures (invalid secret/token) often arrive as HTTP 400 + JSON.
    if (outcome && typeof outcome === "object" && ("success" in outcome || "error-codes" in outcome)) {
      return { ok: true, outcome, httpStatus: response.status };
    }
    const snippet = truncateUpstreamBody(rawText);
    return {
      ok: false,
      status: 502,
      title: "Cloudflare Siteverify 回應異常",
      message: snippet
        ? `上游 HTTP ${response.status}，body：${snippet}。請管理員檢查 Turnstile Secret／sitekey 是否成對，或稍後重試。`
        : `上游 HTTP ${response.status} 且無可用 JSON。請稍後重試；若持續失敗，請管理員檢查 Turnstile Secret 與網路。`,
    };
  } catch (error) {
    const aborted =
      error &&
      (error.name === "AbortError" || /aborted|timeout/i.test(String(error.message || error)));
    return {
      ok: false,
      status: 503,
      title: aborted ? "Cloudflare Siteverify 逾時" : "無法連線 Cloudflare Siteverify",
      message: aborted
        ? "驗證上游超過 8 秒未回應。請檢查網路後回到翻譯工具重試，或改用自訂 API。"
        : "Worker 連不上 challenges.cloudflare.com。請檢查網路／防火牆後重試，或改用自訂 API。",
    };
  } finally {
    clearTimeout(timer);
  }
}

/** Build urlencoded Siteverify body; reject empty secret/response before upstream call. */
export function prepareSiteverifyRequest(input) {
  let secret = "";
  let responseToken = "";
  let remoteip = "";
  let idempotencyKey = "";

  if (input instanceof URLSearchParams) {
    secret = clean(input.get("secret"));
    responseToken = clean(input.get("response"));
    remoteip = clean(input.get("remoteip"));
    idempotencyKey = clean(input.get("idempotency_key"));
  } else if (input && typeof input === "object" && typeof input.get === "function") {
    // FormData or similar — normalize to urlencoded (officially supported).
    secret = clean(input.get("secret"));
    responseToken = clean(input.get("response"));
    remoteip = clean(input.get("remoteip"));
    idempotencyKey = clean(input.get("idempotency_key"));
  } else if (input && typeof input === "object") {
    secret = clean(input.secret);
    responseToken = clean(input.response);
    remoteip = clean(input.remoteip);
    idempotencyKey = clean(input.idempotency_key);
  }

  if (!secret) {
    return {
      ok: false,
      status: 503,
      title: "缺少 Site Secret",
      message: "TURNSTILE_SECRET_KEY 為空，未呼叫上游。請管理員用 wrangler secret put TURNSTILE_SECRET_KEY 設定。",
    };
  }
  if (!responseToken) {
    return {
      ok: false,
      status: 400,
      title: "缺少驗證 token",
      message: "cf-turnstile-response 為空，未呼叫上游。請先完成頁面勾選再送出。",
    };
  }

  const params = new URLSearchParams();
  params.set("secret", secret);
  params.set("response", responseToken);
  if (remoteip) params.set("remoteip", remoteip);
  if (idempotencyKey) params.set("idempotency_key", idempotencyKey);

  return {
    ok: true,
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: params.toString(),
  };
}

async function safeReadText(response) {
  try {
    return await response.text();
  } catch (_) {
    return "";
  }
}

function tryParseSiteverifyJson(rawText) {
  const text = typeof rawText === "string" ? rawText.trim() : "";
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch (_) {
    return null;
  }
}

/** Truncate upstream body for error pages; never include request secret. */
export function truncateUpstreamBody(rawText, maxLen = 240) {
  const text = String(rawText || "")
    .replace(/\s+/g, " ")
    .trim();
  if (!text) return "";
  // Defense in depth: scrub accidental secret-looking query fragments.
  const scrubbed = text.replace(/secret=[^&\s"]+/gi, "secret=[redacted]");
  return scrubbed.length <= maxLen ? scrubbed : `${scrubbed.slice(0, maxLen)}…`;
}

/** 將 Siteverify 失敗轉成可行動中文（不含密鑰）。 */
export function describeSiteverifyFailure(outcome, expectedHostname) {
  if (!outcome || typeof outcome !== "object") {
    return {
      title: "安全驗證回應無效",
      message: "上游未回傳可用結果。請回到翻譯工具重新驗證。",
    };
  }
  if (outcome.success === true) {
    if (outcome.action !== TURNSTILE_ACTION) {
      return {
        title: "安全驗證 action 不符",
        message: `預期 action「${TURNSTILE_ACTION}」，實際「${String(outcome.action || "")}」。請管理員確認 Turnstile widget 設定。`,
      };
    }
    if (outcome.hostname !== expectedHostname) {
      return {
        title: "安全驗證網域不符",
        message: `預期 hostname「${expectedHostname}」，實際「${String(outcome.hostname || "")}」。請把 Turnstile widget 允許網域設為 Worker 網域。`,
      };
    }
    return null;
  }
  const codes = Array.isArray(outcome["error-codes"])
    ? outcome["error-codes"].map((code) => String(code)).filter(Boolean)
    : [];
  const detail = codes.length ? codes.map(describeTurnstileErrorCode).join("；") : "未提供 error-codes";
  return {
    title: "安全驗證未通過",
    message: `${detail}。請回到翻譯工具重新驗證；若顯示金鑰／網域錯誤，請管理員檢查 Cloudflare Turnstile 設定。`,
  };
}

function describeTurnstileErrorCode(code) {
  switch (code) {
    case "missing-input-secret":
      return "缺少 Site Secret（missing-input-secret）";
    case "invalid-input-secret":
      return "Site Secret 無效或與 Site Key 不成對（invalid-input-secret）";
    case "missing-input-response":
      return "缺少驗證 token（missing-input-response）";
    case "invalid-input-response":
      return "驗證 token 無效或已用過（invalid-input-response）";
    case "timeout-or-duplicate":
      return "驗證 token 逾時或重複送出（timeout-or-duplicate）";
    case "bad-request":
      return "Siteverify 請求格式錯誤（bad-request）";
    case "internal-error":
      return "Cloudflare 內部錯誤（internal-error）";
    default:
      return code;
  }
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
