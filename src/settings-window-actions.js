/**
 * 設定視窗的開發人員診斷紀錄與「關於」：檢查更新、重看引導與說明。
 * 重看引導、顯示更新交給主視窗執行，這裡只送出請求並把主視窗叫到前面。
 * 「資料與備份」的其餘動作在 settings/data-pane.js。
 */
import { SETTINGS_ACTION_EVENT } from "./core/settings-sync.js";
import { describeUpdateCheck } from "./core/update-status.js";

let deps = null;
let devLogPath = "";

async function refreshDeveloperMode() {
  const { $, invoke } = deps;
  const status = await invoke("dev_mode_status_cmd");
  $("dev-mode-card").hidden = !status?.eligible;
  if (!status?.eligible) return;
  $("dev-mode").checked = !!status.enabled;
  devLogPath = String(status.logPath || "");
  $("dev-log-path").textContent = status.enabled ? devLogPath : "尚未啟用";
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
}

export async function refreshSettingsActions(prefs) {
  const { $ } = deps;
  $("update-status-text").textContent = `目前版本 ${String(prefs?.appVersion || "—")}`;
  await refreshDeveloperMode();
}
