// B5a-2 #3 無障礙（規格 §6）：可見焦點、對比 4.5:1、最小字級 12px、不截字、分頁方向鍵、路徑可複製、背景圖、字型本地。
// 能用原始碼驗的部分在這裡驗；實際縮放 110–170% 的畫面要使用者目視確認。
import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";

import { nextTabIndex, rovingTabindex } from "./tab-keys.js";

const here = dirname(fileURLToPath(import.meta.url));
const src = resolve(here, "..");
const read = (rel) => readFileSync(join(src, rel), "utf8").replace(/\r\n/g, "\n");
const styleFiles = [...readdirSync(join(src, "styles")).filter((f) => f.endsWith(".css")).map((f) => `styles/${f}`), "settings-window.css"];
const css = Object.fromEntries(styleFiles.map((f) => [f, read(f)]));

function varsIn(block) {
  return Object.fromEntries([...block.matchAll(/(--[a-z0-9-]+):\s*(#[0-9a-fA-F]{6})\b/g)].map((m) => [m[1], m[2]]));
}
function blockAfter(text, selector) {
  const at = text.indexOf(selector);
  assert.ok(at >= 0, `找不到 ${selector}`);
  return text.slice(at, text.indexOf("}", at));
}
function luminance(hex) {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
function contrast(a, b) {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

test("可見焦點：沒有任何 outline:none；全域 :focus-visible 2px", () => {
  for (const [file, text] of Object.entries(css)) {
    assert.ok(!/outline:\s*(none|0)\s*[;}]/.test(text), `${file} 還有 outline:none`);
  }
  const global = blockAfter(css["styles/base.css"], ":focus-visible {");
  assert.match(global, /outline:\s*2px solid/);
});

test("對比 ≥4.5:1：淡字、次要字、主要按鈕（深色與淺色）", () => {
  const base = css["styles/base.css"];
  const dark = varsIn(blockAfter(base, ":root {"));
  const light = { ...dark, ...varsIn(blockAfter(base, '[data-theme="light"] {')) };
  for (const [name, theme] of [["深色", dark], ["淺色", light]]) {
    for (const fg of ["--faint", "--muted", "--ink"]) {
      for (const bg of ["--paper", "--canvas", "--panel"]) {
        const ratio = contrast(theme[fg], theme[bg]);
        assert.ok(ratio >= 4.5, `${name} ${fg} 在 ${bg} 上只有 ${ratio.toFixed(2)}`);
      }
    }
    const ratio = contrast(theme["--primary-ink"], theme["--primary-bg"]);
    assert.ok(ratio >= 4.5, `${name} 主要按鈕只有 ${ratio.toFixed(2)}`);
  }
  const primary = blockAfter(css["styles/translate.css"], ".primary-button {");
  assert.match(primary, /background:\s*var\(--primary-bg\)/);
  assert.match(primary, /color:\s*var\(--primary-ink\)/);
  // 設定視窗
  const sw = css["settings-window.css"];
  const swDark = varsIn(blockAfter(sw, ":root {"));
  const swLight = { ...swDark, ...varsIn(blockAfter(sw, ':root[data-theme="light"] {')) };
  for (const [name, theme] of [["設定深色", swDark], ["設定淺色", swLight]]) {
    for (const [fg, bg] of [["--muted", "--bg"], ["--muted", "--panel"], ["--muted", "--panel-soft"], ["--accent-ink", "--accent"], ["--warn", "--panel"], ["--ok", "--bg"]]) {
      const ratio = contrast(theme[fg], theme[bg]);
      assert.ok(ratio >= 4.5, `${name} ${fg} 在 ${bg} 上只有 ${ratio.toFixed(2)}`);
    }
  }
});

test("最小字級 12px；取消 96px 截字與狀態文字的省略號", () => {
  for (const [file, text] of Object.entries(css)) {
    for (const m of text.matchAll(/font-size:\s*([0-9.]+)px/g)) {
      assert.ok(Number(m[1]) >= 12, `${file} 有 ${m[0]}`);
    }
    for (const m of text.matchAll(/--type-[a-z]+:\s*([0-9.]+)px/g)) {
      assert.ok(Number(m[1]) >= 12, `${file} 有 ${m[0]}`);
    }
    assert.ok(!/max-width:\s*96px/.test(text), `${file} 還有 96px 截字`);
  }
  assert.ok(!/#discord-auth-title\s*\{[^}]*text-overflow:\s*ellipsis/.test(css["styles/translate.css"]), "登入狀態字放大後不可被截斷");
});

test("縮放 80–170%：浮層與選單的視窗高度／寬度都除以縮放倍率（不被切掉、不截字）", () => {
  for (const [file, text] of Object.entries(css)) {
    if (file === "settings-window.css") continue; // 設定視窗的縮放由自己的 zoom 處理，沒有 --ui-zoom
    for (const m of text.matchAll(/calc\(100(?:vh|vw|dvh)[^;]*/g)) {
      assert.match(m[0], /var\(--ui-zoom/, `${file}：${m[0]}`);
    }
  }
});

test("分頁方向鍵：左右／上下循環、Home／End；其他鍵不動", () => {
  assert.equal(nextTabIndex("ArrowRight", 0, 4), 1);
  assert.equal(nextTabIndex("ArrowRight", 3, 4), 0);
  assert.equal(nextTabIndex("ArrowLeft", 0, 4), 3);
  assert.equal(nextTabIndex("ArrowDown", 1, 2), 0);
  assert.equal(nextTabIndex("ArrowUp", 1, 2), 0);
  assert.equal(nextTabIndex("Home", 2, 4), 0);
  assert.equal(nextTabIndex("End", 0, 4), 3);
  assert.equal(nextTabIndex("Enter", 0, 4), -1);
  assert.equal(nextTabIndex("ArrowRight", 0, 0), -1);
  assert.deepEqual(rovingTabindex([false, true, false]), ["-1", "0", "-1"]);
  assert.deepEqual(rovingTabindex([false, false]), ["0", "-1"], "沒有選中時第一個可聚焦");
});

test("分頁方向鍵接上主視窗與設定視窗；分頁有 aria-controls", () => {
  assert.ok(read("app.js").includes('wireTabKeys($("workbench-tabs")'));
  assert.ok(read("settings-window.js").includes("wireTabKeys("));
  const index = read("index.html");
  assert.match(index, /id="tab-translate"[^>]*aria-controls="page-translate"/);
  assert.match(index, /id="tab-font"[^>]*aria-controls="page-font"/);
  const settings = read("settings.html");
  for (const pane of ["general", "translate", "data", "help"]) {
    assert.match(settings, new RegExp(`data-pane="${pane}"`));
    assert.match(settings, new RegExp(`aria-controls="pane-${pane}"`));
  }
});

test("設定視窗的路徑都能一鍵複製（工具資料、本地模型、自己指定的資料夾）", () => {
  const settings = read("settings.html");
  for (const id of ["data-root", "local-model-dir", "output-custom-root"]) {
    assert.ok(settings.includes(`data-copy="${id}"`), `缺 ${id} 的複製路徑`);
  }
  assert.ok(read("settings/data-pane.js").includes("navigator.clipboard.writeText("));
});

test("背景圖跟著深色／淺色換，且檔案真的存在（CSS 相對路徑以 styles/ 為基準）", () => {
  const base = css["styles/base.css"];
  const urls = [...base.matchAll(/url\("([^"]+\.png)"\)/g)].map((m) => m[1]);
  assert.ok(urls.length >= 2);
  for (const url of urls) assert.ok(existsSync(join(src, "styles", url)), `背景圖不存在：${url}`);
  assert.match(blockAfter(base, ".app-atmosphere::after {"), /workspace-background-dark\.png/);
  assert.match(base, /\[data-theme="light"\] \.app-atmosphere::after \{[^}]*workspace-background-light\.png/);
});

test("字型（保守方案 A）：只用 Windows 內建系統字型，不連外網、不內建字型檔；CSP 不含外部字型網域", () => {
  const index = read("index.html");
  assert.ok(!index.includes("fonts.googleapis.com") && !index.includes("fonts.gstatic.com"));
  const conf = readFileSync(join(src, "../src-tauri/tauri.conf.json"), "utf8");
  assert.ok(!conf.includes("fonts.googleapis.com") && !conf.includes("fonts.gstatic.com"), "CSP 要同步拿掉外網字型");
  for (const [file, text] of Object.entries(css)) {
    assert.ok(!/@font-face/.test(text), `${file} 不再內建字型`);
    assert.ok(!/"(Outfit|IBM Plex Mono|MCPL Noto Sans TC)"/.test(text), `${file} 還在用沒有的字型`);
  }
  assert.ok(!existsSync(join(src, "assets/fonts/NotoSansTC-VF.ttf")), "備案才內建 Noto Sans TC");
  const base = css["styles/base.css"];
  assert.match(base, /--font-body:[^;]*"Microsoft JhengHei UI", "Microsoft JhengHei"/);
  assert.match(base, /--font-ui:\s*"Segoe UI Variable/);
  assert.match(base, /--font-mono:\s*"Cascadia Mono", Consolas/);
  assert.ok(!readFileSync(join(src, "../NOTICE.md"), "utf8").includes("NotoSansTC"), "NOTICE 同步");
});

test("審查 F8：設定視窗焦點框實色 var(--accent)，全域 :focus-visible 含 select／checkbox", () => {
  const sw = css["settings-window.css"];
  assert.match(sw, /(^|\n):focus-visible \{[^}]*outline:\s*2px solid var\(--accent\)/);
  for (const m of sw.matchAll(/outline:[^;]*/g)) assert.ok(!m[0].includes("color-mix"), `焦點框要實色：${m[0]}`);
});

test("審查 F9：主視窗分頁內容是 tabpanel，並指回自己的分頁", () => {
  const index = read("index.html");
  assert.match(index, /<div id="page-translate"[^>]*role="tabpanel"[^>]*aria-labelledby="tab-translate"/);
  assert.match(index, /<div id="page-font"[^>]*role="tabpanel"[^>]*aria-labelledby="tab-font"/);
});
