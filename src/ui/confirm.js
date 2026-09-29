/**
 * 全工具唯一的確認對話框。
 *
 * 取代散在各處的 `window.confirm`。三個理由：
 * 1. 原生對話框在 WebView2 會帶來源標題，與工具的視覺語言完全斷裂。
 * 2. 原生對話框只有一種強度——「登出 GPT」和「刪除整個結果資料夾」長得一模一樣。
 * 3. 刪除類動作必須先把「會被刪掉的實際路徑」攤開給使用者看，原生對話框做不到。
 *
 * 破壞性動作（danger + ack）預設把確認鍵鎖住，勾了「我知道這會刪掉上面的資料夾」才解鎖。
 *
 * 焦點規範（規格 §6，B5a-1）：開啟時背景 inert、Tab 鎖在框內、Esc 與點背景＝取消、
 * 關閉後焦點回觸發按鈕；危險對話框預設焦點在「取消」，Enter 只觸發目前焦點。
 */
import { initialFocusRole, modalManager } from "./modal-scope.js";

let activeResolve = null;
let rootEl = null;

function build() {
  if (rootEl) return rootEl;
  rootEl = document.createElement("div");
  rootEl.className = "confirm-overlay";
  rootEl.hidden = true;
  rootEl.setAttribute("aria-hidden", "true");
  rootEl.innerHTML = `
    <div class="confirm-shell" role="dialog" aria-modal="true" aria-labelledby="confirm-title">
      <header class="confirm-top">
        <h2 id="confirm-title" class="confirm-title"></h2>
      </header>
      <div class="confirm-body">
        <p class="confirm-message"></p>
        <ul class="confirm-affected" hidden></ul>
        <label class="option-row compact-option confirm-ack" hidden>
          <input type="checkbox" class="confirm-ack-box" />
          <span class="check-box" aria-hidden="true"></span>
          <span><strong class="confirm-ack-label"></strong></span>
        </label>
      </div>
      <div class="confirm-actions">
        <button type="button" class="text-button confirm-cancel"></button>
        <button type="button" class="primary-button confirm-ok"></button>
      </div>
    </div>`;
  document.body.appendChild(rootEl);
  return rootEl;
}

function close(result) {
  if (rootEl) {
    rootEl.hidden = true;
    rootEl.setAttribute("aria-hidden", "true");
    modalManager().close(rootEl);
  }
  document.body.classList.remove("confirm-open");
  const resolve = activeResolve;
  activeResolve = null;
  if (resolve) resolve(!!result);
}

/** 其他 overlay 判斷「現在有沒有東西擋著」時要看得到這個對話框。 */
export function isConfirmOpen() {
  return !!rootEl && !rootEl.hidden;
}

export function confirmDialog({
  title,
  body = "",
  affected = [],
  danger = false,
  confirmLabel = "確定",
  cancelLabel = "取消",
  ackLabel = "",
  initialFocus = "",
} = {}) {
  const root = build();
  // 同時只允許一個；後來者直接把前一個當成取消收掉，避免疊層卡死。
  if (activeResolve) close(false);

  root.querySelector(".confirm-shell").dataset.danger = danger ? "1" : "0";
  root.querySelector(".confirm-title").textContent = String(title || "請確認");
  root.querySelector(".confirm-message").textContent = String(body || "");

  const list = root.querySelector(".confirm-affected");
  const paths = (Array.isArray(affected) ? affected : []).filter(Boolean);
  list.textContent = "";
  list.hidden = paths.length === 0;
  for (const path of paths) {
    const li = document.createElement("li");
    li.textContent = String(path);
    list.appendChild(li);
  }

  const ackRow = root.querySelector(".confirm-ack");
  const ackBox = root.querySelector(".confirm-ack-box");
  const okBtn = root.querySelector(".confirm-ok");
  const needsAck = !!String(ackLabel || "").trim();
  ackRow.hidden = !needsAck;
  ackBox.checked = false;
  root.querySelector(".confirm-ack-label").textContent = ackLabel || "";
  okBtn.textContent = confirmLabel;
  okBtn.classList.toggle("danger-button", !!danger);
  okBtn.classList.toggle("primary-button", !danger);
  okBtn.disabled = needsAck;
  ackBox.onchange = () => {
    okBtn.disabled = needsAck && !ackBox.checked;
  };

  const cancelBtn = root.querySelector(".confirm-cancel");
  cancelBtn.textContent = cancelLabel;
  okBtn.onclick = () => close(true);
  cancelBtn.onclick = () => close(false);
  root.onclick = (event) => {
    if (event.target === root) close(false);
  };

  root.hidden = false;
  root.setAttribute("aria-hidden", "false");
  document.body.classList.add("confirm-open");
  const promise = new Promise((resolve) => {
    activeResolve = resolve;
  });
  // 危險＝焦點在取消（按 Enter 不會誤刪）；一般＝主鈕。Esc＝取消。
  const focusRole = initialFocusRole({ danger, initialFocus });
  modalManager().open(root, {
    onEscape: () => close(false),
    initialFocus: focusRole === "cancel" ? cancelBtn : okBtn.disabled ? cancelBtn : okBtn,
  });
  return promise;
}

/* ─── 多選項對話框 ───────────────────────────────────────────
 *
 * `confirmDialog` 只能是／否。但有些決定天生就有三條路（例如「已經翻譯過了」
 * 可以接續補完、覆蓋重翻、或另存一份），硬塞成是／否會讓使用者看不出
 * 「按下去會發生什麼」——這正是舊版那顆「仍要重新翻譯」的問題。
 *
 * 每個選項強制要有一句 `detail` 說明後果，不允許只有按鈕名稱。
 */

let choiceRoot = null;
let choiceResolve = null;

function buildChoice() {
  if (choiceRoot) return choiceRoot;
  choiceRoot = document.createElement("div");
  choiceRoot.className = "confirm-overlay";
  choiceRoot.hidden = true;
  choiceRoot.setAttribute("aria-hidden", "true");
  choiceRoot.innerHTML = `
    <div class="confirm-shell choice-shell" role="dialog" aria-modal="true" aria-labelledby="choice-title">
      <header class="confirm-top">
        <h2 id="choice-title" class="confirm-title"></h2>
      </header>
      <div class="confirm-body">
        <p class="confirm-message choice-message"></p>
        <div class="choice-options"></div>
      </div>
      <div class="confirm-actions">
        <button type="button" class="text-button choice-cancel"></button>
      </div>
    </div>`;
  document.body.appendChild(choiceRoot);
  return choiceRoot;
}

function closeChoice(value) {
  if (choiceRoot) {
    choiceRoot.hidden = true;
    choiceRoot.setAttribute("aria-hidden", "true");
    modalManager().close(choiceRoot);
  }
  document.body.classList.remove("confirm-open");
  const resolve = choiceResolve;
  choiceResolve = null;
  if (resolve) resolve(value || null);
}

export function isChoiceOpen() {
  return !!choiceRoot && !choiceRoot.hidden;
}

/**
 * 顯示多選項對話框。回傳被選中選項的 `value`；取消／Esc／點背景回 `null`。
 *
 * B5c：`ack`＝某個選項要先在同一個框裡勾選才可按（D-04「不備份」；規格 §3.5）。
 *
 * @param {{title:string, body?:string, options:Array<{value:string,label:string,detail:string}>, cancelLabel?:string,
 *   ack?: {label: string, forValue: string}}} config
 */
export function choiceDialog({ title, body = "", options = [], cancelLabel = "取消", ack = null } = {}) {
  const root = buildChoice();
  if (choiceResolve) closeChoice(null);

  root.querySelector(".confirm-title").textContent = String(title || "請選擇");
  const message = root.querySelector(".choice-message");
  message.textContent = String(body || "");
  message.hidden = !body;

  const list = root.querySelector(".choice-options");
  list.textContent = "";
  const gated = [];
  let ackBox = null;
  for (const option of Array.isArray(options) ? options : []) {
    if (!option || !option.value) continue;
    const button = document.createElement("button");
    button.type = "button";
    button.className = "choice-option";
    const label = document.createElement("strong");
    label.className = "choice-option-label";
    label.textContent = String(option.label || option.value);
    const detail = document.createElement("span");
    detail.className = "choice-option-detail";
    detail.textContent = String(option.detail || "");
    button.appendChild(label);
    button.appendChild(detail);
    const needsAck = !!(ack && ack.label && ack.forValue === option.value);
    if (needsAck) {
      button.setAttribute("aria-disabled", "true");
      gated.push(button);
    }
    button.onclick = () => {
      if (needsAck && !(ackBox && ackBox.checked)) return;
      closeChoice(option.value);
    };
    list.appendChild(button);
  }
  if (gated.length) {
    const row = document.createElement("label");
    row.className = "option-row compact-option confirm-ack";
    ackBox = document.createElement("input");
    ackBox.type = "checkbox";
    ackBox.className = "confirm-ack-box";
    const mark = document.createElement("span");
    mark.className = "check-box";
    mark.setAttribute("aria-hidden", "true");
    const text = document.createElement("span");
    const strong = document.createElement("strong");
    strong.textContent = String(ack.label);
    text.appendChild(strong);
    row.appendChild(ackBox);
    row.appendChild(mark);
    row.appendChild(text);
    ackBox.onchange = () => gated.forEach((b) => b.setAttribute("aria-disabled", ackBox.checked ? "false" : "true"));
    list.appendChild(row);
  }

  const cancel = root.querySelector(".choice-cancel");
  cancel.textContent = cancelLabel;
  cancel.onclick = () => closeChoice(null);
  root.onclick = (event) => {
    if (event.target === root) closeChoice(null);
  };

  root.hidden = false;
  root.setAttribute("aria-hidden", "false");
  document.body.classList.add("confirm-open");
  const promise = new Promise((resolve) => {
    choiceResolve = resolve;
  });
  modalManager().open(root, {
    onEscape: () => closeChoice(null),
    initialFocus: () => list.querySelector(".choice-option"),
  });
  return promise;
}
