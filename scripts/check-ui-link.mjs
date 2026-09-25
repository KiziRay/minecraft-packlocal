import { readdir } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import path from "node:path";

const SRC = path.resolve(process.cwd(), "src");

async function listJs(dir) {
  const out = [];
  const entries = await readdir(dir, { withFileTypes: true });
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "worker") continue;
      out.push(...(await listJs(full)));
    } else if (entry.isFile() && entry.name.endsWith(".js")) {
      out.push(full);
    }
  }
  return out;
}

async function listHtml(dir) {
  const out = [];
  const entries = await readdir(dir, { withFileTypes: true });
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "worker") continue;
      out.push(...(await listHtml(full)));
    } else if (entry.isFile() && entry.name.endsWith(".html")) {
      out.push(full);
    }
  }
  return out;
}

function classify(err) {
  const name = err && err.name ? String(err.name) : "Error";
  const msg = err && err.message ? String(err.message) : String(err);
  if (name === "SyntaxError") return { ok: false, name, msg };
  if (name === "ReferenceError") return { ok: true, name, msg };
  return { ok: false, name, msg };
}

const files = await listJs(SRC);
let failed = 0;
for (const file of files) {
  const rel = path.relative(process.cwd(), file).replaceAll("\\", "/");
  try {
    await import(pathToFileURL(file).href);
    console.log(`ok   ${rel}`);
  } catch (err) {
    const { ok, name, msg } = classify(err);
    if (ok) {
      console.log(`ok   ${rel}  (${name}: ${msg})`);
    } else {
      failed += 1;
      console.log(`FAIL ${rel}  ${name}: ${msg}`);
    }
  }
}

if (failed) {
  console.error(`check:ui failed: ${failed} SyntaxError/unexpected`);
  process.exit(1);
}
console.log(`check:ui passed: ${files.length} modules`);

// ---------------------------------------------------------------------------
// 接線比對
//
// 模組能 import 成功不代表接線是對的。實際踩過兩個「靜默失效」：
//   - 前端一直呼叫 suggest_output_dir，後端從沒註冊過 → 那個設定選項按了等於沒按
//   - 前端寫 $("font-rail-msg")，HTML 裡是 font-prog-msg → 按鈕沒有任何回饋
// 兩者都被 .catch() 或 if (el) 吞掉，讀碼很難看見，但機械比對三秒就抓到。
// ---------------------------------------------------------------------------
import { readFile } from "node:fs/promises";

const sources = new Map(
  await Promise.all(files.map(async (f) => [path.resolve(f), await readFile(f, "utf8")]))
);
const allJs = [...sources.values()].join("\n");
const htmlFiles = await listHtml(SRC);
const libRs = await readFile(path.resolve(process.cwd(), "src-tauri/src/lib.rs"), "utf8");

const uniq = (arr) => [...new Set(arr)].sort();
const matchAll = (text, re, group = 1) => [...text.matchAll(re)].map((m) => m[group]);

// 1) 前端 invoke 名稱 vs 後端 generate_handler! 註冊清單
const handlerBlock = libRs.slice(libRs.indexOf("generate_handler!["));
const registered = new Set(
  matchAll(handlerBlock.slice(0, handlerBlock.indexOf("])")), /^\s*([a-z_0-9]+),\s*$/gm)
);
const invoked = uniq([
  ...matchAll(allJs, /\binvoke\(\s*"([a-zA-Z_0-9]+)"/g),
  ...matchAll(allJs, /invokeFirstAvailable\(\s*\[([^\]]*)\]/g)
    .flatMap((inner) => matchAll(inner, /"([a-z_0-9]+)"/g)),
]);
// invokeFirstAvailable 是刻意的相容備援：只要清單裡有一個存在就算過。
const fallbackGroups = matchAll(allJs, /invokeFirstAvailable\(\s*\[([^\]]*)\]/g).map((inner) =>
  matchAll(inner, /"([a-z_0-9]+)"/g)
);
const coveredByFallback = new Set(
  fallbackGroups.filter((g) => g.some((n) => registered.has(n))).flat()
);
const missingCmds = invoked.filter((n) => !registered.has(n) && !coveredByFallback.has(n));

// 2) 每個 HTML 各自比對「它自己載入的 JS」取用的 DOM id。
//
// 舊版把所有 HTML 的 id 合併成一池再比對：主視窗的 JS 取了只存在於 settings.html
// 的 id 也會被當成「存在」，正好漏掉「刪掉舊設定頁後主視窗還在找那些控制項」這類
// 跨視窗的斷線。現在從每個 HTML 的 <script src> 出發，沿著 import 走完它實際會
// 載入的模組，只拿那個 HTML 自己的 id 來比。
function importsOf(file) {
  const text = sources.get(file) || "";
  const specs = [
    ...matchAll(text, /\bimport\s+[^'";]*?\bfrom\s*["']([^"']+)["']/g),
    ...matchAll(text, /\bexport\s+[^'";]*?\bfrom\s*["']([^"']+)["']/g),
    ...matchAll(text, /\bimport\s*["']([^"']+)["']/g),
    ...matchAll(text, /\bimport\(\s*["']([^"']+)["']\s*\)/g),
  ];
  return specs
    .filter((spec) => spec.startsWith("."))
    .map((spec) => path.resolve(path.dirname(file), spec))
    .filter((resolved) => sources.has(resolved));
}

function closureFrom(entries) {
  const seen = new Set();
  const stack = [...entries];
  while (stack.length) {
    const file = stack.pop();
    if (seen.has(file) || !sources.has(file)) continue;
    seen.add(file);
    stack.push(...importsOf(file));
  }
  return [...seen];
}

const missingIdsByHtml = [];
let usedIdCount = 0;
for (const htmlFile of htmlFiles) {
  const htmlText = await readFile(htmlFile, "utf8");
  const entries = matchAll(htmlText, /<script\b[^>]*\bsrc="([^"]+\.js)"/g).map((src) =>
    path.resolve(path.dirname(htmlFile), src)
  );
  const moduleFiles = closureFrom(entries);
  if (!moduleFiles.length) continue;
  const js = moduleFiles.map((f) => sources.get(f)).join("\n");
  const htmlIds = new Set(matchAll(htmlText, /\bid="([a-zA-Z0-9_-]+)"/g));
  const dynamicIds = new Set([
    ...matchAll(js, /\.id\s*=\s*"([a-zA-Z0-9_-]+)"/g),
    ...matchAll(js, /\bid="([a-zA-Z0-9_-]+)"/g), // 由 innerHTML 產生的節點
  ]);
  // 取用 id 的四種寫法都要比對：$("x")／$('x')、getElementById("x"|'x')、querySelector("#x")
  const usedIds = uniq([
    ...matchAll(js, /\$\(\s*["']([a-zA-Z0-9_-]+)["']\s*\)/g),
    ...matchAll(js, /getElementById\(\s*["']([a-zA-Z0-9_-]+)["']\s*\)/g),
    ...matchAll(js, /querySelector(?:All)?\(\s*["']#([a-zA-Z0-9_-]+)["']\s*\)/g),
  ]);
  usedIdCount += usedIds.length;
  const missing = usedIds.filter((id) => !htmlIds.has(id) && !dynamicIds.has(id));
  if (missing.length) {
    const rel = path.relative(process.cwd(), htmlFile).replaceAll("\\", "/");
    missingIdsByHtml.push({ rel, missing });
  }
}

let linkFailed = false;
if (missingCmds.length) {
  linkFailed = true;
  console.error(`\nFAIL 前端呼叫了未註冊的指令（會靜默失敗）：\n  ${missingCmds.join("\n  ")}`);
}
for (const { rel, missing } of missingIdsByHtml) {
  linkFailed = true;
  console.error(`\nFAIL ${rel} 載入的 JS 取用了這個頁面沒有的 DOM id（接線斷掉）：\n  ${missing.join("\n  ")}`);
}
if (linkFailed) process.exit(1);
console.log(
  `check:ui link ok: ${invoked.length} invoke 名稱對上 ${registered.size} 個註冊指令；` +
    `${htmlFiles.length} 個頁面各自比對，共 ${usedIdCount} 個 DOM id 全部存在`
);
