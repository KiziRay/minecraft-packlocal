/**
 * 「翻譯已完成、還沒裝進遊戲」的處理（B1 最小對接；畫面在 B8 重做）。
 *
 * 後端在寫入遊戲前會先檢查：遊戲開著、還沒啟動過遊戲、第一次套用還沒選備份、
 * 選了不備份而這次會蓋掉原檔。任何一項不過，後端一個檔都不動，回傳狀態；
 * 這裡依狀態問玩家（備份選擇、覆蓋確認），或顯示說明卡與「套用到遊戲」按鈕。
 */

export const APPLY_STATUS = Object.freeze({
  applied: "applied",
  gameRunning: "gameRunning",
  noOptionsTxt: "noOptionsTxt",
  needsBackupChoice: "needsBackupChoice",
  needsOverwriteConfirm: "needsOverwriteConfirm",
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

/** 說明卡的標題與內文（玩家看得懂的話，不出現技術名詞）。 */
export function describeApplyPending(result) {
  const status = applyStatusOf(result);
  const message = pendingMessageOf(result);
  switch (status) {
    case APPLY_STATUS.gameRunning:
      return { title: "已翻完，關掉遊戲後按「套用到遊戲」", message };
    case APPLY_STATUS.noOptionsTxt:
      return { title: "請先啟動一次遊戲", message };
    case APPLY_STATUS.needsBackupChoice:
      return { title: "已翻完，還沒裝進遊戲", message: message || "第一次裝進遊戲前，要先決定要不要備份。" };
    case APPLY_STATUS.needsOverwriteConfirm:
      return { title: "已翻完，還沒裝進遊戲", message };
    default:
      return { title: "", message: "" };
  }
}

/** 覆蓋確認清單最多列幾個，其餘用一行帶過。 */
export function previewList(items, max = 8) {
  const list = Array.isArray(items) ? items : [];
  if (list.length <= max) return list.slice();
  return [...list.slice(0, max), `…另有 ${list.length - max} 個檔案`];
}

/**
 * @param {{
 *   $: (id: string) => HTMLElement | null,
 *   invoke: Function,
 *   confirmDialog: Function,
 *   choiceDialog: Function,
 *   appendLog: (text: string, level?: string) => void,
 *   setBusy: (busy: boolean, kind?: string) => void,
 *   saveBackupChoice: (value: "always" | "never") => Promise<void>,
 *   onApplied?: (result: object) => void | Promise<void>,
 * }} deps
 */
export function createApplyPendingFlow(deps) {
  let lastContext = null;

  function showCard(result) {
    const card = deps.$("apply-pending-card");
    if (!card) return;
    const { title, message } = describeApplyPending(result);
    const titleEl = deps.$("apply-pending-title");
    const messageEl = deps.$("apply-pending-message");
    if (titleEl) titleEl.textContent = title;
    if (messageEl) messageEl.textContent = message;
    card.hidden = false;
  }

  function hideCard() {
    const card = deps.$("apply-pending-card");
    if (card) card.hidden = true;
  }

  async function askBackupChoice() {
    const choice = await deps.choiceDialog({
      title: "要先備份遊戲裡會被覆蓋的檔案嗎？",
      body: "只會問這一次，之後都照你的選擇。想改可以到設定的「資料與備份」。",
      options: [
        {
          value: "always",
          label: "要備份（建議）",
          detail: "覆蓋前先把遊戲原本的檔案收好。之後按「移除翻譯」可以完整回到原版。",
        },
        {
          value: "never",
          label: "不備份",
          detail: "不另存原本的檔案。之後移除翻譯時，被覆蓋的檔案無法還原；工具加的檔案仍然可以拿掉。",
        },
      ],
      cancelLabel: "先不要裝進遊戲",
    });
    if (choice !== "always" && choice !== "never") return null;
    if (choice === "never") {
      const sure = await deps.confirmDialog({
        title: "確定不備份嗎？",
        body: "之後每次要覆蓋遊戲裡原本的檔案前，都會再問你一次。",
        danger: true,
        ackLabel: NO_BACKUP_ACK,
        confirmLabel: "不備份，繼續",
        cancelLabel: "回去",
      });
      if (!sure) return null;
    }
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
      confirmLabel: "覆蓋並裝進遊戲",
      cancelLabel: "先不要",
    });
  }

  async function applyNow(context, { overwriteConfirmed = false } = {}, depth = 0) {
    deps.setBusy(true, "apply");
    let result;
    try {
      result = await deps.invoke("apply_translation_to_game", {
        instancePath: context.instancePath,
        outputDir: context.outputDir,
        packName: context.packName || null,
        overwriteConfirmed,
      });
    } catch (error) {
      deps.appendLog("套用到遊戲失敗：" + String(error?.message || error), "error");
      if (isBrokenRecordError(error)) {
        deps.setBusy(false);
        await offerRecordReset(deps, context.instancePath);
        return false;
      }
      if (isForkableError(error)) {
        deps.setBusy(false);
        await offerForkInstance(deps, context.instancePath);
        return false;
      }
      showCard({ applyStatus: APPLY_STATUS.gameRunning, applyMessage: "套用沒有完成，請確認遊戲已關閉後再按一次「套用到遊戲」。" });
      return false;
    } finally {
      deps.setBusy(false);
    }
    return handle(result, context, depth + 1);
  }

  /** 翻譯／補翻／修復／單獨套用結束後呼叫。回傳 true＝已經裝進遊戲。 */
  async function handle(result, context, depth = 0) {
    if (context) lastContext = context;
    const status = applyStatusOf(result);
    if (status === APPLY_STATUS.applied) {
      hideCard();
      if (depth > 0) {
        const summary = pendingMessageOf(result);
        if (summary) deps.appendLog(summary);
        if (typeof deps.onApplied === "function") await deps.onApplied(result);
      }
      return true;
    }
    if (depth > 3 || !lastContext) {
      showCard(result);
      return false;
    }
    if (status === APPLY_STATUS.needsBackupChoice) {
      const choice = await askBackupChoice();
      if (!choice) {
        showCard(result);
        return false;
      }
      // 剛在對話框勾過「無法還原」，這次不再重複問覆蓋
      return applyNow(lastContext, { overwriteConfirmed: choice === "never" }, depth);
    }
    if (status === APPLY_STATUS.needsOverwriteConfirm) {
      if (!(await confirmOverwrite(result))) {
        showCard(result);
        return false;
      }
      return applyNow(lastContext, { overwriteConfirmed: true }, depth);
    }
    deps.appendLog(pendingMessageOf(result), "warn");
    showCard(result);
    return false;
  }

  function wire() {
    const button = deps.$("btn-apply-pending");
    if (button) {
      button.onclick = () => {
        if (!lastContext) return;
        void applyNow(lastContext);
      };
    }
    const dismiss = deps.$("btn-apply-pending-dismiss");
    if (dismiss) dismiss.onclick = () => hideCard();
  }

  return { handle, wire, hideCard, isPending: isApplyPending };
}

/** 開始翻譯時「保留翻譯結果」選項的說明：依目前備份設定講正確的話。 */
export function keepOptionDetail(choice) {
  const tail = "翻譯結果也留著，之後可以「分享給其他玩家」或用「補充漏翻」接續。";
  if (choice === "always") return "裝進遊戲前會先備份遊戲原本的檔案；" + tail;
  if (choice === "never") return "依你的設定不備份遊戲原本的檔案（覆蓋前會再問你一次）；" + tail;
  return "第一次裝進遊戲前會問你要不要備份遊戲原本的檔案；" + tail;
}

/** 「不保留翻譯結果」選項的說明：只管結果資料夾，備份仍照設定。 */
export function skipOptionDetail(choice) {
  const head = "裝進遊戲後刪掉這次的翻譯結果，之後不能分享給其他玩家，也不能接續補翻。";
  if (choice === "always") return head + "遊戲原本的檔案仍會先備份，之後按「移除翻譯」可以回到原版。";
  if (choice === "never") return head + "依你的設定不備份遊戲原本的檔案，覆蓋前會再問你一次。";
  return head + "要不要備份遊戲原本的檔案，第一次裝進遊戲前會問你。";
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
      "工具記錄「裝進遊戲的哪些檔案」的清單壞掉了。重設後會重新開始記錄，壞掉的那份會改名保留、不會刪除。\n" +
      "重設之前裝進遊戲的翻譯檔，之後可能無法用「移除翻譯」自動拿掉；有備份的原檔仍在備份資料夾裡。",
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
  return text.includes("把這份當成新的整合包");
}

/**
 * 給複製出來的遊戲資料夾一個出口：確認後請後端替它建立新的識別碼與紀錄（另一份不受影響）。
 * 回傳 true＝已建立。
 */
export async function offerForkInstance({ confirmDialog, invoke, appendLog }, instancePath) {
  if (!instancePath) return false;
  const ok = await confirmDialog({
    title: "把這份當成新的整合包？",
    body:
      "這個遊戲資料夾是從另一個遊戲資料夾複製來的。當成新的整合包後，工具會替它建立自己的紀錄，" +
      "另一份的紀錄與備份不受影響。\n" +
      "從另一份複製過來的翻譯檔無法確定原本是什麼，之後套用時會先保存到隔離區再換成新的翻譯。",
    affected: [instancePath],
    confirmLabel: "當成新的整合包",
    cancelLabel: "先不要",
  });
  if (!ok) return false;
  try {
    const message = await invoke("fork_apply_instance_cmd", { instancePath });
    appendLog(String(message || "已把這份當成新的整合包。"));
    return true;
  } catch (error) {
    appendLog("建立新的整合包紀錄失敗：" + String(error?.message || error), "error");
    return false;
  }
}
