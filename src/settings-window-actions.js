/**
 * 設定視窗的「資料與備份」「關於」兩類：工具資料位置、備份、診斷紀錄、
 * 檢查更新、重看引導、問題回報。需要目前遊戲資料夾的動作（刪除備份、
 * 重看引導、顯示更新）交給主視窗執行，這裡只送出請求並把主視窗叫到前面。
 */
import { opDelete } from "./core/settings-patch.js";
import { SETTINGS_ACTION_EVENT, SETTINGS_UPDATED_EVENT } from "./core/settings-sync.js";
import { describeUpdateCheck } from "./core/update-status.js";

const BACKUP_CHOICE_PATH = "translate.backupChoice";
const BACKUP_CHOICE_LABELS = {
  always: "一律先備份再裝進遊戲。",
  never: "不備份（覆蓋遊戲裡的檔案前會再問你一次）。",
};

let deps = null;
let dataRoot = "";
let devLogPath = "";

function backupChoiceOf(settings) {
  const value = settings?.translate?.backupChoice;
  return value === "always" || value === "never" ? value : "";
}

function renderBackupChoice() {
  const { $ } = deps;
  const choice = backupChoiceOf(deps.settingsSnapshot());
  $("backup-choice-label").textContent = BACKUP_CHOICE_LABELS[choice] || "每次詢問：第一次裝進遊戲時會問你一次。";
  $("reset-backup-choice").hidden = !choice;
}

async function refreshDataRoot() {
  const { $, invoke } = deps;
  const info = await invoke("data_root_info_cmd");
  dataRoot = String(info?.activeRoot || "");
  $("data-root").textContent = dataRoot || "（尚未建立）";
  $("data-root-hint").textContent = info?.usingPortable
    ? "目前資料跟著工具放，不佔用系統碟。"
    : "目前資料放在系統的 AppData；可以複製到工具旁邊，原本的資料會留著當備份。";
  $("migrate-data-root").hidden = !info?.canMigrate;
}

async function refreshDeveloperMode() {
  const { $, invoke } = deps;
  const status = await invoke("dev_mode_status_cmd");
  $("dev-mode-card").hidden = !status?.eligible;
  if (!status?.eligible) return;
  $("dev-mode").checked = !!status.enabled;
  devLogPath = String(status.logPath || "");
  $("dev-log-path").textContent = status.enabled ? devLogPath : "尚未啟用";
}

async function focusMain(section) {
  await deps.invoke("focus_main_window");
  if (section) await deps.emit("mcpl:open-main-section", { section });
}

/** 請主視窗執行動作；主視窗才知道目前選的是哪個遊戲資料夾。 */
async function askMain(action) {
  await deps.invoke("focus_main_window");
  await deps.emit(SETTINGS_ACTION_EVENT, { action });
}

async function checkUpdate() {
  const { $, invoke, setStatus } = deps;
  const button = $("check-update");
  button.disabled = true;
  try {
    const described = describeUpdateCheck(await invoke("check_update"));
    $("update-status-text").textContent = described.message;
    if (described.showModal) {
      // 下載與更新視窗在主視窗處理，避免兩個視窗各下載一份
      await askMain("show-update");
      setStatus("有新版本，已在主畫面顯示更新說明。", "ok");
    } else {
      setStatus(described.message, described.kind === "failed" ? "warn" : "ok");
    }
  } catch (error) {
    setStatus(`暫時無法檢查更新：${String(error)}`, "warn");
  } finally {
    button.disabled = false;
  }
}

export function wireSettingsActions(injected) {
  deps = injected;
  const { $, invoke, setStatus } = deps;
  const warn = (prefix) => (error) => setStatus(`${prefix}：${String(error)}`, "warn");

  $("reset-backup-choice").addEventListener("click", async () => {
    try {
      // 「改回每次詢問」＝明確刪除這個設定，不是寫一個空值
      await deps.patchSettings([opDelete(BACKUP_CHOICE_PATH)]);
      // 通知主視窗同步快取，否則主視窗還以為選擇沒變
      await deps.emit(SETTINGS_UPDATED_EVENT, { path: BACKUP_CHOICE_PATH, value: null });
      renderBackupChoice();
      setStatus("已改回每次詢問。", "ok");
    } catch (error) {
      warn("無法儲存")(error);
    }
  });
  $("delete-backups").addEventListener("click", () => {
    void askMain("delete-backups")
      .then(() => setStatus("已回到主畫面，請在那裡確認要不要刪除。", "ok"))
      .catch(warn("無法開啟刪除備份"));
  });
  $("open-data-root").addEventListener("click", () => {
    void invoke("open_path", { path: dataRoot }).catch(warn("無法開啟資料夾"));
  });
  $("migrate-data-root").addEventListener("click", async () => {
    if (!window.confirm("會把資料複製到工具旁邊，原本的資料會留著當備份。要繼續嗎？")) return;
    const button = $("migrate-data-root");
    try {
      button.disabled = true;
      await invoke("migrate_data_root_cmd");
      await refreshDataRoot();
      setStatus("資料已複製過去，原本的資料仍保留當備份。請重新開啟工具讓新位置生效。", "ok");
    } catch (error) {
      warn("搬移失敗，原本的資料沒有被動到")(error);
    } finally {
      button.disabled = false;
    }
  });
  $("dev-mode").addEventListener("change", async (event) => {
    try {
      const result = await invoke("dev_mode_set_cmd", { enabled: !!event.target.checked });
      devLogPath = String(result?.logPath || "");
      $("dev-log-path").textContent = result?.enabled ? devLogPath : "尚未啟用";
      setStatus(result?.enabled ? "已開啟完整診斷紀錄。" : "已關閉完整診斷紀錄。", "ok");
    } catch (error) {
      event.target.checked = !event.target.checked;
      warn("無法切換診斷紀錄")(error);
    }
  });
  $("open-dev-log").addEventListener("click", () => {
    void invoke("open_path", { path: devLogPath }).catch(warn("無法開啟診斷紀錄"));
  });
  $("check-update").addEventListener("click", () => void checkUpdate());
  $("replay-onboarding").addEventListener("click", () => {
    void askMain("replay-onboarding").catch(warn("無法回到主畫面"));
  });
  $("goto-ai").addEventListener("click", () => void focusMain("ai").catch(warn("無法回到主畫面")));
  $("goto-translation").addEventListener("click", () => void focusMain("translate").catch(warn("無法回到主畫面")));
  $("report-issue").addEventListener("click", async () => {
    try {
      await focusMain("translate");
      await deps.emit("mcpl:show-issue-report", {});
    } catch (error) {
      warn("無法開啟問題回報")(error);
    }
  });
}

export async function refreshSettingsActions(prefs) {
  const { $ } = deps;
  $("update-status-text").textContent = `目前版本 ${String(prefs?.appVersion || "—")}`;
  renderBackupChoice();
  await Promise.all([refreshDataRoot(), refreshDeveloperMode()]);
}
