/**
 * B5a-1 對話框與浮層規範（規格 §6）：焦點陷阱、背景 inert、Esc、危險預設取消、焦點回觸發按鈕。
 * 用假的 DOM 驗證行為；confirm.js 以最小假 document 實際開一次危險對話框。
 */
import test from "node:test";
import assert from "node:assert/strict";

import { createModalManager, initialFocusRole, nextFocusIndex } from "./modal-scope.js";

function el(name, { children = [], exempt = false } = {}) {
  const node = {
    name,
    inert: false,
    hidden: false,
    disabled: false,
    dataset: exempt ? { modalExempt: "1" } : {},
    children,
    focus() {
      fakeDoc.activeElement = node;
    },
    contains(other) {
      return other === node || children.some((c) => c.contains?.(other));
    },
    querySelectorAll() {
      return children.filter((c) => c.focusable);
    },
  };
  return node;
}
function btn(name) {
  const b = el(name);
  b.focusable = true;
  return b;
}

const fakeDoc = { activeElement: null, body: { children: [] }, listeners: [], addEventListener(type, fn) { this.listeners.push({ type, fn }); } };

function setup() {
  fakeDoc.activeElement = null;
  fakeDoc.listeners = [];
  const trigger = btn("trigger");
  const app = el("app", { children: [trigger] });
  const cancel = btn("cancel");
  const middle = btn("middle");
  const ok = btn("ok");
  const dialog = el("dialog", { children: [cancel, middle, ok] });
  const inner = btn("inner");
  const second = el("second", { children: [inner] });
  const toast = el("toast", { exempt: true });
  fakeDoc.body.children = [app, dialog, second, toast];
  const manager = createModalManager({ getDoc: () => fakeDoc, isFocusable: (n) => !!n && !n.disabled && !n.hidden });
  trigger.focus();
  return { manager, trigger, app, dialog, cancel, middle, ok, second, inner, toast };
}

function key(k, shiftKey = false) {
  return { key: k, shiftKey, defaultPrevented: false, preventDefault() { this.defaultPrevented = true; }, stopPropagation() {} };
}

test("危險對話框預設焦點在取消；一般對話框在主鈕；可明確指定", () => {
  assert.equal(initialFocusRole({ danger: true }), "cancel");
  assert.equal(initialFocusRole({ danger: false }), "ok");
  assert.equal(initialFocusRole({ danger: false, initialFocus: "cancel" }), "cancel");
});

test("Tab 循環：最後一個回到第一個，Shift+Tab 從第一個跳到最後一個", () => {
  assert.equal(nextFocusIndex(3, 2, false), 0);
  assert.equal(nextFocusIndex(3, 0, true), 2);
  assert.equal(nextFocusIndex(3, -1, false), 0);
  assert.equal(nextFocusIndex(0, 0, false), -1);
});

test("開啟時背景 inert、聚焦指定的安全按鈕；關閉後拿掉 inert、焦點回觸發按鈕", () => {
  const { manager, trigger, app, dialog, cancel, toast } = setup();
  manager.open(dialog, { onEscape: () => {}, initialFocus: cancel });
  assert.equal(app.inert, true);
  assert.equal(dialog.inert, false);
  assert.equal(toast.inert, false, "toast 不被 inert（還要讀得到）");
  assert.equal(fakeDoc.activeElement, cancel);
  manager.close(dialog);
  assert.equal(app.inert, false);
  assert.equal(fakeDoc.activeElement, trigger);
});

test("Tab 不會跑出框：在最後一個按 Tab 回到第一個，在第一個按 Shift+Tab 到最後一個", () => {
  const { manager, dialog, cancel, ok, middle } = setup();
  manager.open(dialog, { initialFocus: ok });
  const tab = key("Tab");
  manager.handleKeydown(tab);
  assert.equal(tab.defaultPrevented, true);
  assert.equal(fakeDoc.activeElement, cancel);
  const back = key("Tab", true);
  manager.handleKeydown(back);
  assert.equal(fakeDoc.activeElement, ok);
  middle.focus();
  const inside = key("Tab");
  manager.handleKeydown(inside);
  assert.equal(inside.defaultPrevented, false, "框內中間交給瀏覽器");
});

test("Esc＝最安全的選項；同意頁傳 null＝Esc 不作用也不外漏", () => {
  const { manager, dialog } = setup();
  let escaped = 0;
  manager.open(dialog, { onEscape: () => (escaped += 1) });
  const esc = key("Escape");
  manager.handleKeydown(esc);
  assert.equal(escaped, 1);
  assert.equal(esc.defaultPrevented, true);
  manager.close(dialog);
  manager.open(dialog, { onEscape: null });
  const esc2 = key("Escape");
  manager.handleKeydown(esc2);
  assert.equal(esc2.defaultPrevented, true, "Esc 被吃掉，不會傳到其他 Esc 處理");
  assert.equal(manager.isOpen(dialog), true, "同意頁按 Esc 關不掉");
});

test("疊層：上層開著時下層也 inert；關掉上層後回到下層", () => {
  const { manager, app, dialog, second, inner, cancel } = setup();
  manager.open(dialog, { initialFocus: cancel });
  manager.open(second, {});
  assert.equal(dialog.inert, true);
  assert.equal(app.inert, true);
  assert.equal(fakeDoc.activeElement, inner);
  manager.close(second);
  assert.equal(dialog.inert, false);
  assert.equal(app.inert, true);
  assert.equal(fakeDoc.activeElement, cancel, "焦點回到下層的觸發處");
});

test("鍵盤監聽用 capture 掛在 document：比 app.js 其他 Esc 處理先執行", () => {
  const { manager, dialog } = setup();
  manager.open(dialog, {});
  assert.ok(fakeDoc.listeners.some((l) => l.type === "keydown"));
});

// ── confirm.js 實際開一次危險對話框（最小假 document）──
function installFakeDocument() {
  const all = [];
  function makeNode(tag, classes = []) {
    const node = {
      tagName: tag.toUpperCase(),
      classList: {
        set: new Set(classes),
        add(c) { this.set.add(c); },
        remove(c) { this.set.delete(c); },
        toggle(c, on) { if (on) this.set.add(c); else this.set.delete(c); },
        contains(c) { return this.set.has(c); },
      },
      dataset: {},
      hidden: false,
      disabled: false,
      checked: false,
      children: [],
      attrs: {},
      textContent: "",
      setAttribute(k, v) { this.attrs[k] = String(v); },
      getAttribute(k) { return this.attrs[k] ?? null; },
      appendChild(child) { this.children.push(child); all.push(child); return child; },
      focus() { globalThis.document.activeElement = node; },
      getClientRects() { return [{}]; },
      closest() { return null; },
      contains(other) { return other === node || descendants.includes(other); },
      querySelector(sel) { return descendants.find((d) => d.classList.contains(sel.slice(1))) || null; },
      querySelectorAll() { return descendants.filter((d) => ["BUTTON", "INPUT"].includes(d.tagName)); },
    };
    const descendants = [];
    Object.defineProperty(node, "innerHTML", {
      set(html) {
        descendants.length = 0;
        for (const m of String(html).matchAll(/<(\w+)[^>]*class="([^"]+)"/g)) {
          descendants.push(makeNode(m[1], m[2].split(/\s+/)));
        }
      },
    });
    return node;
  }
  const body = makeNode("body");
  globalThis.document = {
    activeElement: null,
    body,
    createElement: (tag) => makeNode(tag),
    addEventListener() {},
  };
  globalThis.window = { requestAnimationFrame: (fn) => fn() };
  return body;
}

test("confirmDialog：危險對話框開啟時焦點在「取消」，Esc 回 false；一般對話框焦點在主鈕", async () => {
  const body = installFakeDocument();
  const { confirmDialog } = await import("./confirm.js");
  const pending = confirmDialog({ title: "移除這個模組整合包的翻譯？", danger: true, confirmLabel: "移除翻譯" });
  const root = body.children[0];
  assert.equal(document.activeElement, root.querySelector(".confirm-cancel"), "危險對話框預設焦點＝取消");
  assert.equal(root.querySelector(".confirm-ok").classList.contains("danger-button"), true);
  const handled = await import("./modal-scope.js");
  const esc = key("Escape");
  handled.modalManager().handleKeydown(esc);
  assert.equal(await pending, false, "Esc＝取消");
  assert.equal(root.hidden, true);

  const plain = confirmDialog({ title: "要把剪貼簿的翻譯併入嗎？", confirmLabel: "併入" });
  assert.equal(document.activeElement, root.querySelector(".confirm-ok"));
  root.querySelector(".confirm-cancel").onclick();
  assert.equal(await plain, false);

  const withAck = confirmDialog({ title: "刪除翻譯結果並重翻？", danger: true, ackLabel: "我知道", confirmLabel: "刪除" });
  assert.equal(document.activeElement, root.querySelector(".confirm-cancel"), "要勾選的危險框也先停在取消");
  root.onclick({ target: root });
  assert.equal(await withAck, false, "點背景＝Esc＝取消");
});
