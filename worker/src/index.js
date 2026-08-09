// modpack-i18n Cloudflare Worker
//
// 兩個職責：
//  1. GET  /api/desktop/latest   → 桌面版更新檢查（回最新版本 + 下載連結）
//  2. POST /v1/chat/completions  → AI 翻譯代理（注入伺服器端 DeepSeek 金鑰）
//
// 為什麼要代理而不是把金鑰編進 exe：
//  - 金鑰若進 exe，任何人反編譯就能抽出，開發者的免費額度幾天內被刷爆。
//  - 代理讓金鑰只存在 Worker secret，且可限流／隨時切換／統計用量。
//
// 客戶端在使用者「沒有自填金鑰」時走這裡；使用者自填金鑰則直連上游，不經本 Worker。

const JSON_HEADERS = { "content-type": "application/json; charset=utf-8" };

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    // CORS 預檢（WebView 內其實同源，但保險起見）
    if (request.method === "OPTIONS") {
      return new Response(null, { headers: corsHeaders() });
    }

    if (url.pathname === "/api/desktop/latest" && request.method === "GET") {
      return latest(env);
    }

    // 安裝檔下載：直接從 R2 串流。/download/<檔名>
    if (url.pathname.startsWith("/download/") && (request.method === "GET" || request.method === "HEAD")) {
      return download(url, env, request.method === "HEAD");
    }

    if (url.pathname === "/v1/chat/completions" && request.method === "POST") {
      return proxyChat(request, env);
    }

    // 共享翻譯記憶（社群）：keyed by (模組, key, 原文) 的雜湊，存 R2、依模組分片。
    if (url.pathname === "/tm/lookup" && request.method === "POST") {
      return tmLookup(request, env);
    }
    if (url.pathname === "/tm/contribute" && request.method === "POST") {
      return tmContribute(request, env);
    }

    // 健康檢查
    if (url.pathname === "/" || url.pathname === "/health") {
      // hasKey：代管金鑰是否已正確設定（只回布林，不洩漏值）——設好 secret 後可用來自我驗證
      return json({
        ok: true,
        service: "modpack-i18n",
        version: env.LATEST_VERSION,
        hasKey: !!(env.DEEPSEEK_KEY && String(env.DEEPSEEK_KEY).trim()),
      });
    }

    return json({ error: "not found" }, 404);
  },
};

// ───────────────────────── 更新端點 ─────────────────────────

function latest(env) {
  return json({
    version: env.LATEST_VERSION || "0.0.0",
    url: env.DOWNLOAD_URL || "",
    notes: env.RELEASE_NOTES || "",
    sha256: env.INSTALLER_SHA256 || "",
  });
}

// ───────────────────────── 共享翻譯記憶（R2，依模組分片）─────────────────────────
//
// 儲存：tm/v1/<namespace>.json = { "<keyhash>": "zh", … }。keyhash = sha256(ns\0key\0src)[:24]。
// 用 R2（沒有 KV 權限）；讀改寫為 last-write-wins，偶爾遺漏會在下次翻譯自動補回。
// 只存字串，無任何身分/個資。

const TM_MAX_ITEMS = 5000;
const TM_MAX_ZH_LEN = 400;
const TM_SHARD_CAP = 200000; // 單模組分片最多條數（防惡意灌爆）

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
  const obj = await env.DOWNLOADS.get(tmShardKey(ns));
  if (!obj) return null;
  try {
    const buf = await obj.arrayBuffer();
    return JSON.parse(await gunzipToStr(buf));
  } catch (_) {
    return null;
  }
}

async function tmLookup(request, env) {
  if (!env.DOWNLOADS) return json({ hits: {} });
  let body;
  try {
    body = await request.json();
  } catch (_) {
    return json({ error: "bad json" }, 400);
  }
  const items = Array.isArray(body.items) ? body.items.slice(0, TM_MAX_ITEMS) : [];
  const byNs = new Map();
  for (const it of items) {
    if (!it || !tmValidNs(it.ns) || !tmValidKh(it.kh)) continue;
    if (!byNs.has(it.ns)) byNs.set(it.ns, new Set());
    byNs.get(it.ns).add(it.kh);
  }
  const hits = {};
  const nss = [...byNs.keys()];
  const CONC = 8;
  for (let i = 0; i < nss.length; i += CONC) {
    await Promise.all(
      nss.slice(i, i + CONC).map(async (ns) => {
        const shard = await tmReadShard(env, ns);
        if (!shard) return;
        for (const kh of byNs.get(ns)) {
          const zh = shard[kh];
          if (typeof zh === "string" && zh) hits[kh] = zh;
        }
      })
    );
  }
  return json({ hits });
}

async function tmContribute(request, env) {
  if (!env.DOWNLOADS) return json({ ok: false, accepted: 0 });
  let body;
  try {
    body = await request.json();
  } catch (_) {
    return json({ error: "bad json" }, 400);
  }
  const items = Array.isArray(body.items) ? body.items.slice(0, TM_MAX_ITEMS) : [];
  const byNs = new Map();
  for (const it of items) {
    if (!it || !tmValidNs(it.ns) || !tmValidKh(it.kh)) continue;
    const zh = typeof it.zh === "string" ? it.zh.trim() : "";
    if (!zh || zh.length > TM_MAX_ZH_LEN) continue;
    if (!byNs.has(it.ns)) byNs.set(it.ns, {});
    byNs.get(it.ns)[it.kh] = zh;
  }
  let accepted = 0;
  for (const [ns, entries] of byNs) {
    const shard = (await tmReadShard(env, ns)) || {};
    let changed = false;
    for (const [kh, zh] of Object.entries(entries)) {
      if (Object.keys(shard).length >= TM_SHARD_CAP && !(kh in shard)) continue;
      if (shard[kh] !== zh) {
        shard[kh] = zh;
        changed = true;
        accepted++;
      }
    }
    // 只有真的有新條目才寫（避免重複寫入）；寫的是 gzip 後的位元組（省容量）
    if (changed) {
      const gz = await gzipBytes(JSON.stringify(shard));
      await env.DOWNLOADS.put(tmShardKey(ns), gz, {
        httpMetadata: { contentType: "application/gzip" },
      });
    }
  }
  return json({ ok: true, accepted });
}

// ───────────────────────── 安裝檔下載（R2）─────────────────────────

async function download(url, env, headOnly) {
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
  headers.set("access-control-allow-origin", "*");
  return new Response(headOnly ? null : obj.body, { headers });
}

// ───────────────────────── AI 代理 ─────────────────────────

async function proxyChat(request, env) {
  // trim：擋掉空字串／只有換行的 secret（貼進遮罩提示常見的坑），也避免結尾換行害上游 401。
  const key = env.DEEPSEEK_KEY && String(env.DEEPSEEK_KEY).trim();
  if (!key) {
    // secret 未設或值為空：明確告訴客戶端這是「服務端未就緒」，不是使用者金鑰問題。
    return json(
      { error: { message: "managed translation not configured", type: "server_not_ready" } },
      503
    );
  }

  // 每日總量保護：超過就回 429，客戶端顯示贊助提示。
  const budget = parseInt(env.DAILY_TOKEN_BUDGET || "0", 10);
  if (budget > 0 && env.USAGE) {
    const dayKey = "usage:" + utcDay();
    const spent = parseInt((await env.USAGE.get(dayKey)) || "0", 10);
    if (spent >= budget) {
      return json(
        {
          error: {
            message: "daily free translation budget reached",
            type: "insufficient_quota",
          },
        },
        429
      );
    }
  }

  let body;
  try {
    body = await request.json();
  } catch (_) {
    return json({ error: { message: "invalid json body" } }, 400);
  }

  // 只允許聊天補全所需欄位轉發，並鎖定模型（避免被拿去打別的昂貴模型）。
  const forward = {
    model: env.UPSTREAM_MODEL || body.model || "deepseek-chat",
    messages: body.messages,
    temperature: typeof body.temperature === "number" ? body.temperature : 0.1,
  };

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
  if (resp.ok && budget > 0 && env.USAGE) {
    try {
      const used = estimateTokens(text, forward);
      const dayKey = "usage:" + utcDay();
      const spent = parseInt((await env.USAGE.get(dayKey)) || "0", 10);
      // 隔日自動歸零：TTL 略長於一天。
      await env.USAGE.put(dayKey, String(spent + used), { expirationTtl: 172800 });
    } catch (_) {
      /* 記帳失敗不影響翻譯 */
    }
  }

  // 原樣回傳上游狀態與內容，客戶端既有的 402/429 判斷即可運作。
  return new Response(text, {
    status: resp.status,
    headers: { ...JSON_HEADERS, ...corsHeaders() },
  });
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

function corsHeaders() {
  return {
    "access-control-allow-origin": "*",
    "access-control-allow-methods": "GET, POST, OPTIONS",
    "access-control-allow-headers": "content-type, authorization",
  };
}

function json(obj, status = 200) {
  return new Response(JSON.stringify(obj), {
    status,
    // no-store：版本檢查等 API 一定要拿到最新值，不能被邊緣或客戶端快取住舊版本資訊。
    headers: { ...JSON_HEADERS, "cache-control": "no-store", ...corsHeaders() },
  });
}
