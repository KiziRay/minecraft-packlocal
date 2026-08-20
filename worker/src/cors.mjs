/** CORS allowlist：Tauri WebView 常無 Origin；瀏覽器僅反射已知來源。 */
const ALLOWED_ORIGINS = new Set([
  "https://modpack-i18n.jolin34563.workers.dev",
  "https://tauri.localhost",
  "http://tauri.localhost",
  "tauri://localhost",
]);

export function corsHeaders(request) {
  const headers = {
    "access-control-allow-methods": "GET, POST, PUT, OPTIONS",
    "access-control-allow-headers":
      "content-type, authorization, x-zeitfrei-ai-protocol, x-zeitfrei-client-version, x-zeitfrei-session, x-zeitfrei-turnstile, x-zeitfrei-report-category, x-zeitfrei-pack-unrelated, x-zeitfrei-pack-name",
  };
  const origin = request?.headers?.get("Origin");
  // 桌面 WebView / null Origin：不反射 *，但也不阻 CORS 預檢（無 ACAO 時同源請求仍可用）
  if (!origin || origin === "null") {
    return headers;
  }
  if (ALLOWED_ORIGINS.has(origin)) {
    headers["access-control-allow-origin"] = origin;
    headers.vary = "Origin";
  }
  return headers;
}
