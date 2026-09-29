/**
 * 「翻譯已完成、還沒套用到遊戲」的處理（B1 對接；B5c 畫面併入狀態卡 S11）。
 */

export const APPLY_STATUS = Object.freeze({
  applied: "applied",
  gameRunning: "gameRunning",
  noOptionsTxt: "noOptionsTxt",
  needsBackupChoice: "needsBackupChoice",
  needsOverwriteConfirm: "needsOverwriteConfirm",
  forkNeeded: "forkNeeded",
});

export const BACKUP_CHOICE_PATH = "translate.backupChoice";
export const NO_BACKUP_ACK = "我了解之後無法還原被覆蓋的檔案";

/** 翻譯結果（applyStatus）與單獨套用結果（status）兩種形狀都認得；沒有欄位＝舊版後端，視為已套用。 */
export function applyStatusOf(result) {
  const raw = result && (result.applyStatus ?? result.apply_status ?? result.status);
  return typeof raw === "string" && raw ? raw : APPLY_STATUS.applied;
}

export function isApplyPending(result) {
  return applyStatusOf(result) !== APPLY_STATUS.applied;
}

export function pendingMessageOf(result) {
  return String(
    (result && (result.applyMessage || result.apply_message || result.playerSummary || result.player_summary)) || ""
  );
}

export function pendingOverwritesOf(result) {
  const list = result && (result.pendingOverwrites || result.pending_overwrites);
  return Array.isArray(list) ? list.map(String) : [];
}

/** 覆蓋確認清單最多列幾個，其餘用一行帶過。 */
export function previewList(items, max = 8) {
  const list = Array.isArray(items) ? items : [];
  if (list.length <= max) return list.slice();
  return [...list.slice(0, max), `…另有 ${list.length - max} 個檔案`];
}

/**
 * 套用失敗的白話原因（S11 失敗變體「套用沒有完成：<原因>」；≤20 字）。完整錯誤照舊寫進紀錄。
 */
export function applyFailureReason(error) {
  const t = String((error && (error.message || error)) || "");
  // 審查 1：後端歸屬比對拒絕（結果屬於別的模組整合包）
  const owner = t.match(/屬於「([^」]+)」，不是「/);
  if (owner) return `這份結果屬於「${Array.from(owner[1]).slice(0, 12).join("")}」`;
  if (/os error (53|67|64|59|1231)|網路路徑|網路磁碟|找不到遊戲資料夾|連不到|network/i.test(t)) return "網路磁碟或遊戲資料夾連不上";
  if (/os error (32|33)|被另一個程序|占用|佔用|being used|locked/i.test(t)) return "有檔案被占用，請先關閉遊戲";
  if (/os error 5|存取被拒|拒絕存取|Access is denied|permission/i.test(t)) return "沒有權限寫入遊戲資料夾";
  if (/空間不足|os error 112|No space/i.test(t)) return "磁碟空間不夠";
  return "發生錯誤，完整原因在紀錄";
}

/**
 * 「翻譯已完成、還沒套用」的處理（B5c：待套用卡併入狀態卡 S11，這裡不碰 DOM）。
 *
 * 後端在寫入遊戲前會先檢查：遊戲開著、還沒啟動過遊戲、第一次套用還沒選備份、選了不備份而這次會蓋掉原檔、
 * 整份複製來的資料夾。任何一項不過，後端一個檔都不動，回傳狀態。只有兩種確認會跳對話框（規格 §3.5）：
 * D-04（全工具第一次、未經 §3.1）與 D-03（不備份要覆蓋原檔）；其餘交給狀態卡（onPending）。
 *
 * @param {{
 *   invoke: Function, confirmDialog: Function, choiceDialog: Function,
 *   appendLog: (text: string, level?: string) => void,
 *   setBusy: (busy: boolean, kind?: string) => void,
 *   saveBackupChoice: (value: "always" | "never") => Promise<void>,
 *   onApplied?: (result: object, context: object) => void | Promise<void>,
 *   onPending?: (result: object, context: object) => void,
 *   onFailed?: (reason: string, context: object, error: unknown) => void,
 *   onBrokenRecord?: (context: object) => Promise<unknown>,
 * }} deps
 */
export function createApplyPendingFlow(deps) {
  let lastContext = null;
  const pending = (result, context) => {
    if (typeof deps.onPending === "function") deps.onPending(result, context || lastContext);
  };

  async function askBackupChoice() {
    // D-04（規格 §3.5）：覆蓋無備份（全工具第一次）；預設焦點「備份」；選不備份在同一個框勾選
    const choice = await deps.choiceDialog({
      title: "要先備份會被覆蓋的原檔嗎？",
      body: "只會問這一次，之後都照你的選擇。可到 設定→資料與備份 改。",
      options: [
        { value: "always", label: "先備份（建議）", detail: "之後按「移除翻譯」可以完整回到原版。" },
        { value: "never", label: "不備份", detail: "被覆蓋的原檔之後無法還原；工具加的檔仍可拿掉。" },
      ],
      ack: { label: NO_BACKUP_ACK, forValue: "never" },
      cancelLabel: "先不要套用",
    });
    if (choice !== "always" && choice !== "never") return null;
    await deps.saveBackupChoice(choice);
    return choice;
  }

  async function confirmOverwrite(result) {
    return deps.confirmDialog({
      title: "要覆蓋遊戲裡原本的檔案嗎？",
      body: pendingMessageOf(result),
      affected: previewList(pendingOverwritesOf(result)),
      danger: true,
      ackLabel: NO_BACKUP_ACK,
      confirmLabel: "覆蓋並套用",
      cancelLabel: "先不要",
    });
  }

  /** 按「套用到遊戲」（或自動接著套用）。回傳 true＝已經套用到遊戲。 */
  async function applyNow(context, { overwriteConfirmed = false } = {}, depth = 0) {
    lastContext = context || lastContext;
    if (!lastContext) return false;
    deps.setBusy(true, "apply");
    let result;
    try {
      result = await deps.invoke("apply_translation_to_game", {
        instancePath: lastContext.instancePath,
        outputDir: lastContext.outputDir,
        packName: lastContext.packName || null,
        overwriteConfirmed,
      });
    } catch (error) {
      deps.setBusy(false);
      deps.appendLog("套用到遊戲失敗：" + String(error?.message || error), "error");
      if (isBrokenRecordError(error) && typeof deps.onBrokenRecord === "function") {
        await deps.onBrokenRecord(lastContext);
        return false;
      }
      // 複製資料夾被擋：回「已翻完未套用」，狀態卡主要按鈕「當成新的模組整合包」（不是失敗）
      if (isForkableError(error)) {
        pending({ applyStatus: APPLY_STATUS.forkNeeded, applyMessage: String(error?.message || error) }, lastContext);
        return false;
      }
      if (typeof deps.onFailed === "function") deps.onFailed(applyFailureReason(error), lastContext, error);
      return false;
    }
    deps.setBusy(false);
    return handle(result, lastContext, depth + 1);
  }

  /**
   * 翻譯／接續補完／修復／單獨套用結束後呼叫。回傳 true＝已經套用到遊戲。
   * `overwriteConfirmed`（B5b 審查 4a）：這一輪已在開始前確認（§3.1 選不備份並勾「我了解」），套用不再跳 D-03。
   */
  async function handle(result, context, depth = 0, { overwriteConfirmed = false } = {}) {
    if (context) lastContext = context;
    const status = applyStatusOf(result);
    if (status === APPLY_STATUS.applied) {
      if (depth > 0) {
        const summary = pendingMessageOf(result);
        if (summary) deps.appendLog(summary);
        if (typeof deps.onApplied === "function") await deps.onApplied(result, lastContext);
      }
      return true;
    }
    if (depth > 3 || !lastContext) {
      pending(result);
      return false;
    }
    if (status === APPLY_STATUS.needsBackupChoice) {
      const choice = await askBackupChoice();
      if (!choice) {
        pending(result);
        return false;
      }
      // 剛在同一個框勾過「無法還原」，這次不再重複問覆蓋
      return applyNow(lastContext, { overwriteConfirmed: choice === "never" }, depth);
    }
    if (status === APPLY_STATUS.needsOverwriteConfirm) {
      if (overwriteConfirmed && depth === 0) return applyNow(lastContext, { overwriteConfirmed: true }, depth);
      if (!(await confirmOverwrite(result))) {
        pending(result);
        return false;
      }
      return applyNow(lastContext, { overwriteConfirmed: true }, depth);
    }
    deps.appendLog(pendingMessageOf(result), "warn");
    pending(result);
    return false;
  }

  return { handle, applyNow, isPending: isApplyPending };
}

/** 後端回報套用紀錄損壞的錯誤（後端訊息固定以「套用紀錄損壞」開頭）。 */
export function isBrokenRecordError(error) {
  const text = String((error && (error.message || error)) || "");
  return text.includes("套用紀錄損壞");
}

/**
 * 紀錄損壞時給玩家的出口：確認後請後端把壞掉的紀錄改名保留、重新開始記錄。
 * 回傳 true＝已重設。
 */
export async function offerRecordReset({ confirmDialog, invoke, appendLog }, instancePath) {
  if (!instancePath) return false;
  const ok = await confirmDialog({
    title: "重設套用紀錄？",
    body:
      "工具記錄「套用到遊戲的哪些檔案」的清單壞掉了。重設後會重新開始記錄，壞掉的那份會改名保留、不會刪除。\n" +
      "重設之前套用到遊戲的翻譯檔，之後可能無法用「移除翻譯」自動拿掉；有備份的原檔仍在備份資料夾裡。",
    affected: [instancePath],
    confirmLabel: "重設套用紀錄",
    cancelLabel: "先不要",
  });
  if (!ok) return false;
  try {
    const message = await invoke("reset_apply_record_cmd", { instancePath });
    appendLog(String(message || "已重設套用紀錄。"));
    return true;
  } catch (error) {
    appendLog("重設套用紀錄失敗：" + String(error?.message || error), "error");
    return false;
  }
}

/** 後端拒絕「整份複製出來」或「原位置連不到」的遊戲資料夾時，訊息會提到這個選項。 */
export function isForkableError(error) {
  const text = String((error && (error.message || error)) || "");
  return text.includes("當成新的");
}

/**
 * 給複製出來的遊戲資料夾一個出口：確認後請後端替它建立新的識別碼與紀錄（另一份不受影響）。
 * 回傳 true＝已建立。
 */
export async function offerForkInstance({ confirmDialog, invoke, appendLog }, instancePath) {
  if (!instancePath) return false;
  // D-10（規格 §3.5，B5d）：不可逆（不再沿用舊紀錄）、預設焦點「取消」；不刪東西，所以不是紅色危險外觀
  const ok = await confirmDialog({
    title: "把這份當成新的模組整合包？",
    body:
      "複製來的：兩份分開記錄，另一份的紀錄與備份不受影響。連不到的：確定舊位置已不在才選。" +
      "複製過來的翻譯檔之後套用時會先放進隔離區。",
    affected: [instancePath],
    confirmLabel: "當成新的",
    cancelLabel: "取消",
    initialFocus: "cancel",
  });
  if (!ok) return false;
  try {
    const message = await invoke("fork_apply_instance_cmd", { instancePath });
    appendLog(String(message || "已把這份當成新的模組整合包。"));
    return true;
  } catch (error) {
    appendLog("建立新的模組整合包紀錄失敗：" + String(error?.message || error), "error");
    return false;
  }
}
