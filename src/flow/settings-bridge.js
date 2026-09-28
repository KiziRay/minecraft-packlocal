/**
 * 主視窗這一側的設定視窗接線（B5a-2）：回報主視窗狀態、收設定視窗做完事的通知。
 * app.js 只注入讀狀態與更新畫面的函式，這裡不碰 DOM。
 */
import {
  DATA_MIGRATING_EVENT,
  MAIN_STATE_REQUEST_EVENT,
  SETTINGS_NOTICE_EVENTS,
  createMainStateBroadcaster,
} from "./main-state.js";

const DELETE_RESULTS_KEY = "mcpl-delete-results-after-apply";
const DELETE_RESULTS_ACK_KEY = "mcpl-delete-results-ack";

const isOn = (value) => value === "1" || value === 1 || value === true || value === "true";

/**
 * 「翻完刪除翻譯結果」是否生效：要開著、而且第一次打開時同列勾過「我了解」。
 * 讀不到（設定檔壞、舊版）＝不刪（失效安全：保留結果是舊行為）。
 */
export function deleteResultsAfterApplyEnabled(getSetting) {
  try {
    return isOn(getSetting(DELETE_RESULTS_KEY, "0")) && isOn(getSetting(DELETE_RESULTS_ACK_KEY, "0"));
  } catch (_) {
    return false;
  }
}

let broadcast = null;
let dataMigrating = false;

/** 設定視窗正在搬移工具資料（審查 F10）：主視窗把它當成忙碌，開始翻譯停用。 */
export function isDataMigrating() {
  return dataMigrating;
}

/** syncUiState 每次跑完呼叫：狀態有變才送給設定視窗。 */
export function announceMainState() {
  if (broadcast) broadcast();
}

export async function wireSettingsNotices({
  listen,
  emit,
  readMainState,
  onApiKeyCleared = async () => {},
  onLocalModelDeleted = async () => {},
  onBackupsDeleted = async () => {},
  onDataMigratingChanged = () => {},
}) {
  broadcast = createMainStateBroadcaster({ emit, read: readMainState });
  const safe = (fn) => (event) => {
    Promise.resolve()
      .then(() => fn(event?.payload || {}))
      .catch(() => {});
  };
  await listen(MAIN_STATE_REQUEST_EVENT, safe(() => broadcast({ force: true })));
  await listen(SETTINGS_NOTICE_EVENTS.apiKeyCleared, safe(() => onApiKeyCleared()));
  await listen(SETTINGS_NOTICE_EVENTS.localModelDeleted, safe(() => onLocalModelDeleted()));
  await listen(SETTINGS_NOTICE_EVENTS.backupsDeleted, safe((payload) => onBackupsDeleted(String(payload.summary || ""))));
  await listen(
    DATA_MIGRATING_EVENT,
    safe((payload) => {
      dataMigrating = !!payload.active;
      onDataMigratingChanged(dataMigrating);
    })
  );
  broadcast({ force: true });
}
