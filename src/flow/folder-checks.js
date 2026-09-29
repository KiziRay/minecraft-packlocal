/**
 * B5d 選資料夾就判定的接線（app.js 只留呼叫）：瀏覽、手動輸入、「上次」共用同一套檢查。
 *
 * - inspect(path)：inspect_folder_cmd（驗證、形狀、寫入；唯讀、背景、≤3 秒）→ 通過才在背景查身分
 *   （inspect_instance_identity_cmd）與遊戲是否開著（is_game_running_cmd），查到就重畫狀態卡。
 * - 狀態卡動作：改用上一層／候選、重新檢查、以管理員重開、重設紀錄、開啟記號位置、當成新的、仍要翻伺服器資料夾。
 * - 橫幅 N-03、N-04：只在可開始／有變動的狀態，每次選資料夾最多一次（關掉或狀態離開後不再出現）。
 * - D 區「上次：<包名>」：啟動時只讀路徑與包名，按下才走 inspect（百分比與狀態按下後才算）。
 *
 * app.js 的狀態由 deps 注入（getter／callback），這裡不 import app.js。
 */
import { FOLDER_ACTION, folderBanners, lastInstanceButton, pickStartPath, samePath } from "./folder-state.js";
import { offerForkInstance } from "../ui/apply-pending.js";
import { parentFolder } from "../ui/banner.js";

const HANDLED = new Set(Object.values(FOLDER_ACTION));

function errorText(e) {
  return String((e && (e.message || e)) || "").trim();
}

export function createFolderChecks(deps) {
  const { $, invoke } = deps;
  let current = { path: "", inspecting: false, inspection: null, identityPending: false, identity: null };
  let token = 0;
  let gameRunning = false;
  /** 這次選資料夾已關掉或已收掉的橫幅（每次選資料夾最多出現一次）。 */
  let bannersDone = [];
  const serverOverrides = [];
  let launcherDir = null;

  const log = (text, level) => deps.appendLog && deps.appendLog(text, level);

  function reset(path = "") {
    token += 1;
    current = { path, inspecting: false, inspection: null, identityPending: false, identity: null };
    gameRunning = false;
    bannersDone = [];
  }

  /** 選資料夾當下的檢查。回 { ok, validation, inspection, stale }。 */
  async function inspect(path) {
    const target = String(path || "").trim();
    reset(target);
    const my = token;
    current.inspecting = true;
    deps.syncUiState();
    let inspection;
    try {
      inspection = await invoke("inspect_folder_cmd", { instancePath: target });
    } catch (e) {
      const reason = errorText(e) || "這個資料夾檢查不了，請重新選擇";
      inspection = { reachable: true, validation: { ok: false, reason }, shape: { kind: "invalid" }, write: null };
    }
    if (my !== token) return { ok: false, stale: true, inspection, validation: inspection?.validation || {} };
    const validation = (inspection && inspection.validation) || { ok: false, reason: "" };
    const reachable = !inspection || inspection.reachable !== false;
    const ok = reachable && !!validation.ok;
    const writable = !(inspection && inspection.write && inspection.write.writable === false);
    current = { path: target, inspecting: false, inspection, identityPending: ok && writable, identity: null };
    if (!writable && inspection.write.message) log(inspection.write.message, "warn");
    deps.syncUiState();
    if (ok && writable) {
      invoke("inspect_instance_identity_cmd", { instancePath: target })
        .then((identity) => {
          if (my === token) current.identity = identity || null;
        })
        .catch(() => {
          /* 判斷不了＝照舊（套用前的檢查仍在） */
        })
        .finally(() => {
          if (my !== token) return;
          current.identityPending = false;
          deps.syncUiState();
        });
    }
    if (ok) {
      invoke("is_game_running_cmd", { instancePath: target })
        .then((verdict) => {
          if (my !== token) return;
          gameRunning = !!(verdict && verdict.running);
          if (gameRunning) deps.syncUiState();
        })
        .catch(() => {});
    }
    return { ok, stale: false, inspection, validation };
  }

  /** 給 computePackState 的輸入；路徑對不上（例如還在打字）時不給，交給一般驗證。 */
  function gateInput(path) {
    if (!current.path || !samePath(current.path, path)) return null;
    return {
      inspecting: current.inspecting,
      inspection: current.inspection,
      identityPending: current.identityPending,
      identity: current.identity,
      serverOverride: serverOverrides.some((p) => samePath(p, path)),
    };
  }

  /** 有沒有 options.txt；不知道回 null（失效安全：照舊提醒）。 */
  function hasOptions(path) {
    const i = current.inspection;
    if (!i || !samePath(current.path, path) || !i.validation || !i.validation.ok) return null;
    return i.shape && typeof i.shape.hasOptions === "boolean" ? i.shape.hasOptions : null;
  }

  function handles(action) {
    return HANDLED.has(action);
  }

  async function onAction(action, item = {}) {
    const path = current.path || deps.getCurrentPath();
    if (action === FOLDER_ACTION.usePath && item.path) return void (await deps.adoptInstancePath(item.path));
    if (action === FOLDER_ACTION.recheck) return void (path && (await deps.adoptInstancePath(path)));
    if (action === FOLDER_ACTION.serverOverride) {
      if (path) serverOverrides.push(path);
      deps.disclosure.retire("server");
      return void deps.syncUiState();
    }
    if (action === FOLDER_ACTION.relaunchAdmin) {
      try {
        const out = await invoke("relaunch_as_admin_cmd", { instancePath: path });
        // UAC 被取消不是錯誤——他只是不想提權
        if (!out || !out.relaunching) log("已取消以系統管理員身分開啟。也可以改選放在自己資料夾底下的模組整合包。");
      } catch (e) {
        log("無法以系統管理員身分重新開啟：" + errorText(e), "warn");
      }
      return;
    }
    if (action === FOLDER_ACTION.resetRecord) {
      // 可逆（改名保留）→ 按鈕本身就是動作，不另跳確認（規格 §3.5）
      try {
        log(String((await invoke("reset_apply_record_cmd", { instancePath: path })) || "已重設套用紀錄。"));
        deps.disclosure.retire("brokenRecord");
      } catch (e) {
        log("重設套用紀錄失敗：" + errorText(e), "error");
      }
      return void (path && (await deps.adoptInstancePath(path)));
    }
    if (action === FOLDER_ACTION.openMarker) {
      const where = parentFolder(item.path || (current.identity && current.identity.path) || "");
      if (!where) return;
      try {
        await invoke("open_path", { path: where });
      } catch (e) {
        log("無法開啟資料夾：" + errorText(e), "warn");
      }
      return;
    }
    if (action === FOLDER_ACTION.forkInstance) {
      // D-10（不可逆：不再沿用舊紀錄）
      const done = await offerForkInstance(
        { confirmDialog: deps.confirmDialog, invoke, appendLog: (text, level) => log(text, level) },
        path
      );
      if (done) {
        deps.disclosure.retire("copied");
        await deps.adoptInstancePath(path);
      }
    }
  }

  /** N-03、N-04。bannerArea 由 pack-actions 管（同一個 B 區）。 */
  function syncBanners(stateId, path) {
    const area = deps.bannerArea && deps.bannerArea();
    if (!area) return;
    const decision = folderBanners({
      stateId,
      gameRunning: gameRunning && samePath(current.path, path),
      hasOptions: hasOptions(path),
      dismissed: bannersDone,
    });
    for (const id of decision.hide) {
      if (area.has(id)) {
        area.hide(id);
        bannersDone.push(id); // 收掉後這次選資料夾不再出現
      }
    }
    for (const banner of decision.show) if (!area.has(banner.id)) area.show(banner);
  }

  function noteBannerDismissed(banner) {
    if (banner && (banner.id === "N-03" || banner.id === "N-04")) bannersDone.push(banner.id);
  }

  /** D 區「上次：<包名>」。 */
  function syncLastButton({ locked = false, lockReason = "" } = {}) {
    const btn = $("btn-last-instance");
    if (!btn) return;
    const view = lastInstanceButton({
      lastPath: deps.readLastInstancePath(),
      currentPath: deps.getCurrentPath(),
      locked,
      lockReason,
    });
    btn.hidden = !view.visible;
    btn.textContent = view.label;
    btn.dataset.path = view.path;
    btn.setAttribute("aria-disabled", view.disabledReason ? "true" : "false");
    if (view.disabledReason) btn.setAttribute("aria-describedby", "folder-lock-reason");
    else btn.removeAttribute("aria-describedby");
  }

  async function useLast() {
    const btn = $("btn-last-instance");
    const path = (btn && btn.dataset.path) || deps.readLastInstancePath();
    if (!path) return;
    await deps.adoptInstancePath(path);
  }

  /** 瀏覽視窗的起始位置：上次路徑，沒有時用偵測到的常見啟動器資料夾（只查一次）。 */
  async function pickStart() {
    const last = deps.readLastInstancePath();
    if (last) return last;
    if (launcherDir === null) {
      try {
        launcherDir = String((await invoke("common_launcher_dir_cmd")) || "");
      } catch (_) {
        launcherDir = "";
      }
    }
    return pickStartPath({ lastPath: last, launcherDir });
  }

  /** §3.1 MC 版本列：狀態卡上的下拉，選了就寫回本包選項的 #target-version。 */
  function wireVersionRow({ onPicked } = {}) {
    const row = $("status-card-version");
    const source = $("target-version");
    if (!row || !source) return;
    if (!row.options.length) {
      row.appendChild(new Option("選擇版本", ""));
      for (const option of Array.from(source.options)) if (option.value) row.appendChild(new Option(option.text, option.value));
    }
    row.addEventListener("change", () => {
      if (!row.value) return;
      if (!Array.from(source.options).some((o) => o.value === row.value)) source.add(new Option(row.value, row.value));
      source.value = row.value;
      source.dataset.autoDetected = "false";
      source.dispatchEvent(new Event("change", { bubbles: true }));
      if (typeof onPicked === "function") onPicked(row.value);
      deps.syncUiState();
    });
  }

  return {
    inspect,
    reset,
    gateInput,
    hasOptions,
    handles,
    onAction,
    syncBanners,
    noteBannerDismissed,
    syncLastButton,
    useLast,
    pickStart,
    wireVersionRow,
  };
}
/**
 * 審查 2：選資料夾通過後的步驟（偵測版本、結果位置、探測）依序跑；每個 await 之後確認使用者沒換資料夾，
 * 換了就停，舊資料夾的結果不寫到畫面上。步驟回傳一個函式＝「還是同一個資料夾才套用」的部分。
 * 回 true＝全部跑完；false＝中途換了資料夾。
 */
export async function runWhileCurrent(path, getCurrentPath, steps) {
  for (const step of steps) {
    if (!samePath(getCurrentPath(), path)) return false;
    const apply = await step();
    if (!samePath(getCurrentPath(), path)) return false;
    if (typeof apply === "function") apply();
  }
  return samePath(getCurrentPath(), path);
}
