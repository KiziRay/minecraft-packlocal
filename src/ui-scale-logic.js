/** 介面縮放純函式。與 ZeitFrei 相同：html { zoom }，自動檔依螢幕寬。 */

export const SCALE_MIN = 80;
export const SCALE_MAX = 170;
export const SCALE_STEP = 10;
export const DESIGN_WIDTH = 1080;

export function clampScalePercent(n) {
  const raw = Number(n);
  if (!Number.isFinite(raw)) return 100;
  const stepped = Math.round(raw / SCALE_STEP) * SCALE_STEP;
  return Math.min(SCALE_MAX, Math.max(SCALE_MIN, stepped));
}

/** ZeitFrei computeAutoScale：用螢幕寬，不用視窗 innerWidth。 */
export function computeAutoScalePercent(screenWidth) {
  const w = Number(screenWidth);
  if (!Number.isFinite(w) || w <= 0) return 100;
  if (w <= 1400) return 90;
  if (w <= 1920) return 100;
  if (w <= 2304) return 110;
  if (w <= 2560) return 120;
  if (w <= 3200) return 130;
  return 140;
}

export function parseCssZoom(raw) {
  const z = Number.parseFloat(raw);
  return Number.isFinite(z) && z > 0 ? z : 1;
}

export function visualToCssPx(visualPx, zoom) {
  const z = zoom > 0 ? zoom : 1;
  return Number(visualPx) / z;
}

export function parseStoredScale(raw) {
  return clampScalePercent(raw);
}

export function parseStoredAuto(raw) {
  return raw === "1" || raw === "true" || raw === true;
}

/**
 * @param {Element|null} hit
 * @param {Element|null} expected
 */
export function hitTargetOk(hit, expected) {
  if (!expected) return true;
  if (!hit) return false;
  if (hit === expected) return true;
  try {
    return typeof expected.contains === "function" && expected.contains(hit);
  } catch (_) {
    return false;
  }
}

/**
 * @param {{ id: string, hit: Element|null, expected: Element|null }[]} probes
 */
export function evaluateHitProbes(probes) {
  const failed = [];
  for (const p of probes || []) {
    if (!p || !p.expected) continue;
    if (!hitTargetOk(p.hit, p.expected)) failed.push(p.id || "?");
  }
  return { ok: failed.length === 0, failed };
}
