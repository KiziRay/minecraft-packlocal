/** SSRF 加固：出站 fetch 前檢查 hostname（字面 IP／保留名）。DNS rebinding 仍需上游 allowlist。 */

export function isPrivateOrReservedHost(hostname) {
  const h = String(hostname || "")
    .replace(/^\[/, "")
    .replace(/\]$/, "")
    .toLowerCase();
  if (!h || h === "localhost") return true;
  if (h.endsWith(".local") || h.endsWith(".internal") || h.endsWith(".localhost")) return true;

  const v4 = h.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  if (v4) {
    const parts = v4.slice(1, 5).map((n) => Number(n));
    if (parts.some((n) => n > 255)) return true;
    const [a, b] = parts;
    if (a === 0 || a === 127 || a === 10) return true;
    if (a === 169 && b === 254) return true;
    if (a === 172 && b >= 16 && b <= 31) return true;
    if (a === 192 && b === 168) return true;
    if (a === 100 && b >= 64 && b <= 127) return true;
    return false;
  }

  if (h === "::1" || h.startsWith("fe80:") || h.startsWith("fc") || h.startsWith("fd")) return true;
  if (h.includes("::ffff:127.") || h.includes("::ffff:0:") || h.includes("::ffff:10.")) return true;
  return false;
}

export function isSafeOutboundUrl(urlStr) {
  try {
    const u = new URL(String(urlStr || ""));
    if (u.protocol !== "https:" && u.protocol !== "http:") return false;
    if (u.username || u.password) return false;
    return !isPrivateOrReservedHost(u.hostname);
  } catch (_) {
    return false;
  }
}
