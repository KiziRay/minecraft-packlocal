/**
 * 設定視窗「資料與備份」（B5a-2，規格 §1.3）：翻完刪除翻譯結果、套用前要不要備份、
 * 刪除目前遊戲資料夾的全部備份（D-07）、本地模型檔案（D-12）、工具資料位置與搬移。
 *
 * 需要知道「主視窗現在在翻譯嗎、選的是哪個遊戲資料夾」：主視窗用 mcpl:main-state 回報
 * （flow/main-state.js）。還沒收到回報時當成忙碌中，三個動作都先停用（失效安全）。
 * 停用一律 aria-disabled＋同列原因（R-4），按下去不做事。
 */
import { opDelete, opSet } from "../core/settings-patch.js";
import { SETTINGS_UPDATED_EVENT } from "../core/settings-sync.js";
import {
  DATA_MIGRATING_EVENT,
  MAIN_STATE_EVENT,
  SETTINGS_NOTICE_EVENTS,
  createStateRequester,
} from "../flow/main-state.js";
import {
  ROW_COPY,
  SETTINGS_DIALOGS,
  backupChoiceValue,
  backupCurrentLine,
  dataActionLocks,
  deleteBackupsDialog,
  deleteModelDialog,
  deleteResultsStep,
  formatGb,
  migrateResultLine,
} from "./settings-copy.js";

const BACKUP_CHOICE_PATH = "translate.backupChoice";
const DELETE_RESULTS_PATH = "translate.deleteResultsAfterApply";
const DELETE_RESULTS_ACK_PATH = "translate.deleteResultsAck";
const DELETE_RESULTS_KEY = "mcpl-delete-results-after-apply";
const DELETE_RESULTS_ACK_KEY = "mcpl-delete-results-ack";

let deps = null;
let mainState = null;
let model = { installed: false, sizeBytes: 0, installDir: "" };
let dataRoot = "";
let migrating = false;

const isOn = (value) => value === "1" || value === 1 || value === true || value === "true";

function readTranslate(name) {
  const settings = deps.settingsSnapshot();
  return settings && settings.translate ? settings.translate[name] : undefined;
}

function setLocked(buttonId, reasonId, lock) {
  const { $ } = deps;
  const button = $(buttonId);
  const reason = $(reasonId);
  if (button) button.setAttribute("aria-disabled", lock.locked ? "true" : "false");
  if (reason) {
    reason.textContent = lock.reason || "";
    reason.hidden = !lock.reason;
  }
}

function isLocked(buttonId) {
  return deps.$(buttonId)?.getAttribute("aria-disabled") === "true";
}

function renderLocks() {
  const locks = dataActionLocks(mainState, { modelInstalled: model.installed });
  setLocked("delete-backups", "delete-backups-reason", locks.deleteBackups);
  setLocked("delete-local-model", "delete-local-model-reason", locks.deleteModel);
  // 搬移那一列同時放停用原因與搬移結果：解除停用時只清掉原因，不清上一次的結果
  const button = deps.$("migrate-data-root");
  const result = deps.$("migrate-data-root-result");
  if (button) button.setAttribute("aria-disabled", locks.migrate.locked || migrating ? "true" : "false");
  if (!result) return;
  if (locks.migrate.locked && result.dataset.kind !== "running") {
    result.textContent = locks.migrate.reason;
    result.dataset.kind = "reason";
  } else if (!locks.migrate.locked && result.dataset.kind === "reason") {
    result.textContent = "";
    result.dataset.kind = "";
  }
}

function renderBackupChoice() {
  const { $ } = deps;
  const value = backupChoiceValue(deps.settingsSnapshot());
  $("backup-choice").value = value;
  $("backup-choice-label").textContent = backupCurrentLine(value);
}

function renderDeleteResults() {
  const { $ } = deps;
  $("delete-results-after-apply").checked = isOn(readTranslate("deleteResultsAfterApply"));
  $("delete-results-ack-row").hidden = true;
  $("delete-results-ack").checked = false;
}

async function saveAndAnnounce(ops, updates, message) {
  await deps.patchSettings(ops);
  for (const [path, value, localKey] of updates) {
    if (localKey) {
      try {
        if (value === null) localStorage.removeItem(localKey);
        else localStorage.setItem(localKey, String(value));
      } catch (_) {
        /* localStorage 壞掉不影響設定檔 */
      }
    }
    // 主視窗靠這個事件同步快取（開始翻譯時讀這個值，G0.1 合併寫入後才通知）
    await deps.emit(SETTINGS_UPDATED_EVENT, { path, value });
  }
  if (message) deps.setStatus(message, "ok");
}

async function onBackupChoiceChange(next) {
  const { confirmDialog, setStatus } = deps;
  const previous = backupChoiceValue(deps.settingsSnapshot());
  if (next === previous) return renderBackupChoice();
  if (next === "never" && !(await confirmDialog({ ...SETTINGS_DIALOGS.noBackup }))) {
    renderBackupChoice();
    return setStatus("沒有改動：套用前要不要備份維持原本的選擇。", "ok");
  }
  try {
    // 「每次詢問」＝明確刪除這個設定，不是寫空值
    const op = next === "ask" ? opDelete(BACKUP_CHOICE_PATH) : opSet(BACKUP_CHOICE_PATH, next);
    await saveAndAnnounce([op], [[BACKUP_CHOICE_PATH, next === "ask" ? null : next]], "已更新套用前要不要備份。");
  } catch (error) {
    setStatus(`沒有儲存成功：${String(error)}`, "warn");
  }
  renderBackupChoice();
}

async function onDeleteResultsToggle(wantOn) {
  const { $, setStatus } = deps;
  const step = deleteResultsStep({ wantOn, acked: isOn(readTranslate("deleteResultsAck")) });
  $("delete-results-ack-row").hidden = !step.showAck;
  if (step.showAck) {
    $("delete-results-ack").checked = false;
    $("delete-results-ack").focus();
    return setStatus("請再勾一次「我了解」，這個設定才會生效。", "warn");
  }
  try {
    await saveAndAnnounce(
      [opSet(DELETE_RESULTS_PATH, step.save)],
      [[DELETE_RESULTS_PATH, step.save, DELETE_RESULTS_KEY]],
      step.save === "1" ? "之後翻完、套用好就會刪掉這次的翻譯結果。" : "之後翻完會留著翻譯結果。"
    );
  } catch (error) {
    $("delete-results-after-apply").checked = !wantOn;
    setStatus(`沒有儲存成功：${String(error)}`, "warn");
  }
}

async function onDeleteResultsAck(checked) {
  const { $, setStatus } = deps;
  if (!checked) return;
  try {
    await saveAndAnnounce(
      [opSet(DELETE_RESULTS_ACK_PATH, "1"), opSet(DELETE_RESULTS_PATH, "1")],
      [
        [DELETE_RESULTS_ACK_PATH, "1", DELETE_RESULTS_ACK_KEY],
        [DELETE_RESULTS_PATH, "1", DELETE_RESULTS_KEY],
      ],
      "之後翻完、套用好就會刪掉這次的翻譯結果。"
    );
    $("delete-results-ack-row").hidden = true;
  } catch (error) {
    $("delete-results-ack").checked = false;
    setStatus(`沒有儲存成功：${String(error)}`, "warn");
  }
}

async function onDeleteBackups() {
  const { invoke, emit, confirmDialog, setStatus } = deps;
  if (isLocked("delete-backups")) return;
  const instancePath = String(mainState?.instancePath || "");
  let location = "";
  try {
    location = String((await invoke("apply_backup_location_cmd", { instancePath })) || "");
  } catch (_) {
    /* 查不到位置仍可刪；對話框只是少列一行路徑 */
  }
  if (!(await confirmDialog(deleteBackupsDialog({ location })))) return;
  // 對話框開著時主視窗可能開始翻譯了：再看一次
  if (isLocked("delete-backups")) return setStatus("正在翻譯，翻完才能刪除備份。", "warn");
  try {
    const result = await invoke("delete_apply_backups_cmd", {
      instancePath,
      outputDir: mainState?.outputDir || null,
    });
    const summary = String(result?.playerSummary || result?.player_summary || "備份刪除完成。");
    setStatus(summary, result?.failed?.length ? "warn" : "ok");
    await emit(SETTINGS_NOTICE_EVENTS.backupsDeleted, { summary });
  } catch (error) {
    setStatus(`刪除備份失敗：${String(error)}`, "warn");
  }
}

async function refreshLocalModel() {
  const { $, invoke } = deps;
  try {
    const status = await invoke("local_llm_status_cmd");
    model = {
      installed: !!status?.installed,
      sizeBytes: Number(status?.sizeBytes) || 0,
      installDir: String(status?.installDir || ""),
    };
  } catch (_) {
    model = { installed: false, sizeBytes: 0, installDir: "" };
  }
  const size = formatGb(model.sizeBytes);
  $("local-model-size").textContent = model.installed ? (size ? `約 ${size}` : "已下載") : ROW_COPY.modelNone;
  $("local-model-dir").textContent = model.installDir || "（尚未設定）";
  renderLocks();
}

async function onDeleteModel() {
  const { invoke, emit, confirmDialog, setStatus } = deps;
  if (isLocked("delete-local-model")) return;
  if (!(await confirmDialog(deleteModelDialog({ sizeBytes: model.sizeBytes, dir: model.installDir })))) return;
  try {
    const message = await invoke("local_llm_delete_cmd", { installDir: model.installDir || null });
    setStatus(String(message || "已刪除本地模型檔案。"), "ok");
    await emit(SETTINGS_NOTICE_EVENTS.localModelDeleted, {});
  } catch (error) {
    setStatus(`刪除失敗：${String(error)}`, "warn");
  }
  await refreshLocalModel();
}

async function refreshDataRoot() {
  const { $, invoke } = deps;
  const info = await invoke("data_root_info_cmd");
  dataRoot = String(info?.activeRoot || "");
  $("data-root").textContent = dataRoot || "（尚未建立）";
  $("data-root-hint").textContent = info?.usingPortable
    ? "目前資料跟著工具放，不佔用系統碟。"
    : `目前資料放在這台電腦的使用者資料夾。${ROW_COPY.migrateHelp}`;
  $("migrate-data-root").hidden = !info?.canMigrate;
}

/** 搬移可逆（原本的資料留著）→ 不問，按下即搬，同列顯示結果（規格 §3.5「改為自動」）。 */
async function onMigrate() {
  const { $, invoke } = deps;
  if (isLocked("migrate-data-root")) return;
  const result = $("migrate-data-root-result");
  const button = $("migrate-data-root");
  migrating = true;
  button.setAttribute("aria-disabled", "true");
  result.dataset.kind = "running";
  result.textContent = "正在搬移…";
  // 搬移期間主視窗不能開始翻譯（審查 F10）
  await Promise.resolve(deps.emit(DATA_MIGRATING_EVENT, { active: true })).catch(() => {});
  try {
    const done = await invoke("migrate_data_root_cmd");
    result.textContent = migrateResultLine(done);
    await refreshDataRoot();
  } catch (error) {
    result.textContent = `沒有搬成，原本的資料沒有被動到：${String(error)}`;
  } finally {
    await Promise.resolve(deps.emit(DATA_MIGRATING_EVENT, { active: false })).catch(() => {});
    migrating = false;
    result.dataset.kind = "result";
    renderLocks();
  }
}

async function copyPath(targetId) {
  const { $, setStatus } = deps;
  const text = String($(targetId)?.textContent || "").trim();
  if (!text || text.startsWith("（") || text === "尚未選擇" || text === "讀取中…") {
    return setStatus("還沒有路徑可以複製。", "warn");
  }
  try {
    await navigator.clipboard.writeText(text);
    setStatus("已複製路徑。", "ok");
  } catch (_) {
    // 剪貼簿不給用：把路徑選起來，讓玩家自己按 Ctrl+C
    const range = document.createRange();
    range.selectNodeContents($(targetId));
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    setStatus("無法直接複製，已把路徑選起來，請按 Ctrl+C。", "warn");
  }
}

export function wireDataPane(injected) {
  deps = injected;
  const { $, listen, emit, invoke, setStatus } = deps;
  $("backup-choice").addEventListener("change", (event) => void onBackupChoiceChange(event.target.value));
  $("delete-results-after-apply").addEventListener("change", (event) => void onDeleteResultsToggle(!!event.target.checked));
  $("delete-results-ack").addEventListener("change", (event) => void onDeleteResultsAck(!!event.target.checked));
  $("delete-backups").addEventListener("click", () => void onDeleteBackups());
  $("delete-local-model").addEventListener("click", () => void onDeleteModel());
  $("migrate-data-root").addEventListener("click", () => void onMigrate());
  $("open-data-root").addEventListener("click", () => {
    void invoke("open_path", { path: dataRoot }).catch((error) => setStatus(`無法開啟資料夾：${String(error)}`, "warn"));
  });
  for (const button of document.querySelectorAll("[data-copy]")) {
    button.addEventListener("click", () => void copyPath(button.dataset.copy));
  }
  void listen(MAIN_STATE_EVENT, (event) => {
    mainState = event?.payload && typeof event.payload === "object" ? event.payload : null;
    renderLocks();
  });
  renderLocks();
  // 設定視窗後開：主動請主視窗回報；收不到就每 2 秒重送，最多 5 次（審查 F3）
  createStateRequester({ emit, hasState: () => mainState !== null }).start();
}

export async function refreshDataPane() {
  renderBackupChoice();
  renderDeleteResults();
  await Promise.all([refreshDataRoot().catch(() => null), refreshLocalModel()]);
}
