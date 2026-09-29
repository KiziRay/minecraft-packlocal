/**
 * 流程狀態（規格 ux-spec §2）：依目前的遊戲資料夾算出「一句現況＋唯一主要按鈕」。
 *
 * 純函式、不碰 DOM，所以能單元測試 R-1（主要按鈕恰好 0 或 1，0 只在例外清單）。
 * 畫面由 status-card.js 依這裡的結果畫；翻譯邏輯一律不在這裡。
 *
 * B5a-1 只實作：S00、S01、S02（暫行：資料夾檢查沒過）、S09（暫行：翻譯中只放停止鈕）、
 * S19a／S19b、暫行「可開始」（READY）與 D 區停用原因 S20。
 * B5d 加：S02 三種、S03–S07、檢查中、S15 暫行、S18、MC 版本列（判定在 folder-state.js）；
 * 接續卡（RESUME-card）刪除，併入 D 區「上次：<包名>」。其餘狀態由 B5b、B5c 加。
 * 失效安全：輸入缺欄位時退回最保守的狀態（沒同意→S00、沒資料夾→S01）。
 */

import { applyVersionGate, folderGateState, readyDetailLines, resultState } from "./folder-state.js";
import { aiBlockedState, applyPrestart, reTranslatePrestart } from "./prestart.js";
import { failureState } from "./run-failure.js";

export const STATE = Object.freeze({
  consent: "S00",
  noFolder: "S01",
  folderNotRight: "S02",
  translating: "S09",
  /** B5b：停止中（只有停止鈕本身，G4.25 文字不變）。 */
  stopping: "S10",
  /** B5b：出錯（依原因給主要按鈕）。 */
  failed: "S12",
  /** B5b：開始前確認模式（從重新翻譯進入）。 */
  prestart: "S13",
  /** B5b：從狀態卡按接續補完／修復、AI 還沒就緒（紅色 AI 列＋同一顆主要按鈕停用）。 */
  aiBlocked: "AI-BLOCKED",
  removedWithResult: "S19a",
  removedNoResult: "S19b",
  /** 暫行「可開始」：一句現況＋「開始翻譯」（呼叫舊 onRun，之後的彈窗照舊，B5b 改）。 */
  ready: "READY",
  /** 暫行：舊的「已翻完未套用卡」出現時（主要動作由該卡提供，B5c 併入狀態卡 S11）。 */
  pendingCard: "S11-card",
  busy: "BUSY",
});

/** 主要按鈕可以是 0 顆的狀態（規格 R-1 例外）。S20 不是狀態卡狀態，所以不在這裡。 */
export const ZERO_PRIMARY_ALLOWED = Object.freeze(["S10", "S17", "S18", "S11-card"]);

export const ACTION = Object.freeze({
  acceptConsent: "accept-consent",
  pickFolder: "pick-folder",
  run: "run",
  stop: "stop",
  applyResult: "apply-result",
  deleteAndRestart: "delete-and-restart",
});

/** 狀態句、附加行、停用原因的字數上限（規格 §5.2；包名不計）。 */
export const SENTENCE_MAX = 40;
/** 包名超長時截到 16 字加省略號（規格 §5.2 共用規則）。 */
export const PACK_NAME_MAX = 16;

export function shortPackName(name, max = PACK_NAME_MAX) {
  const text = String(name || "").trim();
  if (!text) return "";
  const chars = Array.from(text);
  return chars.length > max ? chars.slice(0, max).join("") + "…" : text;
}

/** 從路徑取最後一段當包名（「C:/x/ATM10/」→「ATM10」）。 */
export function packNameFromPath(path) {
  const parts = String(path || "")
    .trim()
    .split(/[\\/]+/)
    .filter(Boolean);
  return parts.length ? parts[parts.length - 1] : "";
}

function clip(text, max = SENTENCE_MAX) {
  const chars = Array.from(String(text || "").replace(/\s+/g, " ").trim());
  return chars.length > max ? chars.slice(0, max - 1).join("") + "…" : chars.join("");
}

function normalize(input) {
  const src = input && typeof input === "object" ? input : {};
  const instancePath = String(src.instancePath || "").trim();
  const validation = src.validation && typeof src.validation === "object" ? src.validation : {};
  const removal = src.removal && typeof src.removal === "object" ? src.removal : null;
  return {
    consentAccepted: !!src.consentAccepted,
    instancePath,
    packName: shortPackName(src.packName || packNameFromPath(instancePath)),
    validated: !!validation.ok,
    validationReason: String(validation.reason || "").trim(),
    versionBlocked: !!src.versionBlocked,
    versionBlockReason: String(src.versionBlockReason || "").trim(),
    busy: !!src.busy,
    busyKind: String(src.busyKind || ""),
    hasResult: !!src.hasResult,
    removal:
      removal && String(removal.instancePath || "").trim() === instancePath && instancePath ? removal : null,
    pickFolderFresh: src.pickFolderFresh !== false,
    applyPendingShown: !!src.applyPendingShown,
    translationComplete: !!src.translationComplete,
    // B5d：選資料夾就判定（folder-state.js）
    folder: src.folder && typeof src.folder === "object" ? src.folder : null,
    packChanged: !!src.packChanged,
    hasTranslationRecord: !!src.hasTranslationRecord,
    versionUnknown: !!src.versionUnknown,
    hasOptions: typeof src.hasOptions === "boolean" ? src.hasOptions : null,
    extraShown: typeof src.extraShown === "function" ? src.extraShown : () => true,
    // B5b：開始前確認（§3.1 列）、翻譯中進度、出錯
    prestart: src.prestart && typeof src.prestart === "object" ? src.prestart : null,
    progress: src.progress && typeof src.progress === "object" ? src.progress : null,
    stopping: !!src.stopping,
    failure:
      src.failure && typeof src.failure === "object" && String(src.failure.instancePath || "").trim() === instancePath && instancePath
        ? src.failure
        : null,
  };
}

function state(id, fields) {
  return {
    id,
    tone: "neutral",
    sentence: "",
    extraLine: "",
    disclosureKey: "",
    detailLines: [],
    primary: null,
    secondary: [],
    more: [],
    disabledReason: "",
    showAiRow: false,
    showVersionRow: false,
    ...fields,
  };
}

function busyReason(kind) {
  if (kind === "apply") return "正在套用到遊戲，完成後才能開始";
  if (kind === "font") return "字體工具正在建立字體包，完成後才能開始";
  if (kind === "migrate") return "正在搬移工具資料，搬完才能開始";
  return "正在處理，完成後才能開始";
}

function busySentence(kind) {
  if (kind === "apply") return "正在套用到遊戲…";
  if (kind === "font") return "字體工具正在建立字體包…";
  if (kind === "migrate") return "正在搬移工具資料…";
  return "正在處理…";
}

/** 移除翻譯結果的下一行：刪了幾個、放回幾個、幾個無法還原（規格 S19a 附加）。 */
export function removalDetailLines(result) {
  const r = result && typeof result === "object" ? result : {};
  const count = (value) => (Array.isArray(value) ? value.length : Number(value) || 0);
  const removed = count(r.removed ?? r.removedFiles ?? r.removed_files);
  const restored = count(r.restored ?? r.restoredFiles ?? r.restored_files);
  const unrestorable = count(r.unrestorable);
  const lines = [`刪了 ${removed} 個檔、放回 ${restored} 個原檔、${unrestorable} 個無法還原`];
  const quarantined = Array.isArray(r.quarantined) ? r.quarantined.filter(Boolean) : [];
  if (quarantined.length) lines.push(`你原本的版本保存在隔離區（${quarantined.length} 個），詳見紀錄`);
  return lines;
}

/**
 * 算出目前的流程狀態。
 *
 * @param {{
 *   consentAccepted?: boolean, instancePath?: string, packName?: string,
 *   validation?: {ok?: boolean, reason?: string}, versionBlocked?: boolean, versionBlockReason?: string,
 *   busy?: boolean, busyKind?: string, hasResult?: boolean,
 *   removal?: {instancePath: string, result?: object, hasResult?: boolean} | null,
 *   pickFolderFresh?: boolean,
 * }} input
 */
export function computePackState(input) {
  const i = normalize(input);
  // 審查 5a：偵測不到 MC 版本時，所有會開始翻譯的狀態都停用（可開始、S15、S18 的次要、S19b）
  const gated = applyVersionGate(computeState(i), { versionUnknown: i.versionUnknown });
  // B5b：顯示 AI 列的狀態就是 §3.1 開始前確認模式：紅列時主要按鈕停用（先處理標紅的那一列）
  if (gated && gated.showAiRow && i.prestart && Array.isArray(i.prestart.rows)) return applyPrestart(gated, i.prestart.rows);
  return gated;
}

function computeState(i) {
  const name = i.packName || "這個模組整合包";

  if (!i.consentAccepted) {
    // 同意頁本身就是這個狀態的畫面；按鈕在同意頁上（唯一位置），狀態卡不重複畫。
    return state(STATE.consent, {
      sentence: "開始前先看完使用前說明",
      primary: { action: ACTION.acceptConsent, label: "我了解，開始使用" },
    });
  }

  if (i.busy && (i.busyKind === "translate" || !i.busyKind)) {
    const view = i.progress || {};
    if (i.stopping || view.stopping) {
      // S10：只有停止鈕本身（文字由 stop-button 管，G4.25／G4.29 不變）
      return state(STATE.stopping, {
        sentence: "正在停止，寫出已翻好的部分…",
        primary: { action: ACTION.stop, label: "停止翻譯" },
      });
    }
    const tips = i.extraShown("runTips");
    return state(STATE.translating, {
      sentence: view.sentence || `正在翻「${name}」`,
      extraLine: tips ? "可以縮小視窗；翻譯中先別開這個模組整合包" : "",
      disclosureKey: "runTips",
      detailLines: Array.isArray(view.notes) ? view.notes : [],
      progress: i.progress,
      primary: { action: ACTION.stop, label: "停止翻譯" },
    });
  }

  if (i.busy) {
    const reason = busyReason(i.busyKind);
    return state(STATE.busy, {
      sentence: busySentence(i.busyKind),
      primary: { action: ACTION.run, label: "開始翻譯", disabled: true },
      disabledReason: reason,
    });
  }

  if (!i.instancePath) {
    return state(STATE.noFolder, {
      sentence: "選要翻譯的模組整合包遊戲資料夾（裡面有 mods）",
      extraLine: i.pickFolderFresh ? "CurseForge：在模組整合包上按右鍵→開啟資料夾" : "",
      disclosureKey: "pickFolder",
      primary: { action: ACTION.pickFolder, label: "選擇遊戲資料夾" },
    });
  }

  // B5d：資料夾本身與身分（S02 三種、S03–S07、檢查中）
  if (i.folder) {
    const gate = folderGateState({ ...i.folder, instancePath: i.instancePath, extraShown: i.extraShown });
    if (gate) return gate;
  }

  if (!i.validated || i.versionBlocked) {
    const why = i.versionBlocked
      ? i.versionBlockReason || "這個 Minecraft 版本太舊，無法翻譯"
      : i.validationReason && i.validationReason !== "尚未選擇遊戲資料夾。"
        ? i.validationReason
        : "這個資料夾不能翻譯，請重新選擇";
    return state(STATE.folderNotRight, {
      tone: "block",
      sentence: clip(why),
      primary: { action: ACTION.pickFolder, label: "重新選擇" },
    });
  }

  // 暫行（B5c 取代）：待套用卡在畫面上時，主要動作「套用到遊戲」在那張卡
  if (i.applyPendingShown) {
    return state(STATE.pendingCard, { sentence: "已翻完，還沒套用到遊戲" });
  }

  // B5b：從狀態卡按接續補完／修復、AI 還沒就緒（R-8 不經過 §3.1，直接在這裡顯示紅列）
  const origin = i.prestart && i.prestart.aiBlockOrigin;
  if (origin === "supplement" || origin === "repair") return aiBlockedState(origin);

  // B5b：S12 出錯（本批暫用錯誤字串判斷原因）
  if (i.failure) return failureState(i.failure.classified);

  // B5b：從 S15／已翻譯／S18 按「重新翻譯」→ 開始前確認（刻意多一步，R-8）
  if (i.prestart && i.prestart.open) return reTranslatePrestart(name);

  if (i.removal) {
    const hasResult = i.removal.hasResult ?? i.hasResult;
    const detailLines = removalDetailLines(i.removal.result);
    const sentence = "已移除翻譯，遊戲回到原本的語言";
    if (hasResult) {
      return state(STATE.removedWithResult, {
        sentence,
        detailLines,
        primary: { action: ACTION.applyResult, label: "套用到遊戲" },
      });
    }
    return state(STATE.removedNoResult, {
      sentence,
      detailLines,
      primary: { action: ACTION.run, label: "開始翻譯" },
      showAiRow: true,
    });
  }

  // B5d：S15 暫行（模組整合包有變動）、S18（已套用、這台電腦沒留結果）
  const after = resultState({
    packName: name,
    packChanged: i.packChanged,
    hasTranslationRecord: i.hasTranslationRecord,
    hasResult: i.hasResult,
    translationComplete: i.translationComplete,
    extraShown: i.extraShown,
  });
  if (after) return { ...after, reTranslate: true };

  // S13（還沒翻過）：開始前確認模式本身；第一次多說「確認下面幾項就能開始」（規格 §2.2 S13）
  const fresh = !i.translationComplete && !i.hasResult;
  const firstTime = i.extraShown("prestart");
  return state(STATE.ready, {
    sentence: i.translationComplete
      ? "這個模組整合包已翻譯"
      : fresh
        ? firstTime
          ? "還沒翻過，確認下面幾項就能開始"
          : "還沒翻過"
        : `已選好「${name}」，可以開始翻譯`,
    reTranslate: i.translationComplete,
    primary: { action: ACTION.run, label: i.translationComplete ? "重新翻譯" : "開始翻譯" },
    more: i.hasResult ? [{ action: ACTION.deleteAndRestart, label: "刪除結果並重翻", danger: true }] : [],
    // 原本在開始前「保留結果」詢問裡的提醒（B5a-2 刪了那個詢問）；B5d：知道有沒有 options.txt 時改由 N-04 說或不說
    detailLines: readyDetailLines({ translationComplete: i.translationComplete, hasOptions: i.hasOptions }),
    showAiRow: true,
  });
}

/**
 * D 區（整合包區）的停用原因：翻譯中整區停用，原因寫 S20 句（規格 §2.2 S20）。
 * 其他工作進行中也鎖，但原因不同。
 */
export function folderAreaLock(input) {
  const src = input && typeof input === "object" ? input : {};
  if (!src.busy) return { locked: false, reason: "" };
  const kind = String(src.busyKind || "");
  if (kind === "translate" || !kind) {
    const name = shortPackName(src.packName || packNameFromPath(src.instancePath)) || "這個模組整合包";
    return { locked: true, reason: `正在翻「${name}」，翻完才能換資料夾` };
  }
  return { locked: true, reason: "正在處理，完成後才能換資料夾" };
}

/**
 * D 區「移除翻譯」按鈕（全工具唯一入口）：有翻譯紀錄才出現；忙碌時停用並寫原因（R-4）。
 */
export function removeTranslationControl(input) {
  const src = input && typeof input === "object" ? input : {};
  const visible = !!src.instancePath && !!src.hasTranslationRecord;
  if (!visible) return { visible: false, disabledReason: "" };
  const lock = folderAreaLock(src);
  return { visible: true, disabledReason: lock.locked ? lock.reason : "" };
}

/**
 * 翻譯中切到其他分頁（字體工具）時的共用一行（審查中1）：「正在翻譯「<包名>」」＋同一顆停止鈕。
 * 翻譯分頁可見時不顯示（狀態卡已經在說，一件事只在一處說）。
 */
export function runElsewhereLine({ page = "translate", state: current = null, packName = "" } = {}) {
  if (page === "translate" || !current || current.id !== STATE.translating) return { shown: false, sentence: "" };
  const name = shortPackName(packName) || "這個模組整合包";
  return { shown: true, sentence: `正在翻譯「${name}」` };
}
