import { HELP_COPY } from "./help-copy.js";
import { parseCssZoom, visualToCssPx } from "../ui-scale-logic.js";

function cssZoom() {
  try {
    return parseCssZoom(getComputedStyle(document.documentElement).zoom);
  } catch (_) {
    return 1;
  }
}

let pop = null;
let hideTimer = 0;

function ensurePop() {
  if (pop) return pop;
  pop = document.createElement("div");
  pop.className = "help-tip-pop";
  pop.hidden = true;
  pop.setAttribute("role", "tooltip");
  document.body.appendChild(pop);
  return pop;
}

function hidePop() {
  if (hideTimer) window.clearTimeout(hideTimer);
  hideTimer = 0;
  if (pop) {
    pop.hidden = true;
    pop.textContent = "";
  }
}

function showPop(btn) {
  const key = btn.getAttribute("data-help") || "";
  const text = HELP_COPY[key];
  if (!text) return;
  const el = ensurePop();
  el.textContent = text;
  el.hidden = false;
  const r = btn.getBoundingClientRect();
  const pad = 8;
  const z = cssZoom();
  let left = r.left;
  let top = r.bottom + 6;
  const width = Math.min(352, window.innerWidth - pad * 2);
  el.style.width = `${visualToCssPx(width, z)}px`;
  const box = el.getBoundingClientRect();
  if (left + box.width > window.innerWidth - pad) {
    left = Math.max(pad, window.innerWidth - box.width - pad);
  }
  if (top + box.height > window.innerHeight - pad) {
    top = Math.max(pad, r.top - box.height - 6);
  }
  el.style.left = `${visualToCssPx(Math.max(pad, left), z)}px`;
  el.style.top = `${visualToCssPx(top, z)}px`;
}

function wireButton(btn) {
  if (!btn || btn.dataset.helpReady === "true") return;
  btn.dataset.helpReady = "true";
  if (!btn.getAttribute("aria-label")) btn.setAttribute("aria-label", "說明");
  btn.addEventListener("mouseenter", () => showPop(btn));
  btn.addEventListener("focus", () => showPop(btn));
  btn.addEventListener("mouseleave", () => {
    hideTimer = window.setTimeout(hidePop, 120);
  });
  btn.addEventListener("blur", hidePop);
  btn.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (pop && !pop.hidden && pop.textContent === HELP_COPY[btn.getAttribute("data-help")]) {
      hidePop();
    } else {
      showPop(btn);
    }
  });
}

export function wireHelpTips(root = document) {
  root.querySelectorAll(".help-tip[data-help]").forEach(wireButton);
  if (!document.body.dataset.helpTipDoc) {
    document.body.dataset.helpTipDoc = "1";
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape") hidePop();
    });
    document.addEventListener("pointerdown", (event) => {
      if (event.target?.closest?.(".help-tip, .help-tip-pop")) return;
      hidePop();
    });
  }
}
