const CONTENT_FADE_MS = 260;
let startupContentRevealed = false;
let pageTransitionToken = 0;

export function prefersReducedMotion() {
  return !!window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches;
}

/** 強制卸除啟動骨架／is-loading，避免永遠蓋住可點元件。 */
export function forceRevealUi() {
  try {
    const body = document.body;
    if (!body) return;
    body.classList.remove("is-loading");
    body.querySelectorAll(".is-loading").forEach((el) => el.classList.remove("is-loading"));
  } catch (_) {
    /* ignore */
  }
}

export function revealInitialContent() {
  if (!document.body || startupContentRevealed) {
    forceRevealUi();
    return;
  }
  startupContentRevealed = true;
  forceRevealUi();
  if (prefersReducedMotion()) return;
  document.body.classList.add("content-fade-in");
  window.setTimeout(() => {
    document.body.classList.remove("content-fade-in");
  }, CONTENT_FADE_MS + 90);
}

export function revealPagePanel(panel) {
  if (!panel) return;
  const token = ++pageTransitionToken;
  document.querySelectorAll(".page-panel.is-loading").forEach((el) => {
    el.classList.remove("is-loading");
  });
  panel.classList.remove("is-loading");
  panel.classList.remove("content-fade-in");
  if (prefersReducedMotion()) return;
  panel.classList.add("content-fade-in");
  window.setTimeout(() => {
    if (token === pageTransitionToken) panel.classList.remove("content-fade-in");
  }, CONTENT_FADE_MS + 90);
}

// 腳本一載入就掛 fallback：即使 DOMContentLoaded 中途拋錯，2s 內也必須露出 UI
(function bootRevealGuard() {
  const run = () => forceRevealUi();
  if (document.body) run();
  document.addEventListener("DOMContentLoaded", run);
  window.setTimeout(run, 2000);
})();
