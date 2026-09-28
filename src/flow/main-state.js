/**
 * 主視窗 ↔ 設定視窗的狀態回報（B5a-2）。
 *
 * 設定視窗不載入 app.js，不知道「現在在翻譯嗎、選的是哪個遊戲資料夾」。
 * 刪除全部備份（D-07）、搬移工具資料、刪除本地模型要在翻譯中停用（規格 §1.3、§7），
 * 所以主視窗在狀態改變時送 mcpl:main-state；設定視窗開啟時送 mcpl:main-state-request 要一次。
 * 設定視窗做完事，再用 SETTINGS_NOTICE_EVENTS 通知主視窗立即更新畫面。
 */

export const MAIN_STATE_EVENT = "mcpl:main-state";
export const MAIN_STATE_REQUEST_EVENT = "mcpl:main-state-request";

/** 設定視窗 → 主視窗：做完了，請主畫面跟著更新。 */
export const SETTINGS_NOTICE_EVENTS = Object.freeze({
  apiKeyCleared: "mcpl:api-key-cleared",
  localModelDeleted: "mcpl:local-model-deleted",
  backupsDeleted: "mcpl:backups-deleted",
});

/** 只送設定視窗需要的欄位（不送金鑰、不送紀錄）。 */
export function mainStateSnapshot({ busy, instancePath, outputDir, packName } = {}) {
  return {
    busy: !!busy,
    instancePath: String(instancePath || "").trim(),
    outputDir: String(outputDir || "").trim(),
    packName: String(packName || "").trim(),
  };
}

/**
 * 狀態沒變就不送（syncUiState 一秒可能跑很多次）；force＝設定視窗剛開、要一次。
 * @param {{ emit: (event: string, payload: unknown) => unknown, read: () => object }} deps
 */
export function createMainStateBroadcaster({ emit, read }) {
  let last = "";
  return function broadcast({ force = false } = {}) {
    const snapshot = mainStateSnapshot(read());
    const key = JSON.stringify(snapshot);
    if (!force && key === last) return false;
    last = key;
    try {
      const pending = emit(MAIN_STATE_EVENT, snapshot);
      if (pending && typeof pending.catch === "function") pending.catch(() => {});
    } catch (_) {
      /* 設定視窗沒開也沒關係 */
    }
    return true;
  };
}

/** 設定視窗 → 主視窗：正在搬移工具資料（active true／false）。搬移中主視窗不能開始翻譯（審查 F10）。 */
export const DATA_MIGRATING_EVENT = "mcpl:data-migrating";

/**
 * 設定視窗開著但主視窗還沒回報狀態（主視窗剛啟動、事件掉了）：每 intervalMs 重送請求，最多 maxRetries 次，
 * 收到狀態就停（審查 F3）。
 */
export function createStateRequester({ emit, hasState, setTimer = setTimeout, intervalMs = 2000, maxRetries = 5 }) {
  let retries = 0;
  const send = () => {
    try {
      const pending = emit(MAIN_STATE_REQUEST_EVENT, {});
      if (pending && typeof pending.catch === "function") pending.catch(() => {});
    } catch (_) {
      /* 主視窗沒開也沒關係，下一次再試 */
    }
  };
  const tick = () => {
    if (hasState() || retries >= maxRetries) return;
    retries += 1;
    send();
    setTimer(tick, intervalMs);
  };
  return {
    start() {
      send();
      setTimer(tick, intervalMs);
    },
  };
}
