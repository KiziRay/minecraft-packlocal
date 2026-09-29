/**
 * 狀態卡（規格 §1.1 E 區）：把 computePackState 的結果畫到固定位置。
 *
 * - 整個翻譯頁只有這裡有主要按鈕；主要按鈕沿用既有 id（#btn-run、#btn-stop）與既有接線。
 * - 停用用 aria-disabled（仍可聚焦），原因常駐在按鈕下一行（R-4）。
 * - 停止鈕的文字由 ui/stop-button.js 管（G4.25），這裡不覆寫。
 * - S00 的按鈕在同意頁上（唯一位置），狀態卡整張收起來，不重複畫。
 */
import { ACTION, STATE } from "./pack-state.js";
import { DISCLOSURES } from "./disclosure.js";
import { renderPrestartRows, renderRunProgress } from "./prestart-view.js";

/** 主要動作 → 狀態卡裡對應的按鈕 id（同一時間只露出其中一顆）。 */
export const PRIMARY_BUTTON_IDS = Object.freeze({
  [ACTION.pickFolder]: "btn-card-pick",
  [ACTION.run]: "btn-run",
  [ACTION.stop]: "btn-stop",
  [ACTION.applyResult]: "btn-card-apply",
});

/** B5d：其他主要動作（改用上一層、重新檢查、以管理員重開、重設紀錄、當成新的…）共用這一顆，動作寫在 data-action。 */
export const GENERIC_PRIMARY_ID = "btn-card-action";

/** 純函式：決定每顆按鈕與每一行的樣子。DOM 套用在 applyStatusCard。 */
export function planStatusCard(state, { extraShown = null } = {}) {
  const s = state && typeof state === "object" ? state : {};
  const hidden = s.id === STATE.consent || !s.id;
  const primary = s.primary || null;
  const primaryId = primary ? PRIMARY_BUTTON_IDS[primary.action] || GENERIC_PRIMARY_ID : "";
  const buttons = {};
  for (const [action, id] of Object.entries(PRIMARY_BUTTON_IDS)) {
    const isPrimary = !hidden && id === primaryId;
    buttons[id] = {
      hidden: !isPrimary,
      label: isPrimary && action !== ACTION.stop ? primary.label : null,
      ariaDisabled: isPrimary && !!primary.disabled,
    };
  }
  const generic = !hidden && primaryId === GENERIC_PRIMARY_ID;
  buttons[GENERIC_PRIMARY_ID] = {
    hidden: !generic,
    label: generic ? primary.label : null,
    ariaDisabled: generic && !!primary.disabled,
    action: generic ? primary.action : "",
    path: generic ? String(primary.path || "") : "",
  };
  const extraText = String(s.extraLine || "");
  const showExtra = extraShown == null ? !!extraText : !!extraShown && !!extraText;
  return {
    hidden,
    id: s.id || "",
    tone: s.tone || "neutral",
    sentence: String(s.sentence || ""),
    extra: {
      text: extraText,
      shown: showExtra,
      // 說明退場後原位置留「？」叫回（R-6）；還在顯示時給「不再顯示」
      canDismiss: showExtra && !!s.disclosureKey,
      canRecall: !showExtra && !!s.disclosureKey,
      key: s.disclosureKey || "",
      topic: (DISCLOSURES[s.disclosureKey] && DISCLOSURES[s.disclosureKey].topic) || "",
    },
    detailLines: Array.isArray(s.detailLines) ? s.detailLines.filter(Boolean) : [],
    buttons,
    // 主要或次要按鈕停用時都寫原因（審查 5a：S18 的「重新翻譯」是次要）
    reason:
      (primary && primary.disabled) || (Array.isArray(s.secondary) && s.secondary.some((b) => b && b.disabled))
        ? String(s.disabledReason || "")
        : "",
    secondary: Array.isArray(s.secondary) ? s.secondary : [],
    more: Array.isArray(s.more) ? s.more : [],
    showAiRow: !!s.showAiRow,
    versionRow: !!s.showVersionRow,
    // B5b：§3.1 開始前確認的列（只在顯示 AI 列的狀態）與翻譯中進度區（S09）
    rows: s.showAiRow && Array.isArray(s.rows) ? s.rows : [],
    progress: s.id === STATE.translating && s.progress ? s.progress : null,
    inlineReason: s.reasonInline ? String(s.disabledReason || "") : "",
  };
}

function setHidden(el, hidden) {
  if (!el) return;
  el.hidden = !!hidden;
}

function renderActionList(container, items, doc, onAction, className) {
  if (!container) return;
  container.textContent = "";
  for (const item of items) {
    if (item && !item.action && item.note) {
      const p = doc.createElement("p");
      p.className = "status-card-more-note";
      p.textContent = String(item.note);
      container.appendChild(p);
      continue;
    }
    if (!item || !item.action) continue;
    const btn = doc.createElement("button");
    btn.type = "button";
    btn.className = item.danger ? className + " danger-text-button" : className;
    btn.dataset.action = item.action;
    btn.textContent = String(item.label || item.action);
    if (item.path) btn.dataset.path = String(item.path);
    if (item.disabled) {
      btn.setAttribute("aria-disabled", "true");
      btn.setAttribute("aria-describedby", "status-card-reason");
    }
    btn.addEventListener("click", () => {
      if (item.disabled) return;
      onAction(item.action, item);
    });
    container.appendChild(btn);
  }
}

/**
 * 把計畫套到畫面上。`$` 取 id、`doc` 用來建次要按鈕；`onAction` 收次要／更多按鈕的動作。
 * 主要按鈕沿用既有接線（#btn-run→onRun、#btn-stop→onStop），這裡只管顯示。
 */
export function applyStatusCard(plan, { $, doc, onAction = () => {}, onRowChange = () => {} }) {
  const card = $("status-card");
  if (!card) return;
  card.hidden = plan.hidden;
  card.dataset.state = plan.id;
  card.dataset.tone = plan.tone;
  card.dataset.disclosure = plan.extra.key || "";
  const sentence = $("status-card-sentence");
  if (sentence) sentence.textContent = plan.sentence;

  const extra = $("status-card-extra");
  const extraText = $("status-card-extra-text");
  if (extraText) extraText.textContent = plan.extra.text;
  setHidden(extra, !plan.extra.shown);
  setHidden($("btn-status-extra-dismiss"), !plan.extra.canDismiss);
  const help = $("btn-status-extra-help");
  setHidden(help, !plan.extra.canRecall);
  if (help && plan.extra.topic) help.setAttribute("aria-label", `說明：${plan.extra.topic}`);

  const detail = $("status-card-detail");
  if (detail) {
    detail.textContent = plan.detailLines.join("\n");
    detail.hidden = plan.detailLines.length === 0;
  }

  for (const [id, view] of Object.entries(plan.buttons)) {
    const btn = $(id);
    if (!btn) continue;
    btn.hidden = view.hidden;
    if (id === GENERIC_PRIMARY_ID && btn.dataset) {
      btn.dataset.action = view.action || "";
      btn.dataset.path = view.path || "";
    }
    if (view.label != null) {
      const span = btn.querySelector ? btn.querySelector(".start-label") : null;
      if (span) span.textContent = view.label;
      else btn.textContent = view.label;
    }
    // 隱藏的按鈕也歸回可用：之後被搬去別處或再次露出時，不帶著上一個狀態的停用
    const disabled = !view.hidden && view.ariaDisabled;
    btn.setAttribute("aria-disabled", disabled ? "true" : "false");
    if (disabled) btn.setAttribute("aria-describedby", "status-card-reason");
    else btn.removeAttribute("aria-describedby");
  }
  const reason = $("status-card-reason");
  if (reason) {
    reason.textContent = plan.reason;
    reason.hidden = !plan.reason;
  }

  if (doc) {
    renderActionList($("status-card-secondary"), plan.secondary, doc, onAction, "secondary-button");
    renderActionList($("status-card-more-list"), plan.more, doc, onAction, "text-button");
  }
  setHidden($("status-card-more"), plan.more.length === 0);
  // B5c：S11 按下時遊戲仍開著 → 就地原因（按鈕保持可按，關遊戲後再按一次）
  if (plan.inlineReason) {
    const reasonEl = $("status-card-reason");
    if (reasonEl) {
      reasonEl.textContent = plan.inlineReason;
      reasonEl.hidden = false;
    }
  }
  setHidden($("status-card-version-row"), !plan.versionRow);
  renderPrestartRows(plan.rows || [], { $, doc, onAction: (action) => onAction(action, { action }), onRowChange });
  renderRunProgress(plan.progress || null, { $ });
}

/** 主要按鈕被點到時：aria-disabled 的按鈕仍會收到 click，要在這裡擋下。 */
export function isAriaDisabled(el) {
  return !!el && typeof el.getAttribute === "function" && el.getAttribute("aria-disabled") === "true";
}
