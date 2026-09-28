/**
 * 從 HTML 原始碼取出「畫面上看得到的字」：文字節點＋title／placeholder／aria-label／alt。
 * 給詞表掃描測試用（B5a-2）；不跑瀏覽器，所以 hidden 的區塊也算（寧可多掃）。
 */

/** 取出從 startNeedle 開始、到同名標籤配對結束為止的原始碼（支援巢狀同名標籤）。 */
export function sliceElement(html, startNeedle) {
  const start = html.indexOf(startNeedle);
  if (start < 0) return "";
  const tag = /^<([a-zA-Z0-9]+)/.exec(html.slice(start))?.[1];
  if (!tag) return "";
  const pattern = new RegExp(`<${tag}\\b|</${tag}>`, "g");
  pattern.lastIndex = start;
  let depth = 0;
  let match;
  while ((match = pattern.exec(html))) {
    if (match[0].startsWith("</")) {
      depth -= 1;
      if (depth === 0) return html.slice(start, match.index + match[0].length);
    } else {
      depth += 1;
    }
  }
  return html.slice(start);
}

/** 把 innerNeedle 開頭的整個元素從 html 拿掉（用來排除別批負責的區塊）。 */
export function withoutElement(html, innerNeedle) {
  const part = sliceElement(html, innerNeedle);
  return part ? html.replace(part, "") : html;
}

const ENTITIES = { "&amp;": "&", "&lt;": "<", "&gt;": ">", "&quot;": '"', "&#39;": "'", "&nbsp;": " " };

export function visibleText(html) {
  const source = String(html || "")
    .replace(/<!--[\s\S]*?-->/g, " ")
    .replace(/<script[\s\S]*?<\/script>/gi, " ")
    .replace(/<style[\s\S]*?<\/style>/gi, " ");
  const attrs = [...source.matchAll(/\s(?:title|placeholder|aria-label|alt)="([^"]*)"/g)].map((m) => m[1]);
  const text = source.replace(/<[^>]+>/g, "\n");
  return [text, ...attrs]
    .join("\n")
    .replace(/&[a-z#0-9]+;/gi, (e) => ENTITIES[e] ?? " ")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .join("\n");
}
