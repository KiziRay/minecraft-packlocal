/**
 * 移除翻譯（D 區唯一入口，規格 D-05、S19）與刪除結果並重翻（狀態卡「更多」，規格 D-08）。
 *
 * 只負責「問→做→回結果」；畫面（狀態卡 S19、就地失敗原因）由主視窗依回傳值畫。
 * 後端 G1.30：遊戲開著時 restore_last_apply_cmd 一個檔都不動並回白話，這裡原樣帶回就地顯示。
 */

/** D-05：不可逆、危險，預設焦點「取消」。內文 ≤80 字、≤3 句。 */
export const D05 = Object.freeze({
  title: "移除這個模組整合包的翻譯？",
  body: "會刪掉工具加的檔，語言改回原本的設定；沒有備份的原檔放不回去。翻譯結果會保留，之後可以再套用。",
  danger: true,
  confirmLabel: "移除翻譯",
  cancelLabel: "取消",
});

/** D-08：不可逆、危險＋勾選，預設焦點「取消」。不寫「包含所有備份」。 */
export const D08 = Object.freeze({
  title: "刪除翻譯結果並重翻？",
  body: "會刪掉下面的翻譯結果資料夾，之後按「開始翻譯」從頭翻一次。這個動作無法復原。",
  danger: true,
  confirmLabel: "刪除並重翻",
  cancelLabel: "取消",
  ackLabel: "我知道這會刪掉上面的資料夾，且無法復原",
});

function errorText(error) {
  if (!error) return "移除翻譯沒有完成。";
  if (typeof error === "string") return error;
  return String(error.message || error) || "移除翻譯沒有完成。";
}

export function createRemovalFlow({
  invoke,
  confirmDialog,
  ensureGameClosed = async () => true,
  formatError = errorText,
} = {}) {
  /**
   * @returns {Promise<{status: "cancelled"} | {status: "removed", result: object} | {status: "failed", message: string, error: unknown}>}
   */
  async function removeTranslation({ instancePath, outputDir = null } = {}) {
    const path = String(instancePath || "").trim();
    if (!path) return { status: "cancelled" };
    if (!(await ensureGameClosed(path, "移除翻譯"))) return { status: "cancelled" };
    const ok = await confirmDialog({ ...D05, affected: [path] });
    if (!ok) return { status: "cancelled" };
    try {
      const result = await invoke("restore_last_apply_cmd", { instancePath: path, outputDir: outputDir || null });
      return { status: "removed", result: result || {} };
    } catch (error) {
      return { status: "failed", message: formatError(error), error };
    }
  }

  /**
   * @returns {Promise<{status: "cancelled"} | {status: "deleted", result: object} | {status: "nothing"} | {status: "failed", message: string}>}
   */
  async function deleteResultAndRestart({ outputDir, workRoot = "" } = {}) {
    const dir = String(outputDir || "").trim();
    if (!dir) return { status: "cancelled" };
    const ok = await confirmDialog({ ...D08, affected: [workRoot || dir].filter(Boolean) });
    if (!ok) return { status: "cancelled" };
    try {
      const result = await invoke("delete_result_folder_cmd", { outputDir: dir });
      const deleted = !!(result && result.deleted);
      return deleted ? { status: "deleted", result } : { status: "nothing" };
    } catch (error) {
      return { status: "failed", message: formatError(error) };
    }
  }

  return { removeTranslation, deleteResultAndRestart };
}
