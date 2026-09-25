/**
 * 介面縮放：與 ZeitFrei 相同，只用 html { zoom }。
 * 永不呼叫 WebView setZoom。
 */
import {
  clampScalePercent,
  computeAutoScalePercent,
  parseCssZoom,
  parseStoredAuto,
  parseStoredScale,
  visualToCssPx,
} from "./ui-scale-logic.js";

const SCALE_KEY = "mcpl-webview-scale";
const AUTOSCALE_KEY = "mcpl-webview-autoscale";
const LEGACY_SCALE_KEYS = ["modpack-i18n-ui-scale", "modpack-i18n-ui-autoscale"];

let currentPercent = 100;
let autoScale = false;
let applying = false;
let lastAutoScreenW = 0;

function purgeLegacyStorage() {
  try {
    for (const k of LEGACY_SCALE_KEYS) localStorage.removeItem(k);
  } catch (_) {
    /* ignore */
  }
}

export function cssUiZoom() {
  if (typeof document === "undefined") return 1;
  try {
    return parseCssZoom(getComputedStyle(document.documentElement).zoom);
  } catch (_) {
    return 1;
  }
}

function screenWidth() {
  try {
    return (window.screen && (screen.availWidth || screen.width)) || 1920;
  } catch (_) {
    return 1920;
  }
}

function readStoredScale() {
  try {
    return parseStoredScale(localStorage.getItem(SCALE_KEY) || "100");
  } catch (_) {
    return 100;
  }
}

function readStoredAuto() {
  try {
    return parseStoredAuto(localStorage.getItem(AUTOSCALE_KEY));
  } catch (_) {
    return false;
  }
}

let persistHook = null;
let lastPersisted = "";

/**
 * 縮放值寫進 localStorage 之後通知外部（主視窗用來寫進設定檔並同步給設定視窗）。
 * 值沒變就不通知，避免自動縮放在視窗調整大小時連續觸發寫檔。
 */
export function onScalePersisted(fn) {
  persistHook = typeof fn === "function" ? fn : null;
}

function persistScale(percent, auto) {
  try {
    localStorage.setItem(SCALE_KEY, String(percent));
    localStorage.setItem(AUTOSCALE_KEY, auto ? "1" : "0");
  } catch (_) {
    /* ignore */
  }
  const signature = `${percent}:${auto ? 1 : 0}`;
  if (signature === lastPersisted) return;
  lastPersisted = signature;
  try {
    if (persistHook) persistHook(percent, !!auto);
  } catch (_) {
    /* 同步失敗不影響縮放本身 */
  }
}

/** 縮放控制項在獨立設定視窗；主視窗沒有控制項要同步。保留呼叫點方便日後擴充。 */
function updateScaleControls() {}

function applyCssZoom(percent) {
  const n = clampScalePercent(percent);
  const factor = n / 100;
  try {
    document.documentElement.style.zoom = String(factor);
    document.documentElement.style.setProperty("--ui-zoom", String(factor));
  } catch (_) {
    /* ignore */
  }
  return n;
}

function flash(msg) {
  if (typeof window.flashAppSettingsSaved === "function") {
    window.flashAppSettingsSaved(msg);
  }
}

function ensureZoomHint() {
  let el = document.getElementById("zoom-hint");
  if (el) return el;
  el = document.createElement("div");
  el.id = "zoom-hint";
  el.className = "zoom-hint";
  el.setAttribute("role", "status");
  document.body.appendChild(el);
  return el;
}

let hintTimer = 0;
function showZoomHint(text, clientX, clientY) {
  const el = ensureZoomHint();
  el.textContent = text;
  const z = cssUiZoom();
  const vw = window.innerWidth || 800;
  const vh = window.innerHeight || 600;
  const x = Math.max(8, Math.min((clientX || 24) + 12, vw - 180));
  const y = Math.max(8, Math.min((clientY || 24) + 12, vh - 48));
  el.style.left = `${visualToCssPx(x, z)}px`;
  el.style.top = `${visualToCssPx(y, z)}px`;
  el.classList.add("show");
  clearTimeout(hintTimer);
  hintTimer = window.setTimeout(() => el.classList.remove("show"), 1200);
}

/**
 * @param {number|string} n
 * @param {{ persist?: boolean, fromAuto?: boolean }} [opts]
 */
export async function setWebviewScalePercent(n, opts = {}) {
  if (applying) return currentPercent;
  const persist = opts.persist !== false && !opts.fromAuto;
  applying = true;
  try {
    currentPercent = applyCssZoom(n);
    if (!opts.fromAuto) autoScale = false;
    if (persist) persistScale(currentPercent, autoScale);
    updateScaleControls();
  } catch (e) {
    console.warn("[ui-scale]", e);
  } finally {
    applying = false;
  }
  return currentPercent;
}

/**
 * @param {boolean} on
 * @param {{ persist?: boolean, flash?: boolean }} [opts]
 */
export async function setWebviewAutoScale(on, opts = {}) {
  autoScale = !!on;
  if (opts.persist !== false) persistScale(currentPercent, autoScale);
  if (autoScale) {
    await applyAutoScale();
  } else {
    updateScaleControls();
  }
  if (opts.persist !== false && opts.flash !== false) {
    flash(autoScale ? "已依螢幕大小縮放" : "已改為手動縮放");
  }
}

async function applyAutoScale() {
  if (!autoScale) return;
  const w = screenWidth();
  lastAutoScreenW = w;
  const next = computeAutoScalePercent(w);
  await setWebviewScalePercent(next, { persist: false, fromAuto: true });
  persistScale(currentPercent, true);
}

export async function resetWebviewScale() {
  autoScale = false;
  persistScale(100, false);
  await setWebviewScalePercent(100, { persist: true, fromAuto: false });
  flash("已重設為 100%");
}

function bumpScale(delta, ev) {
  if (autoScale) {
    showZoomHint("自動縮放開啟中，請先關閉再手動調整", ev && ev.clientX, ev && ev.clientY);
    return;
  }
  void setWebviewScalePercent(currentPercent + delta, { persist: true });
}

function wireShortcuts() {
  window.addEventListener(
    "keydown",
    (ev) => {
      if (!ev.ctrlKey && !ev.metaKey) return;
      if (ev.key === "0" || ev.code === "Numpad0") {
        ev.preventDefault();
        if (autoScale) {
          showZoomHint("自動縮放開啟中，請先關閉再手動調整", 24, 24);
          return;
        }
        void resetWebviewScale();
        return;
      }
      if (ev.key === "=" || ev.key === "+" || ev.key === "ArrowUp") {
        ev.preventDefault();
        bumpScale(10, ev);
        return;
      }
      if (ev.key === "-" || ev.key === "_" || ev.key === "ArrowDown") {
        ev.preventDefault();
        bumpScale(-10, ev);
      }
    },
    { passive: false }
  );
  window.addEventListener(
    "wheel",
    (ev) => {
      if (!ev.ctrlKey && !ev.metaKey) return;
      ev.preventDefault();
      const delta = ev.deltaY < 0 ? 10 : -10;
      bumpScale(delta, ev);
    },
    { passive: false }
  );
  window.addEventListener("resize", () => {
    if (!autoScale) return;
    void applyAutoScale();
  });
  window.setInterval(() => {
    if (!autoScale) return;
    const w = screenWidth();
    if (w !== lastAutoScreenW) void applyAutoScale();
  }, 1500);
}

export function initWebviewScale() {
  purgeLegacyStorage();
  autoScale = readStoredAuto();
  currentPercent = readStoredScale();
  wireShortcuts();
  updateScaleControls();
  const apply = () => {
    if (autoScale) void applyAutoScale();
    else void setWebviewScalePercent(currentPercent, { persist: false, fromAuto: false });
  };
  if (typeof requestAnimationFrame === "function") requestAnimationFrame(apply);
  else apply();
}

export { clampScalePercent, computeAutoScalePercent };
