import { parseCssZoom, visualToCssPx } from "../ui-scale-logic.js";

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
 * 新手引導只教「完成第一次翻譯」需要的四件事。
 *
 * 舊版有 7 步，其中「錯誤分析」「字體資源包」「⋯ 設定」對第一次開工具的人
 * 來說是雜訊——那些是遇到問題才會用到的功能，開頭講只會讓人想按跳過。
 * 減到 4 步、每步只講一件事、隨時可跳過，而且中途關掉下次會從同一步接續。
 */
const ONBOARD_STEPS = [
  {
    selector: ".path-block",
    title: "第一步：選遊戲資料夾",
    body: "先選你要翻譯的 Minecraft 整合包資料夾。通過檢查後，下面的 AI 選項與「開始翻譯」才會出現。",
  },
  {
    selector: "#ai-options-group",
    fallback: "#path-gate-hint",
    title: "第二步：選翻譯來源",
    body: "本地模型免費但要先下載一次；自訂 API 與 GPT 需要自己的帳號。不選也能翻，只是只用得到既有的中文資料。",
  },
  {
    selector: "#btn-run",
    fallback: "#path-gate-hint",
    title: "第三步：按開始翻譯",
    body: "翻完會自動套用到遊戲，中途可以隨時停止，已完成的部分都會保留。",
  },
  {
    selector: "#btn-overflow",
    title: "遇到問題時",
    body: "右上角 ⋯ 裡有完整使用說明、錯誤分析與問題回報。中文變成方框是字體問題，那裡也有字體工具。",
  },
];

export function createOnboarding({ $, closeGuideOverlaySafe }) {
  let onboardIndex = 0;
  let onboardActive = false;
  let activeSteps = [];
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
  function resolveActiveSteps() {
    activeSteps = ONBOARD_STEPS.filter((step) => !!resolveOnboardTarget(step));
    return activeSteps;
  }
  function layoutOnboarding() {
    if (!onboardActive) return;
    if (!activeSteps.length) {
      stopOnboarding(true);
      return;
    }
    if (onboardIndex >= activeSteps.length) onboardIndex = activeSteps.length - 1;
    const step = activeSteps[onboardIndex];
    const root = $("onboard-root");
    const hole = $("onboard-hole");
    const bubble = $("onboard-bubble");
    const meta = $("onboard-meta");
    const title = $("onboard-title");
    const body = $("onboard-body");
    const prev = $("onboard-prev");
    const next = $("onboard-next");
    if (!root || !bubble || !step) return;
    if (meta) meta.textContent = `${onboardIndex + 1} / ${activeSteps.length}`;
    if (title) title.textContent = step.title;
    if (body) body.textContent = step.body;
    if (prev) prev.disabled = onboardIndex <= 0;
    if (next) next.textContent = onboardIndex >= activeSteps.length - 1 ? "完成" : "下一步";
    const target = resolveOnboardTarget(step);
    if (!target) {
      resolveActiveSteps();
      layoutOnboarding();
      return;
    }
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
  function stopOnboarding(markSeen) {
    onboardActive = false;
    const root = $("onboard-root");
    if (root) { root.hidden = true; root.classList.remove("is-active"); root.setAttribute("aria-hidden", "true"); }
    if (markSeen) markOnboardingSeen();
    window.removeEventListener("resize", layoutOnboarding);
  }
  function startOnboarding(opts = {}) {
    const force = !!opts.force;
    if (!force && hasSeenOnboarding()) return;
    const root = $("onboard-root");
    if (!root) return;
    closeGuideOverlaySafe();
    resolveActiveSteps();
    // 重播（force）一律從頭；正常啟動則接續上次看到的那一步
    onboardIndex = force ? 0 : Math.min(loadProgress(), Math.max(0, activeSteps.length - 1));
    onboardActive = true;
    root.hidden = false;
    root.classList.add("is-active");
    root.setAttribute("aria-hidden", "false");
    layoutOnboarding();
    window.addEventListener("resize", layoutOnboarding);
  }
  function previousOnboardingStep() {
    if (onboardIndex <= 0) return;
    onboardIndex -= 1;
    saveProgress(onboardIndex);
    layoutOnboarding();
  }
  function nextOnboardingStep() {
    if (onboardIndex >= activeSteps.length - 1) { stopOnboarding(true); return; }
    onboardIndex += 1;
    saveProgress(onboardIndex);
    layoutOnboarding();
  }
  return { startOnboarding, stopOnboarding, layoutOnboarding, isOnboardingActive: () => onboardActive, previousOnboardingStep, nextOnboardingStep };
}
