/**
 * B5d 選資料夾就判定（規格 §2.2 S02–S07、S15 暫行、S18、§3.1 MC 版本列、§3.4 N-03／N-04）。
 *
 * 純函式、不碰 DOM：輸入是後端唯讀查詢的結果（inspect_folder_cmd、inspect_instance_identity_cmd、
 * probe 的 modsChanged），輸出是狀態卡的狀態或橫幅決定。computePackState 依優先序呼叫這裡。
 * 失效安全：查不到（null）＝舊行為，不擋。
 */

import { shortPackName, packNameFromPath } from "./pack-state.js";
import { packUpdateState } from "./pack-update.js";

export const FOLDER_STATE = Object.freeze({
  checking: "CHECKING",
  wrongFolder: "S02",
  server: "S03",
  cannotWrite: "S04",
  brokenRecord: "S05",
  originUnreachable: "S06",
  copied: "S07",
  packChanged: "S15",
  appliedNoResult: "S18",
});

/** 狀態卡動作（除了 pack-state 的 pick-folder／run 之外，B5d 新增的）。 */
export const FOLDER_ACTION = Object.freeze({
  usePath: "use-path",
  recheck: "recheck",
  relaunchAdmin: "relaunch-admin",
  resetRecord: "reset-record",
  openMarker: "open-marker",
  forkInstance: "fork-instance",
  serverOverride: "server-override",
});

const PICK = Object.freeze({ action: "pick-folder", label: "重新選擇" });
const PICK_OTHER = Object.freeze({ action: "pick-folder", label: "改選其他資料夾" });
const RECHECK = Object.freeze({ action: FOLDER_ACTION.recheck, label: "重新檢查" });

/** S04 依分類碼的現況句（≤40 字；規格 §2.2 S04）。 */
export const WRITE_SENTENCES = Object.freeze({
  needs_admin: "這個資料夾需要系統管理員權限才寫得進去",
  network: "網路磁碟暫時寫不進去，可能是連線不穩",
  antivirus: "Windows 安全性可能擋住了寫入",
  denied: "寫不進這個資料夾：Windows 拒絕寫入",
  disk_full: "寫不進這個資料夾：磁碟空間不夠",
  cloud: "寫不進這個資料夾：OneDrive 雲端同步擋住了",
  locked: "寫不進這個資料夾：有檔案正被其他程式使用",
  missing: "寫不進這個資料夾：找不到它，可能被移走了",
  unknown: "寫不進這個資料夾，原因不明（詳見紀錄）",
});

/** S04 的詳細行（一行白話原因或做法）。 */
const WRITE_DETAILS = Object.freeze({
  denied: "可能是防毒的「受控資料夾存取」或資料夾被設成唯讀",
  antivirus: "檔案可能被移到隔離區，請到 Windows 安全性查看",
  disk_full: "清出一些空間後按「重新檢查」",
  cloud: "在 mods 按右鍵選「一律保留在此裝置上」後再檢查",
  locked: "遊戲或啟動器可能還開著，關閉後再檢查",
});

function st(id, fields) {
  return {
    id,
    tone: "block",
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

function norm(path) {
  return String(path || "")
    .trim()
    .replace(/[\\/]+$/, "")
    .replace(/\\/g, "/")
    .toLowerCase();
}

/** 路徑相同（大小寫、斜線、結尾分隔符不影響）。 */
export function samePath(a, b) {
  return !!norm(a) && norm(a) === norm(b);
}

/**
 * 資料夾本身與身分的判定（S02–S07、檢查中）。都沒事回 null（交給後面的狀態）。
 *
 * @param {{
 *   instancePath?: string, inspecting?: boolean,
 *   inspection?: {reachable?: boolean, validation?: {ok?: boolean, reason?: string}, shape?: object, write?: object} | null,
 *   identityPending?: boolean, identity?: {state?: string, path?: string, originName?: string} | null,
 *   serverOverride?: boolean, extraShown?: (key: string) => boolean,
 * }} input
 */
export function folderGateState(input) {
  const src = input && typeof input === "object" ? input : {};
  const shown = typeof src.extraShown === "function" ? src.extraShown : () => true;
  const extra = (key, text) => (shown(key) ? { extraLine: text, disclosureKey: key } : { disclosureKey: key });
  if (src.inspecting) {
    return st(FOLDER_STATE.checking, {
      tone: "neutral",
      sentence: "正在檢查這個資料夾…",
      primary: { action: "run", label: "開始翻譯", disabled: true },
      disabledReason: "檢查完才能開始",
    });
  }
  const inspection = src.inspection && typeof src.inspection === "object" ? src.inspection : null;
  if (!inspection) return null;
  const validation = inspection.validation || {};
  const shape = inspection.shape || {};

  if (inspection.reachable === false) {
    return st(FOLDER_STATE.cannotWrite, {
      sentence: String(validation.reason || "連不到這個資料夾").replace(/。$/, ""),
      detailLines: [String(src.instancePath || "")].filter(Boolean),
      primary: RECHECK,
      secondary: [PICK_OTHER],
    });
  }

  if (!validation.ok) {
    if (shape.kind === "mods_selected" && shape.parent) {
      return st(FOLDER_STATE.wrongFolder, {
        sentence: "你選的是 mods，要選它上一層的遊戲資料夾",
        primary: {
          action: FOLDER_ACTION.usePath,
          label: `改用「${shortPackName(packNameFromPath(shape.parent), 10)}」`,
          path: shape.parent,
        },
      });
    }
    if (shape.kind === "launcher_list" && Array.isArray(shape.candidates) && shape.candidates.length) {
      const picks = shape.candidates.slice(0, 5).map((c) => ({
        action: FOLDER_ACTION.usePath,
        label: shortPackName(c.name || packNameFromPath(c.path)),
        path: c.path,
      }));
      return st(FOLDER_STATE.wrongFolder, {
        sentence: "這裡有好幾個模組整合包，請選一個",
        primary: PICK,
        secondary: picks.slice(0, 3),
        more: picks.slice(3),
      });
    }
    if (shape.kind === "no_mods") {
      return st(FOLDER_STATE.wrongFolder, {
        sentence: "這裡找不到 mods，可能選到上一層或下一層",
        primary: PICK,
      });
    }
    return null; // 其他原因（路徑不存在等）由 pack-state 的一般 S02 用後端原因句
  }

  if (shape.kind === "server" && !src.serverOverride) {
    return st(FOLDER_STATE.server, {
      sentence: "這是伺服器資料夾，請選玩家電腦上的遊戲資料夾",
      ...extra("server", "玩家看到的文字不在伺服器這份"),
      primary: PICK,
      // 有 options.txt＝這份也被當成玩家遊戲資料夾開過，讓玩家自己決定
      secondary: shape.hasOptions ? [{ action: FOLDER_ACTION.serverOverride, label: "仍要翻這個資料夾" }] : [],
    });
  }

  const write = inspection.write;
  if (write && write.writable === false) {
    const code = WRITE_SENTENCES[write.code] ? write.code : "unknown";
    const detail = WRITE_DETAILS[code];
    return st(FOLDER_STATE.cannotWrite, {
      sentence: WRITE_SENTENCES[code],
      detailLines: [detail, write.path ? `位置：${write.path}` : ""].filter(Boolean),
      // 管理員鈕只給本機磁碟的系統保護位置（後端分類 needs_admin；網路磁碟一律不會是這個碼）
      primary:
        code === "needs_admin"
          ? { action: FOLDER_ACTION.relaunchAdmin, label: "以系統管理員身分重新開啟" }
          : RECHECK,
      secondary: [PICK_OTHER],
    });
  }

  if (src.identityPending) {
    return st(FOLDER_STATE.checking, {
      tone: "neutral",
      sentence: "正在確認這個資料夾的紀錄…",
      primary: { action: "run", label: "開始翻譯", disabled: true },
      disabledReason: "確認完才能開始",
    });
  }

  const identity = src.identity && typeof src.identity === "object" ? src.identity : null;
  const kind = identity ? String(identity.state || "") : "";
  if (kind === "record_broken") {
    return st(FOLDER_STATE.brokenRecord, {
      sentence: "套用紀錄壞了，先停下，沒動任何檔案",
      ...extra("brokenRecord", "為了不弄錯檔案或認錯模組整合包"),
      detailLines: [identity.path ? `紀錄位置：${identity.path}` : ""].filter(Boolean),
      primary: { action: FOLDER_ACTION.resetRecord, label: "重設套用紀錄" },
      secondary: [RECHECK],
    });
  }
  if (kind === "marker_broken") {
    return st(FOLDER_STATE.brokenRecord, {
      sentence: "工具記號壞了，先停下，沒動任何檔案",
      ...extra("brokenRecord", "為了不弄錯檔案或認錯模組整合包"),
      detailLines: ["修復方法：刪掉這個記號檔，再按「重新檢查」", identity.path || ""].filter(Boolean),
      primary: { action: FOLDER_ACTION.openMarker, label: "開啟記號所在位置", path: identity.path || "" },
      secondary: [RECHECK],
    });
  }
  if (kind === "unreachable") {
    return st(FOLDER_STATE.originUnreachable, {
      sentence: "上次的位置連不到（外接硬碟沒接上？）",
      detailLines: [identity.path || "", "確定那個位置已不在，才選「當成新的模組整合包」"].filter(Boolean),
      primary: { action: FOLDER_ACTION.recheck, label: "接上後重新檢查" },
      secondary: [{ action: FOLDER_ACTION.forkInstance, label: "當成新的模組整合包" }],
    });
  }
  if (kind === "copied") {
    const origin = shortPackName(identity.originName || packNameFromPath(identity.path)) || "另一個遊戲資料夾";
    return st(FOLDER_STATE.copied, {
      sentence: `這份是從「${origin}」複製來的，要先分開記錄`,
      ...extra("copied", "分開後兩份各自翻譯、移除，互不影響"),
      primary: { action: FOLDER_ACTION.forkInstance, label: "當成新的模組整合包" },
      secondary: identity.path ? [{ action: FOLDER_ACTION.usePath, label: "改選原本那份", path: identity.path }] : [],
    });
  }
  return null;
}

/**
 * 翻過的包之後的狀態（S15 暫行、S18）。都不成立回 null。
 */
export function resultState(input) {
  const src = input && typeof input === "object" ? input : {};
  const name = src.packName || "這個模組整合包";
  const shown = typeof src.extraShown === "function" ? src.extraShown : () => true;
  // B6a-1：探測帶更新差異 → S16（MC 版本變了）或 S15「翻譯更新的部分」；沒有差異資料時退回 S15 暫行
  const updated = src.packChanged && !src.translationComplete
    ? packUpdateState({ packName: name, packUpdate: src.packUpdate, extraShown: shown })
    : null;
  if (updated) return updated;
  // 剛重新翻完（這次工作階段已記下新的 mods 指紋）時不再說有變動
  if (src.packChanged && !src.translationComplete) {
    return st(FOLDER_STATE.packChanged, {
      tone: "neutral",
      sentence: `「${name}」上次翻譯後有變動，要重新翻譯`,
      ...(shown("packChanged")
        ? { extraLine: "翻過的句子會用翻譯記憶直接沿用", disclosureKey: "packChanged" }
        : { disclosureKey: "packChanged" }),
      primary: { action: "run", label: "重新翻譯" },
      more: [{ action: "delete-and-restart", label: "刪除結果並重翻", danger: true }],
      showAiRow: true,
    });
  }
  if (src.hasTranslationRecord && !src.hasResult && !src.translationComplete) {
    return st(FOLDER_STATE.appliedNoResult, {
      tone: "neutral",
      sentence: "已套用到遊戲，但這台電腦沒留下翻譯結果",
      detailLines: ["之後要補只能重新翻譯"],
      primary: null,
      secondary: [{ action: "run", label: "重新翻譯" }],
    });
  }
  return null;
}

/** §3.1 MC 版本列：偵測不到又還沒選時，主要按鈕停用、狀態卡出下拉。 */
export function versionGate({ versionUnknown = false } = {}) {
  if (!versionUnknown) return null;
  return { showVersionRow: true, disabledReason: "偵測不到 Minecraft 版本，請選一個" };
}

/**
 * 新的判定會把舊的「可開始」狀態的詳細行換掉：知道沒有 options.txt 時改由 N-04 說；
 * 知道有 options.txt 時不用再提醒；不知道時保留原本那句（失效安全）。
 */
export function readyDetailLines({ translationComplete = false, hasOptions = null } = {}) {
  if (translationComplete || hasOptions !== null) return [];
  return ["建議先啟動一次遊戲再翻譯：有些模組第一次啟動才產生語言檔。"];
}

/** N-03、N-04 只在這些狀態出現（規格 §3.4：只在 S13–S16；B5d 的「可開始」即 S13 暫行；S16 由 B6a-1 加）。 */
export const BANNER_STATES = Object.freeze(["READY", "S15", "S16"]);

/**
 * 橫幅決定：N-03 遊戲正在執行（每次選資料夾最多一次）、N-04 還沒啟動過（沒 options.txt）。
 * @returns {{show: object[], hide: string[]}}
 */
export function folderBanners({ stateId = "", gameRunning = false, hasOptions = null, dismissed = [] } = {}) {
  const show = [];
  const hide = [];
  const eligible = BANNER_STATES.includes(stateId);
  const closed = new Set(Array.isArray(dismissed) ? dismissed : []);
  if (eligible && gameRunning && !closed.has("N-03")) {
    show.push({ id: "N-03", text: "遊戲正在執行。可以先翻，套用前要關遊戲" });
  } else hide.push("N-03");
  if (eligible && hasOptions === false && !closed.has("N-04")) {
    show.push({ id: "N-04", text: "這個遊戲還沒啟動過，套用前要先開一次" });
  } else hide.push("N-04");
  return { show, hide };
}

/** D 區「上次：<包名>」：只讀上次路徑與包名（百分比與狀態按下後才算）。 */
export function lastInstanceButton({ lastPath = "", currentPath = "", locked = false, lockReason = "" } = {}) {
  const path = String(lastPath || "").trim();
  if (!path || samePath(path, currentPath)) return { visible: false, label: "", path: "", disabledReason: "" };
  const name = shortPackName(packNameFromPath(path)) || "上次的資料夾";
  return { visible: true, label: `上次：${name}`, path, disabledReason: locked ? lockReason : "" };
}

/** 瀏覽視窗的起始位置：上次路徑，沒有時用偵測到的常見啟動器資料夾。 */
export function pickStartPath({ lastPath = "", launcherDir = "" } = {}) {
  return String(lastPath || "").trim() || String(launcherDir || "").trim();
}

/** 開始翻譯前「已有可用結果」（會跳三選一）：模組整合包有變動時不算（規格 §8.3）。 */
export function hasUsableExistingResult({ probe = null, hasShareableFiles = false, packChanged = false } = {}) {
  if (packChanged || (probe && (probe.modsChanged || probe.status === "changed"))) return false;
  return !!(probe && (probe.status === "ready" || probe.shareable)) || !!hasShareableFiles;
}

/** 本機已有翻譯卡：有變動（status changed）時不出現（那不是可用的結果）。 */
export function isUsableProbe(probe) {
  return !!probe && !!probe.status && probe.status !== "none" && probe.status !== "changed" && !probe.modsChanged;
}
/**
 * 審查 5b：這個資料夾偵測不到版本時，下拉裡「上一個資料夾自動偵測留下的值」不算選好；玩家自己選的才算。
 */
export function versionUnknown({ detectFailed = false, selectValue = "", autoDetected = "" } = {}) {
  if (!detectFailed) return false;
  return !String(selectValue || "") || String(autoDetected) === "true";
}

/**
 * 審查 5a：偵測不到版本時，所有會開始翻譯的按鈕（主要或次要的 run）都停用並寫原因、出版本列；
 * 已經因為別的原因停用的狀態保留自己的原因。
 */
export function applyVersionGate(state, { versionUnknown: unknown = false } = {}) {
  const gate = versionGate({ versionUnknown: unknown });
  if (!gate || !state) return state;
  const runs = (b) => b && b.action === "run" && !b.disabled;
  const primaryRuns = runs(state.primary);
  const secondaryRuns = (state.secondary || []).some(runs);
  if (!primaryRuns && !secondaryRuns) return state;
  return {
    ...state,
    primary: primaryRuns ? { ...state.primary, disabled: true } : state.primary,
    secondary: (state.secondary || []).map((b) => (runs(b) ? { ...b, disabled: true } : b)),
    disabledReason: state.disabledReason || gate.disabledReason,
    showVersionRow: true,
  };
}
