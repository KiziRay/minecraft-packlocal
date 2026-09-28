import { parseCssZoom, visualToCssPx } from "../ui-scale-logic.js";
import { TOUR_STEPS, tourAdvance, tourMeta, tourNextLabel, tourPlan } from "./tour-steps.js";

const ONBOARDING_STORAGE_KEY = "modpack-i18n-onboarding-seen-v1.0.9";
/** 舊鍵：看過就算看過，改版不該讓既有使用者被重播一次導覽。 */
const ONBOARDING_STORAGE_KEY_LEGACY = ["modpack-i18n-onboarding-seen-v1.0.7"];
/** 走到第幾步。中途關掉下次從這裡接續，不從頭重來。 */
const ONBOARDING_PROGRESS_KEY = "modpack-i18n-onboarding-step-v1";

function cssZoom() {
  try {
    return parseCssZoom(getComputedStyle(document.documentElement).zoom);
  } catch (_) {
    return 1;
  }
}

/**
 * 新手引導（規格 §4.2 tour，B5a-1）：4 步、依狀態出現，步驟定義在 tour-steps.js。
 * 還沒選資料夾時走完第 1 步先暫停（不算看完），選好資料夾後由主視窗呼叫
 * resumeOnboarding() 從第 2 步接著。Esc／跳過＝整個跳過並記為看過，通知主視窗 toast。
 */
export function createOnboarding({ $, closeGuideOverlaySafe, isInstanceReady = () => false, onSkipped = () => {} }) {
  let onboardIndex = 0;
  let onboardActive = false;
  function hasSeenOnboarding() {
    try {
      if (localStorage.getItem(ONBOARDING_STORAGE_KEY) === "1") return true;
      return ONBOARDING_STORAGE_KEY_LEGACY.some((k) => localStorage.getItem(k) === "1");
    } catch (_) {
      return false;
    }
  }
  function markOnboardingSeen() {
    try {
      localStorage.setItem(ONBOARDING_STORAGE_KEY, "1");
      localStorage.removeItem(ONBOARDING_PROGRESS_KEY);
    } catch (_) { /* ignore */ }
  }
  /** 記住走到第幾步：中途關掉，下次接著看，不要每次都從第一步重來。 */
  function saveProgress(index) {
    try { localStorage.setItem(ONBOARDING_PROGRESS_KEY, String(index)); } catch (_) { /* ignore */ }
  }
  function loadProgress() {
    try {
      const n = parseInt(localStorage.getItem(ONBOARDING_PROGRESS_KEY) || "0", 10);
      return Number.isFinite(n) && n > 0 ? n : 0;
    } catch (_) { return 0; }
  }
  function isVisible(el) {
    if (!el) return false;
    const style = window.getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    return style.display !== "none" && style.visibility !== "hidden" && !el.hidden && rect.width > 0 && rect.height > 0;
  }
  function resolveOnboardTarget(step) {
    if (!step) return null;
    const el = document.querySelector(step.selector);
    if (isVisible(el)) return el;
    const fallback = step.fallback ? document.querySelector(step.fallback) : null;
    return isVisible(fallback) ? fallback : null;
  }
  function layoutOnboarding() {
    if (!onboardActive) return;
    const step = TOUR_STEPS[onboardIndex];
    const root = $("onboard-root");
    const hole = $("onboard-hole");
    const bubble = $("onboard-bubble");
    const meta = $("onboard-meta");
    const title = $("onboard-title");
    const body = $("onboard-body");
    const prev = $("onboard-prev");
    const next = $("onboard-next");
    if (!root || !bubble || !step) return;
    if (meta) meta.textContent = tourMeta(onboardIndex);
    if (title) title.textContent = step.title;
    if (body) body.textContent = step.body;
    // 第 2 步是選好資料夾後才接著出現的，回上一步會回到已經做完的第 1 步，沒有意義
    if (prev) prev.disabled = onboardIndex <= 1;
    if (next) next.textContent = tourNextLabel(onboardIndex);
    const target = resolveOnboardTarget(step);
    const pad = 8;
    const vw = window.innerWidth || 800;
    const vh = window.innerHeight || 600;
    const z = cssZoom();
    let holeRect = { left: vw * 0.2, top: vh * 0.2, width: vw * 0.6, height: 80 };
    if (target) {
      const r = target.getBoundingClientRect();
      holeRect = { left: Math.max(8, r.left - pad), top: Math.max(8, r.top - pad), width: Math.min(vw - 16, r.width + pad * 2), height: Math.min(vh - 16, r.height + pad * 2) };
    }
    if (hole) {
      hole.hidden = false;
      hole.style.left = `${Math.round(visualToCssPx(holeRect.left, z))}px`;
      hole.style.top = `${Math.round(visualToCssPx(holeRect.top, z))}px`;
      hole.style.width = `${Math.round(visualToCssPx(holeRect.width, z))}px`;
      hole.style.height = `${Math.round(visualToCssPx(holeRect.height, z))}px`;
    }
    bubble.style.visibility = "hidden";
    bubble.style.left = "0px";
    bubble.style.top = "0px";
    const b = bubble.getBoundingClientRect();
    let left = holeRect.left;
    let top = holeRect.top + holeRect.height + 12;
    if (top + b.height > vh - 12) top = Math.max(12, holeRect.top - b.height - 12);
    if (left + b.width > vw - 12) left = Math.max(12, vw - b.width - 12);
    if (left < 12) left = 12;
    if (top < 12) top = 12;
    bubble.style.left = `${Math.round(visualToCssPx(left, z))}px`;
    bubble.style.top = `${Math.round(visualToCssPx(top, z))}px`;
    bubble.style.visibility = "";
  }
  function focusStepTarget(step) {
    window.setTimeout(() => {
      const target = resolveOnboardTarget(step);
      if (target && typeof target.focus === "function") target.focus();
    }, 0);
  }
  function hide() {
    onboardActive = false;
    const root = $("onboard-root");
    if (root) { root.hidden = true; root.classList.remove("is-active"); root.setAttribute("aria-hidden", "true"); }
    window.removeEventListener("resize", layoutOnboarding);
  }
  /** 結束引導。markSeen＝記為看過；skipped＝玩家跳過（主視窗會 toast 告知去哪找回）。 */
  function stopOnboarding(markSeen, { skipped = false } = {}) {
    const wasActive = onboardActive;
    hide();
    if (markSeen) markOnboardingSeen();
    if (skipped && wasActive) onSkipped();
  }
  function show(index) {
    const root = $("onboard-root");
    if (!root) return;
    closeGuideOverlaySafe();
    onboardIndex = index;
    onboardActive = true;
    root.hidden = false;
    root.classList.add("is-active");
    root.setAttribute("aria-hidden", "false");
    layoutOnboarding();
    window.addEventListener("resize", layoutOnboarding);
  }
  function showPlanned(progress) {
    const plan = tourPlan({ progress, instanceReady: !!isInstanceReady() });
    if (plan.kind === "done") {
      stopOnboarding(true);
      return;
    }
    if (plan.kind === "pause") {
      const pausedAt = TOUR_STEPS[Math.max(0, progress - 1)];
      saveProgress(progress);
      hide();
      // 引導開著時背景是 inert，框住的按鈕按不到；收起引導後把焦點交給它，玩家直接按 Enter 就能選
      focusStepTarget(pausedAt);
      return;
    }
    show(plan.index);
  }
  function startOnboarding(opts = {}) {
    const force = !!opts.force;
    if (!force && hasSeenOnboarding()) return;
    // 重播（force）一律從頭；正常啟動則接續上次看到的那一步
    const progress = force ? 0 : loadProgress();
    if (force) saveProgress(0);
    showPlanned(progress);
  }
  /** 選好資料夾後呼叫：引導在第 1 步後暫停的話，從第 2 步接著。 */
  function resumeOnboarding() {
    if (onboardActive || hasSeenOnboarding()) return;
    const progress = loadProgress();
    if (progress <= 0) return;
    showPlanned(progress);
  }
  function previousOnboardingStep() {
    if (onboardIndex <= 1) return;
    onboardIndex -= 1;
    saveProgress(onboardIndex);
    layoutOnboarding();
  }
  function nextOnboardingStep() {
    const progress = tourAdvance({ index: onboardIndex });
    showPlanned(progress);
  }
  return { startOnboarding, stopOnboarding, resumeOnboarding, layoutOnboarding, isOnboardingActive: () => onboardActive, previousOnboardingStep, nextOnboardingStep };
}
