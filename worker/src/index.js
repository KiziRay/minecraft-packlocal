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
  turnstileUnavailableMessage,
  verifyTurnstileAccess,
} from "./turnstile.mjs";

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

    // 免安裝 EXE 下載：直接從 R2 串流。/download/<檔名>
    if (url.pathname.startsWith("/download/") && (request.method === "GET" || request.method === "HEAD")) {
      return download(url, env, request.method === "HEAD");
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
    if (url.pathname === "/glossary/lookup" && request.method === "POST") {
      return glossaryLookup(request, env);
    }
    if (url.pathname === "/glossary/contribute" && request.method === "POST") {
      return glossaryContribute(request, env);
    }

    // 分享檔使用獨立的 SHARES R2 bucket，不會寫入安裝檔或翻譯記憶。
    if (url.pathname === "/api/share/upload" && request.method === "POST") {
      return shareUpload(request, env);
    }
    if (url.pathname.startsWith("/s/") && (request.method === "GET" || request.method === "HEAD")) {
      return shareDownload(url, env, request.method === "HEAD");
    }

    // 健康檢查
    if (url.pathname === "/" || url.pathname === "/health") {
      // hasKey：代管金鑰是否已正確設定（只回布林，不洩漏值）——設好 secret 後可用來自我驗證
      const turnstile = turnstileStatus(env);
      return json({
        ok: true,
        service: "modpack-i18n",
        version: env.LATEST_VERSION,
        hasKey: !!(env.DEEPSEEK_KEY && String(env.DEEPSEEK_KEY).trim()),
        turnstileReady: turnstileConfigured(env),
        turnstile,
        // 僅缺項名稱，不含值；方便管理員對照 secret list／vars。
        turnstileMissing: turnstileMissingNames(env),
      });
    }

    return json({ error: "not found" }, 404);
  },
  async scheduled(_event, env) {
    await cleanupShares(env);
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

// ───────────────────────── 共享翻譯記憶（R2，依模組分片）─────────────────────────
//
// 儲存：TRANSLATIONS R2 的 tm/v1/<namespace>.json.gz 是精確鍵，
// tm/v2/global.json.gz 是跨模組候選；不與更新檔 DOWNLOADS 混用。
// 只在上下文一致時命中；不同譯文會標記 conflict，避免後來的 AI 結果靜默覆蓋先前結果。
// 只存匿名文字與語境，不存本機路徑、Discord 身分或整合包檔案。

const TM_MAX_ITEMS = 5000;
const TM_MAX_ZH_LEN = 400;
const TM_SHARD_CAP = 200000; // 單模組分片最多條數（防惡意灌爆）
const TM_GLOBAL_CAP = 300000;
const GLOSSARY_MAX_ITEMS = 5000;
const GLOSSARY_CAP = 300000;

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

function tmRecord(value) {
  if (typeof value === "string") return { zh: value, ctx: "", packs: {}, conflict: false };
  if (!value || typeof value !== "object" || typeof value.zh !== "string") return null;
  return {
    zh: value.zh,
    ctx: typeof value.ctx === "string" ? value.ctx : "",
    packs: value.packs && typeof value.packs === "object" ? value.packs : {},
    conflict: value.conflict === true,
  };
}

function tmCanUse(value, ctx) {
  const record = tmRecord(value);
  if (!record || !record.zh.trim() || record.conflict) return null;
  if (record.ctx && ctx && record.ctx !== ctx) return null;
  return record.zh;
}

function tmMerge(target, key, next) {
  const previous = tmRecord(target[key]);
  if (!previous) {
    target[key] = next;
    return next.conflict ? "conflict" : "accepted";
  }
  if (previous.zh === next.zh && (!previous.ctx || !next.ctx || previous.ctx === next.ctx)) {
    const before = Object.keys(previous.packs || {}).length;
    previous.packs = { ...(previous.packs || {}), ...(next.packs || {}) };
    target[key] = previous;
    return Object.keys(previous.packs).length > before ? "accepted" : "duplicate";
  }
  target[key] = { ...previous, conflict: true };
  return "conflict";
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
    if (!byNs.has(it.ns)) byNs.set(it.ns, new Map());
    byNs.get(it.ns).set(it.kh, { ctx, sk });
    queries.set(it.kh, { ctx, sk });
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
          const zh = tmCanUse(shard[kh], query.ctx);
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
      const zh = tmCanUse(global[query.sk], query.ctx);
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
    if (!zh || zh.length > TM_MAX_ZH_LEN) continue;
    const record = {
      zh,
      ctx: typeof it.ctx === "string" ? it.ctx.slice(0, 64) : "",
      packs: validPackKey(it.pk) ? { [it.pk]: typeof it.pn === "string" ? it.pn.slice(0, 120) : "" } : {},
      conflict: false,
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
      } else if (result === "conflict") {
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
      } else if (result === "conflict") {
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
// 只回傳「沒有衝突且已被至少一個來源確認」的譯名；同一術語不同譯文會停用，
// 不讓後來的單一使用者靜默覆蓋既有結果。
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

function glossaryRecord(value) {
  if (!value || typeof value !== "object" || typeof value.zh !== "string") return null;
  return {
    zh: value.zh,
    ctx: typeof value.ctx === "string" ? value.ctx : "",
    packs: value.packs && typeof value.packs === "object" ? value.packs : {},
    conflict: value.conflict === true,
  };
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
    const record = glossaryRecord(glossary[gh]);
    if (!record || record.conflict || (record.ctx && query.ctx && record.ctx !== query.ctx)) continue;
    const packCount = Object.keys(record.packs).length;
    // 只有至少兩個不同整合包確認，才自動採用共享術語；單一來源不夠可靠。
    if (packCount >= 2) hits[gh] = record.zh;
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
  for (const item of items) {
    if (!item || !validGlossaryHash(item.gh) || !validPackKey(item.pk)) continue;
    const zh = typeof item.zh === "string" ? item.zh.trim() : "";
    const pn = typeof item.pn === "string" ? item.pn.trim().slice(0, 120) : "";
    if (!zh || zh.length > TM_MAX_ZH_LEN || !pn) continue;
    const previous = glossaryRecord(glossary[item.gh]);
    if (!previous) {
      glossary[item.gh] = { zh, ctx: typeof item.ctx === "string" ? item.ctx.slice(0, 64) : "", packs: { [item.pk]: pn }, conflict: false };
      accepted++;
      continue;
    }
    if (previous.zh !== zh || (previous.ctx && item.ctx && previous.ctx !== item.ctx)) {
      previous.conflict = true;
      conflicts++;
      continue;
    }
    if (!previous.packs[item.pk]) {
      if (Object.keys(previous.packs).length >= GLOSSARY_CAP) continue;
      previous.packs[item.pk] = pn;
      accepted++;
    }
  }
  if (accepted || conflicts) {
    const gz = await gzipBytes(JSON.stringify(glossary));
    await env.TRANSLATIONS.put(glossaryKey(), gz, { httpMetadata: { contentType: "application/gzip" } });
  }
  return json({ ok: true, accepted, conflicts });
}

// ───────────────────────── 免安裝 EXE 下載（R2）─────────────────────────

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

// ───────────────────────── 一日分享檔（獨立 R2）─────────────────────────

const SHARE_PREFIX = "v1/";
const SHARE_TTL_SECONDS = 24 * 60 * 60;

async function shareUpload(request, env) {
  if (!env.SHARES) return json({ error: "share storage not configured" }, 503);
  const access = await authorizeManagedAi(request, env);
  if (!access.ok) return access.response;
  const type = String(request.headers.get("content-type") || "").split(";")[0].toLowerCase();
  const maxBytes = Math.min(parseInt(env.SHARE_MAX_BYTES || "104857600", 10) || 104857600, 104857600);
  const declared = parseInt(request.headers.get("content-length") || "0", 10);
  if (type !== "application/zip") return json({ error: "zip content type required" }, 415);
  if (!Number.isFinite(declared) || declared <= 0) return json({ error: "content length required" }, 411);
  if (declared > maxBytes) return json({ error: "share file too large" }, 413);

  const token = randomShareToken();
  const expiresAt = Math.floor(Date.now() / 1000) + SHARE_TTL_SECONDS;
  const key = SHARE_PREFIX + token + ".zip";
  const rawName = String(request.headers.get("x-zeitfrei-pack-name") || "");
  let packName = "Minecraft 模組翻譯資源包";
  try {
    const decoded = decodeURIComponent(rawName);
    const cleaned = decoded.replace(/[\\/\u0000-\u001f\u007f]/g, "").trim().slice(0, 120);
    if (cleaned) packName = cleaned;
  } catch (_) {}
  const object = await env.SHARES.put(key, request.body, {
    httpMetadata: { contentType: "application/zip", cacheControl: "no-store" },
    customMetadata: {
      expiresAt: String(expiresAt),
      uploader: access.userId,
      service: "packlocal-share",
      name: packName,
    },
  });
  if (!object) return json({ error: "share upload failed" }, 500);
  const base = String(env.SHARE_PUBLIC_URL || new URL(request.url).origin).replace(/\/+$/, "");
  return json({ url: `${base}/s/${token}`, expiresAt });
}

async function shareDownload(url, env, headOnly) {
  if (!env.SHARES) return json({ error: "share storage not configured" }, 503);
  const token = decodeURIComponent(url.pathname.slice("/s/".length).replace(/\/download$/, ""));
  if (!/^[A-Za-z0-9_-]{32,128}$/.test(token)) return json({ error: "bad share token" }, 400);
  const object = await env.SHARES.get(SHARE_PREFIX + token + ".zip");
  if (!object) return json({ error: "share not found or expired" }, 404);
  const expiresAt = Number(object.customMetadata?.expiresAt || 0);
  if (!expiresAt || expiresAt <= Math.floor(Date.now() / 1000)) {
    await env.SHARES.delete(SHARE_PREFIX + token + ".zip");
    return json({ error: "share expired" }, 404);
  }
  const downloadRequested = headOnly || url.pathname.endsWith("/download") || url.searchParams.get("download") === "1";
  if (!downloadRequested) {
    return renderShareLanding(url, env, token, object, expiresAt);
  }
  const headers = new Headers();
  object.writeHttpMetadata(headers);
  headers.set("content-type", "application/zip");
  headers.set("content-length", String(object.size));
  headers.set("cache-control", "no-store, max-age=0");
  headers.set("content-disposition", `attachment; filename*=UTF-8''packlocal-${token}.zip`);
  return new Response(headOnly ? null : object.body, { headers });
}

function escapeHtml(value) {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function renderShareLanding(url, env, token, object, expiresAt) {
  const name = String(object.customMetadata?.name || "Minecraft 模組翻譯資源包");
  const title = name + "｜模組包翻譯分享";
  const base = String(env.SHARE_PUBLIC_URL || url.origin).replace(/\/+$/, "");
  const canonical = base + "/s/" + token;
  const downloadUrl = canonical + "?download=1";
  const expires = new Date(expiresAt * 1000).toLocaleString("zh-TW", { dateStyle: "medium", timeStyle: "short" });
  const html = [
    "<!doctype html><html lang=\"zh-Hant\"><head><meta charset=\"utf-8\">",
    "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    "<meta name=\"robots\" content=\"noindex,nofollow\">",
    "<meta property=\"og:type\" content=\"website\">",
    "<meta property=\"og:title\" content=\"" + escapeHtml(title) + "\">",
    "<meta property=\"og:description\" content=\"24 小時有效的 Minecraft 繁體中文翻譯資源包分享。\">",
    "<meta property=\"og:url\" content=\"" + escapeHtml(canonical) + "\">",
    "<meta name=\"twitter:card\" content=\"summary\">",
    "<title>" + escapeHtml(title) + "</title>",
    "<style>body{margin:0;background:#101214;color:#e8e5df;font:16px/1.6 system-ui,sans-serif}main{max-width:680px;margin:8vh auto;padding:28px}article{border:1px solid #3d4147;border-radius:14px;background:#191c20;padding:28px;box-shadow:0 20px 70px #0006}p{color:#b8b8b2}.tag{color:#e29a62;font-size:12px;letter-spacing:.14em;text-transform:uppercase}a.button{display:inline-block;background:#dd8951;color:#17110d;text-decoration:none;font-weight:700;padding:11px 18px;border-radius:8px;margin:12px 0}footer{margin-top:28px;font-size:13px;color:#8e918d}footer a{color:#d8a47d;margin-right:14px}</style>",
    "</head><body><main><article><div class=\"tag\">ZEITFREI · PACKLOCAL SHARE</div>",
    "<h1>" + escapeHtml(name) + "</h1>",
    "<p>這是可直接覆蓋到 Minecraft 實例的翻譯資源包。連結只保留 24 小時，下載後請解壓到對應的遊戲資料夾。</p>",
    "<p>有效期限：<strong>" + escapeHtml(expires) + "</strong></p>",
    "<a class=\"button\" href=\"" + escapeHtml(downloadUrl) + "\">下載翻譯檔</a>",
    "<footer>需要更多遊戲與工具？<a href=\"https://cloud.zeitfrei.uk/\">cloud.zeitfrei.uk 遊戲下載</a><a href=\"https://cloud.zeitfrei.uk/zeitfreitool\">ZeitFrei 工具箱</a></footer>",
    "</article></main></body></html>",
  ].join("");
  return new Response(html, {
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store, max-age=0",
      "content-security-policy": "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors *; base-uri 'none'",
      "x-content-type-options": "nosniff",
      "referrer-policy": "no-referrer",
    },
  });
}

function randomShareToken() {
  const bytes = new Uint8Array(24);
  crypto.getRandomValues(bytes);
  return btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=/g, "");
}

async function cleanupShares(env) {
  if (!env.SHARES) return;
  const listing = await env.SHARES.list({ prefix: SHARE_PREFIX, limit: 1000 });
  const now = Math.floor(Date.now() / 1000);
  const expired = await Promise.all(
    listing.objects.map(async (object) => {
      const head = await env.SHARES.head(object.key);
      return head && Number(head.customMetadata?.expiresAt || 0) <= now ? object.key : null;
    })
  );
  await Promise.all(expired.filter(Boolean).map((key) => env.SHARES.delete(key)));
}

// ───────────────────────── AI 代理 ─────────────────────────

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

  // 每日總量保護：超過就回 429，客戶端顯示贊助提示。
  const budget = parseInt(env.DAILY_TOKEN_BUDGET || "0", 10);
  const userBudget = parseInt(env.PER_USER_DAILY_TOKEN_BUDGET || "0", 10);
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
  if (userBudget > 0 && env.USAGE) {
    const userDayKey = `usage:user:${utcDay()}:${access.userId}`;
    const spent = parseInt((await env.USAGE.get(userDayKey)) || "0", 10);
    if (spent >= userBudget) {
      return json(
        { error: { message: "personal daily translation budget reached", type: "insufficient_quota" } },
        429
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
  if (resp.ok && env.USAGE && (budget > 0 || userBudget > 0)) {
    try {
      const used = estimateTokens(text, forward);
      if (budget > 0) {
        const dayKey = "usage:" + utcDay();
        const spent = parseInt((await env.USAGE.get(dayKey)) || "0", 10);
        // 隔日自動歸零：TTL 略長於一天。
        await env.USAGE.put(dayKey, String(spent + used), { expirationTtl: 172800 });
      }
      if (userBudget > 0) {
        const userDayKey = `usage:user:${utcDay()}:${access.userId}`;
        const userSpent = parseInt((await env.USAGE.get(userDayKey)) || "0", 10);
        await env.USAGE.put(userDayKey, String(userSpent + used), { expirationTtl: 172800 });
      }
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

async function authorizeManagedAi(request, env) {
  const identity = await authorizeManagedIdentity(request, env);
  if (!identity.ok) return identity;

  // 代管 AI 與分享檔共用 Discord 會員及 Turnstile 閘門；舊版缺少新標頭時會先被拒絕。
  // 強制模式下，設定不完整也必須拒絕，不能退化成只檢查 Discord。
  const turnstileEnforced = String(env.TURNSTILE_ENFORCED || "") === "1";
  if (turnstileEnforced && !turnstileConfigured(env)) {
    return {
      ok: false,
      response: json(
        {
          error: {
            message: turnstileUnavailableMessage(env),
            type: "turnstile_unavailable",
          },
        },
        503
      ),
    };
  }
  if (turnstileEnforced) {
    const proof = String(request.headers.get("x-zeitfrei-turnstile") || "").trim();
    const checked = await verifyTurnstileAccess(proof, env, identity.userId);
    if (!checked.ok) {
      return {
        ok: false,
        response: json(
          {
            error: {
              message: "Cloudflare Turnstile verification required",
              type: checked.type,
            },
          },
          checked.type === "turnstile_unavailable" ? 503 : 428
        ),
      };
    }
  }
  return identity;
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

  try {
    const response = await fetch(`${authBase}/api/member-tier/${encodeURIComponent(userId)}`, {
      signal: AbortSignal.timeout(8000),
    });
    if (!response.ok) throw new Error("membership check failed");
    const membership = await response.json();
    if (!membership || membership.inGuild !== true) {
      return {
        ok: false,
        response: json({ error: { message: "official discord membership required", type: "guild_required" } }, 403),
      };
    }
  } catch (_) {
    return {
      ok: false,
      response: json({ error: { message: "membership verification unavailable", type: "auth_unavailable" } }, 503),
    };
  }

  return { ok: true, userId };
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

function corsHeaders() {
  return {
    "access-control-allow-origin": "*",
    "access-control-allow-methods": "GET, POST, OPTIONS",
    "access-control-allow-headers": "content-type, authorization, x-zeitfrei-ai-protocol, x-zeitfrei-client-version, x-zeitfrei-session, x-zeitfrei-turnstile",
  };
}

function json(obj, status = 200) {
  return new Response(JSON.stringify(obj), {
    status,
    // no-store：版本檢查等 API 一定要拿到最新值，不能被邊緣或客戶端快取住舊版本資訊。
    headers: { ...JSON_HEADERS, "cache-control": "no-store", ...corsHeaders() },
  });
}
