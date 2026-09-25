// 本地模型檔：獨立 R2（LOCAL_LLM）。禁止寫入 SHARES／DOWNLOADS。
import { corsHeaders } from "./cors.mjs";

const JSON_HEADERS = { "content-type": "application/json; charset=utf-8" };
const SAFE_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,180}$/;
export const LOCAL_LLM_MANIFEST_KEY = "manifest.json";

export async function localLlmBound(env) {
  try {
    if (!env?.LOCAL_LLM) return false;
    const head = await env.LOCAL_LLM.head(LOCAL_LLM_MANIFEST_KEY);
    return !!head;
  } catch (_) {
    return false;
  }
}

/**
 * 把 corsHeaders() 的純物件套進 Headers 實例。
 *
 * 舊版寫 `Object.assign(headers, corsHeaders(request))`——Object.assign 只複製自有可列舉
 * 屬性，對 Headers 完全無效，CORS 標頭是靜默消失的。桌面端走 reqwest 不做 CORS 所以
 * 看不出來，任何瀏覽器情境都會踩到。
 */
export function applyCors(headers, request) {
  for (const [key, value] of Object.entries(corsHeaders(request))) {
    headers.set(key, value);
  }
  return headers;
}

function json(obj, status = 200, request) {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { ...JSON_HEADERS, "cache-control": "no-store", ...corsHeaders(request) },
  });
}

export function isSafeObjectName(name) {
  const n = String(name || "").trim();
  if (!n || n.includes("/") || n.includes("\\") || n.includes("..")) return false;
  return SAFE_NAME.test(n);
}

export function allowlistFromManifest(manifest) {
  const names = new Set([LOCAL_LLM_MANIFEST_KEY]);
  if (!manifest || typeof manifest !== "object") return names;
  const gguf = manifest.gguf && typeof manifest.gguf.name === "string" ? manifest.gguf.name : "";
  if (isSafeObjectName(gguf)) names.add(gguf);
  const runtimes = manifest.runtimes && typeof manifest.runtimes === "object" ? manifest.runtimes : {};
  for (const key of ["cuda", "vulkan", "cpu"]) {
    const name = runtimes[key] && typeof runtimes[key].name === "string" ? runtimes[key].name : "";
    if (isSafeObjectName(name)) names.add(name);
  }
  return names;
}

export function parseRangeHeader(header, size) {
  const raw = String(header || "").trim();
  if (!raw) return null;
  const m = /^bytes=(\d*)-(\d*)$/i.exec(raw);
  if (!m) return { error: "invalid range" };
  const total = Number(size) || 0;
  if (total <= 0) return { error: "empty" };
  let start;
  let end;
  if (m[1] === "" && m[2] !== "") {
    const suffix = parseInt(m[2], 10);
    if (!Number.isFinite(suffix) || suffix <= 0) return { error: "invalid range" };
    start = Math.max(0, total - suffix);
    end = total - 1;
  } else if (m[1] !== "") {
    start = parseInt(m[1], 10);
    end = m[2] === "" ? total - 1 : parseInt(m[2], 10);
  } else {
    return { error: "invalid range" };
  }
  if (!Number.isFinite(start) || !Number.isFinite(end) || start < 0 || end < start || start >= total) {
    return { error: "unsatisfiable" };
  }
  end = Math.min(end, total - 1);
  return { start, end, length: end - start + 1 };
}

async function readManifestObject(env) {
  if (!env?.LOCAL_LLM) return null;
  const obj = await env.LOCAL_LLM.get(LOCAL_LLM_MANIFEST_KEY);
  if (!obj) return null;
  const text = await obj.text();
  try {
    return JSON.parse(text);
  } catch (_) {
    return null;
  }
}

export async function localLlmManifest(request, env, authorize) {
  const access = await authorize(request, env);
  if (!access.ok) return access.response;
  if (!env?.LOCAL_LLM) return json({ error: "local llm storage not configured" }, 503, request);
  const obj = await env.LOCAL_LLM.get(LOCAL_LLM_MANIFEST_KEY);
  if (!obj) return json({ error: "manifest not found" }, 404, request);
  const headers = new Headers({ "content-type": "application/json; charset=utf-8", "cache-control": "no-store" });
  applyCors(headers, request);
  return new Response(obj.body, { status: 200, headers });
}

export async function localLlmFile(request, env, url, authorize) {
  const access = await authorize(request, env);
  if (!access.ok) return access.response;
  if (!env?.LOCAL_LLM) return json({ error: "local llm storage not configured" }, 503, request);
  const name = decodeURIComponent(String(url.pathname || "").slice("/api/local-llm/file/".length));
  if (!isSafeObjectName(name)) return json({ error: "bad object name" }, 400, request);
  const manifest = await readManifestObject(env);
  const allow = allowlistFromManifest(manifest);
  if (!allow.has(name)) return json({ error: "not allowlisted" }, 403, request);
  const head = await env.LOCAL_LLM.head(name);
  const size = Number(head?.size || 0);
  if (!head || size <= 0) return json({ error: "not found" }, 404, request);
  const range = parseRangeHeader(request.headers.get("Range") || request.headers.get("range"), size);
  if (range && range.error) {
    return json({ error: range.error }, range.error === "unsatisfiable" ? 416 : 400, request);
  }
  const getOpts = range ? { range: { offset: range.start, length: range.length } } : undefined;
  const obj = await env.LOCAL_LLM.get(name, getOpts);
  if (!obj) return json({ error: "not found" }, 404, request);
  const headers = new Headers();
  if (typeof obj.writeHttpMetadata === "function") obj.writeHttpMetadata(headers);
  if (obj.httpEtag) headers.set("etag", obj.httpEtag);
  headers.set("accept-ranges", "bytes");
  headers.set("content-disposition", `attachment; filename*=UTF-8''${encodeURIComponent(name)}`);
  if (!headers.has("content-type")) headers.set("content-type", "application/octet-stream");
  applyCors(headers, request);
  if (range) {
    headers.set("content-range", `bytes ${range.start}-${range.end}/${size}`);
    headers.set("content-length", String(range.length));
    return new Response(request.method === "HEAD" ? null : obj.body, { status: 206, headers });
  }
  headers.set("content-length", String(size || obj.size || ""));
  return new Response(request.method === "HEAD" ? null : obj.body, { status: 200, headers });
}
