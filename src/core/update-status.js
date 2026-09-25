// 檢查更新結果的白話分類。主視窗（update.js）與設定視窗（settings-window-actions.js）共用；
// 這個檔案不碰 DOM，設定視窗載入它不會帶進主視窗的更新視窗接線。

export const TEST_BUILD_MESSAGE = "測試版不自動更新。要換新的測試版，請到測試資料夾手動下載。";

/**
 * 把 check_update 的結果歸成四種狀況，主視窗與設定視窗共用同一套白話訊息。
 * kind：test-build（測試版，不更新、不跳視窗）｜failed｜current｜available
 */
export function describeUpdateCheck(info) {
  if (!info || typeof info !== "object") {
    return { kind: "failed", message: "暫時無法檢查更新", showModal: false };
  }
  if (info.testBuild) {
    return { kind: "test-build", message: String(info.message || TEST_BUILD_MESSAGE), showModal: false };
  }
  if (!info.ok) {
    return { kind: "failed", message: String(info.message || "暫時無法檢查更新"), showModal: false };
  }
  if (!info.updateAvailable) {
    return { kind: "current", message: String(info.message || "已是最新版"), showModal: false };
  }
  return { kind: "available", message: String(info.message || ""), showModal: true };
}
