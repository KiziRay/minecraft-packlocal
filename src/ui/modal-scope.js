/**
 * 對話框與浮層共用的一套開關（規格 §6）。
 *
 * 開啟：記住觸發按鈕、背景設 inert、聚焦第一個安全按鈕；開著時 Tab 在框內循環；
 * Esc＝最安全的選項（由呼叫端提供；同意頁傳 null＝Esc 不作用）；
 * 關閉：拿掉 inert、焦點回觸發按鈕。可以疊層（例如問題回報浮層上再跳確認框）。
 *
 * 不直接綁 document：測試時注入假的 doc，所以焦點陷阱、Esc、inert 都能在 node 裡驗。
 */

const FOCUSABLE =
  'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])';

/** 危險對話框預設焦點在「取消」（Enter 不會誤刪）；其餘在主鈕。可用 initialFocus 明確指定。 */
export function initialFocusRole({ danger = false, initialFocus = "" } = {}) {
  if (initialFocus === "cancel" || initialFocus === "ok") return initialFocus;
  return danger ? "cancel" : "ok";
}

/** Tab 循環：最後一個往後回到第一個，第一個往前跳到最後一個。 */
export function nextFocusIndex(count, current, shift) {
  if (!count) return -1;
  if (current < 0) return shift ? count - 1 : 0;
  if (shift) return current <= 0 ? count - 1 : current - 1;
  return current >= count - 1 ? 0 : current + 1;
}

function defaultIsFocusable(el) {
  if (!el || el.disabled) return false;
  if (el.hidden) return false;
  if (typeof el.closest === "function" && el.closest("[hidden]")) return false;
  if (typeof el.getClientRects === "function" && el.getClientRects().length === 0) return false;
  return true;
}

export function createModalManager({ getDoc = () => globalThis.document, isFocusable = defaultIsFocusable } = {}) {
  const stack = [];
  let keyListenerDoc = null;

  function doc() {
    return getDoc();
  }

  function focusablesOf(root) {
    if (!root || typeof root.querySelectorAll !== "function") return [];
    return Array.from(root.querySelectorAll(FOCUSABLE)).filter((el) => isFocusable(el));
  }

  function top() {
    return stack.length ? stack[stack.length - 1] : null;
  }

  function applyInert() {
    const d = doc();
    const body = d && d.body;
    if (!body) return;
    const current = top();
    for (const child of Array.from(body.children || [])) {
      if (child && child.dataset && child.dataset.modalExempt === "1") continue;
      const contains = current && typeof child.contains === "function" && child.contains(current.root);
      child.inert = !!current && child !== current.root && !contains;
    }
  }

  function focusSafely(el) {
    if (el && typeof el.focus === "function") {
      try {
        el.focus();
      } catch (_) {
        /* ignore */
      }
    }
  }

  function resolveInitial(entry) {
    const pick = entry.initialFocus;
    const target = typeof pick === "function" ? pick() : pick;
    if (target && isFocusable(target)) return target;
    return focusablesOf(entry.root)[0] || null;
  }

  function onKeydown(event) {
    const entry = top();
    if (!entry) return;
    if (event.key === "Escape") {
      event.preventDefault();
      if (typeof event.stopPropagation === "function") event.stopPropagation();
      if (typeof entry.onEscape === "function") entry.onEscape();
      return;
    }
    if (event.key !== "Tab") return;
    const list = focusablesOf(entry.root);
    if (!list.length) {
      event.preventDefault();
      return;
    }
    const active = doc() ? doc().activeElement : null;
    const index = list.indexOf(active);
    // 焦點在框內中間的元素時交給瀏覽器；只有頭尾與跑到框外時才接手
    const atEdge = index < 0 || (event.shiftKey ? index === 0 : index === list.length - 1);
    if (!atEdge) return;
    event.preventDefault();
    focusSafely(list[nextFocusIndex(list.length, index, !!event.shiftKey)]);
  }

  function ensureKeyListener() {
    const d = doc();
    if (!d || keyListenerDoc === d || typeof d.addEventListener !== "function") return;
    // capture：比 app.js 其他 Esc 處理先執行，Esc 只作用在最上層的框
    d.addEventListener("keydown", onKeydown, true);
    keyListenerDoc = d;
  }

  /**
   * @param {Element} root 對話框或浮層最外層
   * @param {{onEscape?: (()=>void)|null, initialFocus?: Element|(()=>Element|null)}} options
   */
  function open(root, { onEscape = null, initialFocus = null } = {}) {
    if (!root) return;
    const existing = stack.findIndex((e) => e.root === root);
    if (existing >= 0) stack.splice(existing, 1);
    const d = doc();
    const entry = { root, onEscape, initialFocus, returnTo: d ? d.activeElement : null };
    stack.push(entry);
    ensureKeyListener();
    applyInert();
    focusSafely(resolveInitial(entry));
  }

  function close(root) {
    const index = stack.findIndex((e) => e.root === root);
    if (index < 0) return;
    const [entry] = stack.splice(index, 1);
    applyInert();
    // 只有關掉的是最上層時才搬焦點；底下的框被關掉不該把焦點從上層搶走
    if (index === stack.length) {
      const back = entry.returnTo;
      const connected = back && (back.isConnected === undefined || back.isConnected);
      if (connected && isFocusable(back)) focusSafely(back);
      else if (top()) focusSafely(resolveInitial(top()));
    }
  }

  return {
    open,
    close,
    handleKeydown: onKeydown,
    isOpen: (root) => stack.some((e) => e.root === root),
    depth: () => stack.length,
    topRoot: () => (top() ? top().root : null),
  };
}

let shared = null;

/** 主視窗共用的一個實例（confirm.js、浮層、同意頁、引導都走這裡，疊層才算得對）。 */
export function modalManager() {
  if (!shared) shared = createModalManager();
  return shared;
}

/**
 * 盯住一個用 `hidden` 開關的浮層：變成可見就 open、變回 hidden 就 close。
 * 既有的顯示／隱藏函式不用改，焦點規範自動套上。
 */
export function watchOverlay(root, { onEscape = null, initialFocus = null, backdropEscape = false } = {}) {
  if (!root || typeof MutationObserver === "undefined") return () => {};
  const manager = modalManager();
  const sync = () => {
    if (!root.hidden && !manager.isOpen(root)) manager.open(root, { onEscape, initialFocus });
    else if (root.hidden && manager.isOpen(root)) manager.close(root);
  };
  const observer = new MutationObserver(sync);
  observer.observe(root, { attributes: true, attributeFilter: ["hidden"] });
  if (backdropEscape && typeof onEscape === "function") {
    root.addEventListener("click", (event) => {
      if (event.target === root) onEscape();
    });
  }
  sync();
  return () => observer.disconnect();
}
