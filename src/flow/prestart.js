/**
 * B5b 開始前確認模式（規格 §3.1）：取代開始前最多 5 個彈窗。
 *
 * 每列一行；有問題的列變紅並帶該列唯一的修正按鈕。紅列存在時主要按鈕停用：「先處理標紅的那一列」。
 * 開跑後不再跳對話框，直到需要 D-03（未事先確認的無備份覆蓋，後端沒有「本輪已確認」旗標，保留）或結束。
 * 純函式、不碰 DOM；列的草稿值（備份、本地翻不好時）由呼叫端保存，按下開始時才落盤（記住的選擇，§4.3）。
 */

import { AI_FIT, AI_LABEL, AI_PREP } from "./ai-readiness.js";

export const BLOCKED_REASON = "先處理標紅的那一列";

export const PRESTART_ACTION = Object.freeze({
  skipAi: "ai-skip-once",
  change: "ai-change",
  back: "prestart-back",
  supplement: "supplement",
  repair: "repair",
});

export const BACKUP_OPTIONS = Object.freeze([
  Object.freeze({ value: "always", label: "先備份會被換掉的原檔（建議）" }),
  Object.freeze({ value: "never", label: "不備份" }),
]);
export const BACKUP_ACK_LABEL = "我了解之後無法還原被覆蓋的檔案";

export const CLOUD_OPTIONS = Object.freeze([
  Object.freeze({ value: "0", label: "只用本地（不花錢）" }),
  Object.freeze({ value: "1", label: "改用線上 AI 補完（會用額度）" }),
]);

/**
 * @param {{
 *   ai: ReturnType<import("./ai-readiness.js").aiReadiness>,
 *   skipAiOnce?: boolean,
 *   cloud?: {needsConsent?: boolean, configured?: boolean} | null,
 *   cloudDraft?: string,
 *   backupChoice?: string, backupDraft?: {value?: string, ack?: boolean} | null,
 *   aiExpanded?: boolean,
 * }} input
 */
export function prestartRows(input) {
  const src = input && typeof input === "object" ? input : {};
  const ai = src.ai || { mode: "local", ready: false, sentence: "", summary: "", fix: null, paidNote: "" };
  const rows = [];
  const skip = !!src.skipAiOnce && ai.mode !== "none";
  if (skip) {
    rows.push({ id: "ai", tone: "ok", text: "這次不用 AI（只用共享庫與翻譯記憶）", fix: null, secondary: [change()], note: "" });
  } else if (ai.ready) {
    rows.push({ id: "ai", tone: "ok", text: ai.summary, fix: null, secondary: [change()], note: ai.paidNote || "" });
  } else {
    rows.push({
      id: "ai",
      tone: "block",
      text: ai.sentence,
      fix: ai.fix,
      secondary: [{ action: PRESTART_ACTION.skipAi, label: "這次不用 AI" }, change()],
      note: ai.paidNote || "",
    });
  }
  // 本地翻不好時：只在「本地模型＋已設好線上 AI」且從沒選過時問一次（U3-12；G0.6：預設只用本地）
  const cloud = src.cloud && typeof src.cloud === "object" ? src.cloud : null;
  if (!skip && ai.mode === "local" && cloud && cloud.needsConsent && cloud.configured) {
    const value = src.cloudDraft === "1" ? "1" : "0";
    rows.push({ id: "cloud", tone: "ok", text: "本地翻不好的句子：", options: CLOUD_OPTIONS, value });
  }
  // 備份：全工具第一次（translate.backupChoice 沒選過）在這裡問
  const chosen = src.backupChoice === "always" || src.backupChoice === "never";
  if (!chosen) {
    const draft = src.backupDraft && typeof src.backupDraft === "object" ? src.backupDraft : {};
    const value = draft.value === "never" ? "never" : "always";
    const needsAck = value === "never" && !draft.ack;
    rows.push({
      id: "backup",
      tone: needsAck ? "block" : "ok",
      text: "第一次套用：",
      options: BACKUP_OPTIONS,
      value,
      ack: value === "never" ? { label: BACKUP_ACK_LABEL, checked: !!draft.ack } : null,
    });
  }
  return rows;
}

function change() {
  return { action: PRESTART_ACTION.change, label: "更換" };
}

/** 有紅列就不能開始。 */
export function prestartBlocked(rows) {
  return Array.isArray(rows) && rows.some((r) => r && r.tone === "block");
}

/**
 * 按下開始時要記住的選擇（§4.3）：回傳要寫的值與 toast（第一次記住時告訴玩家在哪改）。
 * 這次不用 AI 不記住。
 */
export function prestartCommits(rows) {
  const out = [];
  for (const row of Array.isArray(rows) ? rows : []) {
    if (row.id === "backup" && row.tone !== "block") {
      out.push({
        kind: "backup",
        value: row.value,
        toast: `已記住：${row.value === "never" ? "不備份" : "備份"}，可到 設定→資料與備份 改`,
      });
    }
    if (row.id === "cloud") {
      out.push({
        kind: "cloud",
        value: row.value,
        toast: `已記住：${row.value === "1" ? "線上補完" : "只用本地"}，可到 設定→翻譯與 AI 改`,
      });
    }
  }
  return out;
}

/** 「更換」展開時每個選項的「適合誰」與選中者的準備清單。 */
export function aiChoiceLines(mode) {
  return Object.keys(AI_LABEL).map((key) => ({
    mode: key,
    label: AI_LABEL[key],
    fit: AI_FIT[key],
    prep: key === mode ? AI_PREP[key] : "",
  }));
}

/**
 * 把 §3.1 的列套到一個會開始翻譯的狀態上：紅列→主要按鈕停用並寫原因。
 * 已經因為別的原因停用的保留自己的原因（例：偵測不到 MC 版本）。
 */
export function applyPrestart(state, rows) {
  if (!state) return state;
  const list = Array.isArray(rows) ? rows : [];
  const next = { ...state, rows: list };
  if (!prestartBlocked(list)) return next;
  const p = state.primary;
  if (p && !p.disabled && isStartAction(p.action)) {
    next.primary = { ...p, disabled: true };
    next.disabledReason = state.disabledReason || BLOCKED_REASON;
  }
  return next;
}

function isStartAction(action) {
  return action === "run" || action === PRESTART_ACTION.supplement || action === PRESTART_ACTION.repair;
}

/**
 * 重新翻譯（S15 暫行、已翻譯、S18）刻意多一步進入開始前確認（R-8）：
 * 主要「開始重新翻譯」、次要「返回」。
 */
export function reTranslatePrestart(name) {
  return {
    id: "S13",
    tone: "neutral",
    sentence: `確認下面幾項就能重新翻譯「${name}」`,
    extraLine: "",
    disclosureKey: "",
    detailLines: [],
    primary: { action: "run", label: "開始重新翻譯" },
    secondary: [{ action: PRESTART_ACTION.back, label: "返回" }],
    more: [],
    disabledReason: "",
    showAiRow: true,
    showVersionRow: false,
  };
}

/**
 * AI 還沒就緒時，從狀態卡按接續補完／修復翻譯檔（R-8 不經過 §3.1）：
 * 直接在狀態卡顯示紅色 AI 列並停用那顆主要按鈕；處理好後再按同一顆。
 */
export function aiBlockedState(origin) {
  const label = origin === PRESTART_ACTION.repair ? "修復翻譯檔" : "接續補完";
  return {
    id: "AI-BLOCKED",
    tone: "block",
    sentence: `${label}前要先處理 AI`,
    extraLine: "",
    disclosureKey: "",
    detailLines: [],
    primary: { action: origin === PRESTART_ACTION.repair ? PRESTART_ACTION.repair : PRESTART_ACTION.supplement, label },
    secondary: [],
    more: [],
    disabledReason: "",
    showAiRow: true,
    showVersionRow: false,
  };
}
