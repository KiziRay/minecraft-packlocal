/**
 * B5b 狀態卡的 §3.1 列與翻譯中進度區（DOM）。資料由 prestart.js／run-progress.js 算好，這裡只畫。
 * 每列一行；紅列（data-tone=block）帶該列唯一的修正按鈕。AI 列是 E1（「用誰翻：…｜更換」）。
 */

const ROW_IDS = Object.freeze({ ai: "status-card-ai-row", cloud: "status-card-cloud-row", backup: "status-card-backup-row" });

function button(doc, { action, label }, className, onAction) {
  const b = doc.createElement("button");
  b.type = "button";
  b.className = className;
  b.dataset.action = action;
  b.textContent = label;
  b.addEventListener("click", () => onAction(action));
  return b;
}

function optionGroup(doc, row, onChange) {
  const group = doc.createElement("span");
  group.className = "prestart-options";
  group.setAttribute("role", "radiogroup");
  group.setAttribute("aria-label", row.text.replace(/：$/, ""));
  for (const opt of row.options || []) {
    const label = doc.createElement("label");
    label.className = "prestart-option";
    const input = doc.createElement("input");
    input.type = "radio";
    input.name = `prestart-${row.id}`;
    input.value = opt.value;
    input.checked = opt.value === row.value;
    input.addEventListener("change", () => onChange(row.id, opt.value, false));
    const text = doc.createElement("span");
    text.textContent = opt.label;
    label.append(input, text);
    group.appendChild(label);
  }
  return group;
}

/**
 * 畫 §3.1 的列到 #status-card-rows。
 * @param {object[]} rows prestartRows 的結果
 * @param {{$: Function, doc: Document, onAction: (a: string) => void, onRowChange: (id: string, v: string, ack: boolean) => void}} ctx
 */
export function renderPrestartRows(rows, { $, doc, onAction, onRowChange }) {
  const box = $("status-card-rows");
  if (!box || !doc) return;
  box.textContent = "";
  const list = Array.isArray(rows) ? rows : [];
  box.hidden = list.length === 0;
  for (const row of list) {
    const el = doc.createElement("div");
    el.className = "prestart-row";
    el.id = ROW_IDS[row.id] || `status-card-${row.id}-row`;
    el.dataset.row = row.id;
    el.dataset.tone = row.tone;
    const text = doc.createElement("span");
    text.className = "prestart-row-text";
    text.textContent = row.text;
    el.appendChild(text);
    if (row.options) el.appendChild(optionGroup(doc, row, onRowChange));
    if (row.fix) el.appendChild(button(doc, row.fix, "small-button prestart-fix", onAction));
    for (const s of row.secondary || []) {
      const b = button(doc, s, "text-button", onAction);
      if (s.action === "ai-change") {
        b.setAttribute("aria-controls", "ai-options-group");
        const group = $("ai-options-group");
        b.setAttribute("aria-expanded", group && !group.hidden ? "true" : "false");
      }
      el.appendChild(b);
    }
    if (row.ack) {
      const label = doc.createElement("label");
      label.className = "prestart-ack";
      const input = doc.createElement("input");
      input.type = "checkbox";
      input.id = "status-card-backup-ack";
      input.checked = !!row.ack.checked;
      input.addEventListener("change", () => onRowChange(row.id, row.value, input.checked));
      const t = doc.createElement("span");
      t.textContent = row.ack.label;
      label.append(input, t);
      el.appendChild(label);
    }
    if (row.note) {
      const note = doc.createElement("small");
      note.className = "prestart-note";
      note.textContent = row.note;
      el.appendChild(note);
    }
    box.appendChild(el);
  }
}

/**
 * 翻譯中進度區（#status-card-progress）：條數＋進度條、約還要多久、白話徽章、保證語。
 * 狀態句本身在 #status-card-sentence（唯一）；右欄只留步驟燈號與詳細數字。
 */
export function renderRunProgress(view, { $ }) {
  const box = $("status-card-progress");
  if (!box) return;
  box.hidden = !view;
  if (!view) return;
  const set = (id, text) => {
    const el = $(id);
    if (!el) return;
    el.textContent = String(text || "");
    el.hidden = !text;
  };
  set("status-card-count", view.count);
  set("status-card-eta", view.eta);
  set("status-card-badge", view.badge);
  set("status-card-reassure", view.reassure);
  const fill = $("status-card-fill");
  if (fill && fill.style) fill.style.width = `${Math.max(0, Math.min(100, Math.floor(Number(view.percent) || 0)))}%`;
}
