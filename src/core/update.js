// ───────────────────────── 檢查更新（主視窗）─────────────────────────
// 啟動時自動檢查一次；有新版時跳更新視窗。手動「檢查更新」在設定視窗「關於」，
// 有新版時由設定視窗請主視窗呼叫 window.zfCheckUpdate() 顯示同一個更新視窗。
// 契約：invoke("check_update") → { current, latest, updateAvailable, url, notes, ok, message, testBuild }
//       invoke("download_update") → { path, launched, automatic, shouldExit, message }
import { describeUpdateCheck } from "./update-status.js";

export { describeUpdateCheck, TEST_BUILD_MESSAGE } from "./update-status.js";

export function wireUpdateChecker({ appendLog = null, isBusy = () => false } = {}) {
  const _invoke =
    (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) || null;
  let latestUpdateInfo = null;
  let updateInFlight = false;
  let pendingUpdateInfo = null;

  function parseUpdateNotes(notes) {
    const raw = String(notes || "").trim();
    if (!raw) return [];
    return raw
      .split(/[\n;；]+/)
      .map((s) => s.trim())
      .filter(Boolean);
  }

  function isLegacy102Client(version) {
    return String(version || "")
      .trim()
      .replace(/^v/i, "") === "1.0.2";
  }

  function showUpdateModal(info) {
    const overlay = document.getElementById("update-overlay");
    const line = document.getElementById("update-version-line");
    const list = document.getElementById("update-notes-list");
    const title = document.getElementById("update-title");
    const migration = document.getElementById("update-migration-hint");
    if (!overlay) return;
    if (title) title.textContent = "MCPL " + (info.latest || "");
    if (line) {
      line.textContent =
        "發現新版本 " + info.latest + "（目前 " + info.current + "）";
    }
    if (migration) {
      if (isLegacy102Client(info.current)) {
        migration.textContent =
          "您目前是 1.0.2：請按「手動下載」，關閉舊工具後開啟；之後版本才支援一鍵自動更新。";
        migration.hidden = false;
        migration.setAttribute("aria-hidden", "false");
      } else {
        migration.textContent = "";
        migration.hidden = true;
        migration.setAttribute("aria-hidden", "true");
      }
    }
    if (list) {
      list.innerHTML = "";
      const items = parseUpdateNotes(info.notes);
      if (items.length > 0) {
        list.hidden = false;
        items.forEach((item) => {
          const li = document.createElement("li");
          li.textContent = item;
          list.appendChild(li);
        });
      } else {
        list.hidden = true;
      }
    }
    overlay.hidden = false;
    overlay.setAttribute("aria-hidden", "false");
  }

  function hideUpdateModal() {
    const overlay = document.getElementById("update-overlay");
    if (!overlay) return;
    overlay.hidden = true;
    overlay.setAttribute("aria-hidden", "true");
  }

  function setPendingUpdateInfo(info) {
    if (info) pendingUpdateInfo = info;
  }

  // 給 setBusy 使用：翻譯/其他工作完成後，自動補回被延遲的更新。
  window.zfUpdateModalMaybeShowPending = () => {
    try {
      if (pendingUpdateInfo && !isBusy()) {
        const info = pendingUpdateInfo;
        pendingUpdateInfo = null;
        latestUpdateInfo = info;
        window.__mcpl_latestUpdateInfo = latestUpdateInfo;
        showUpdateModal(info);
      }
    } catch (_) {
      /* ignore */
    }
  };

  window.zfUpdateModalHide = hideUpdateModal;
  window.zfUpdateModalSetPending = setPendingUpdateInfo;

  function scheduleClientExit() {
    setTimeout(() => {
      try {
        const win = window.__TAURI__ && window.__TAURI__.window && window.__TAURI__.window.getCurrentWindow;
        if (typeof win === "function") win().close().catch(() => {});
      } catch (_) {
        /* ignore */
      }
      setTimeout(() => {
        try {
          window.close();
        } catch (_) {
          /* ignore */
        }
      }, 500);
    }, 1200);
  }

  async function runUpdateCheck(interactive) {
    if (!_invoke) return;
    let info;
    try {
      info = await _invoke("check_update");
    } catch (e) {
      if (interactive && typeof appendLog === "function") {
        appendLog("暫時無法檢查更新：" + String(e), "warn");
      }
      return;
    }
    const described = describeUpdateCheck(info);
    // 測試版：不下載、不跳更新視窗，只講一句白話
    if (described.kind === "test-build") {
      latestUpdateInfo = null;
      pendingUpdateInfo = null;
      window.__mcpl_latestUpdateInfo = null;
      if (interactive && typeof appendLog === "function") appendLog(described.message);
      return;
    }
    if (!info || !info.ok) {
      if (interactive && typeof appendLog === "function") {
        appendLog((info && info.message) || "暫時無法檢查更新", "warn");
      }
      return;
    }
    latestUpdateInfo = info;
    window.__mcpl_latestUpdateInfo = latestUpdateInfo;
    if (!info.updateAvailable) {
      latestUpdateInfo = null;
      window.__mcpl_latestUpdateInfo = null;
      if (interactive && typeof appendLog === "function") appendLog(info.message || "已是最新版");
      return;
    }
    if (isBusy()) {
      setPendingUpdateInfo(info);
      if (interactive && typeof appendLog === "function") appendLog("忙碌中：更新延遲顯示", "warn");
      return;
    }
    pendingUpdateInfo = null;
    showUpdateModal(info);
    if (interactive && typeof appendLog === "function") {
      appendLog("發現新版本 " + info.latest + "（目前 " + info.current + "）。", "warn");
    }
  }

  async function openManualDownload() {
    hideUpdateModal();
    const url = latestUpdateInfo && latestUpdateInfo.url;
    if (!url || !_invoke) return;
    try {
      await _invoke("open_url", { url });
      if (typeof appendLog === "function") appendLog("已用瀏覽器開啟官方免安裝版下載。", "warn");
    } catch (e) {
      if (typeof appendLog === "function") appendLog("無法開啟手動下載：" + String(e), "error");
    }
  }

  async function runDownload() {
    if (!_invoke || updateInFlight) return;
    updateInFlight = true;
    hideUpdateModal();
    const nowBtn = document.getElementById("btn-update-now");
    if (nowBtn) nowBtn.disabled = true;
    try {
      if (typeof appendLog === "function") appendLog("正在下載並驗證新版 EXE，請勿關閉工具…");
      const DOWNLOAD_INVOKE_TIMEOUT_MS = 180000; // UI 層硬超時：避免前端等待無限久
      let timer = null;
      let r;
      try {
        r = await Promise.race([
          _invoke("download_update"),
          new Promise((_, reject) => {
            timer = window.setTimeout(() => {
              const err = new Error("update_invoke_timeout");
              err.code = "update_invoke_timeout";
              reject(err);
            }, DOWNLOAD_INVOKE_TIMEOUT_MS);
          }),
        ]);
      } finally {
        if (timer) clearTimeout(timer);
      }
      const message = (r && r.message) || "免安裝更新檔已啟動。";
      if (typeof appendLog === "function") appendLog(message);
      if (r && r.alreadyCurrent) {
        latestUpdateInfo = null;
        window.__mcpl_latestUpdateInfo = null;
        return;
      }
      if (r && (r.shouldExit || r.automatic)) scheduleClientExit();
    } catch (e) {
      const isTimeout = e && (e.code === "update_invoke_timeout" || String(e).includes("update_invoke_timeout"));
      if (isTimeout) {
        if (typeof appendLog === "function") appendLog("更新呼叫超時，前端將恢復但更新可能仍在背景進行。", "warn");
      } else {
        if (typeof appendLog === "function") appendLog("下載更新失敗：" + String(e), "error");
        if (latestUpdateInfo && latestUpdateInfo.url) showUpdateModal(latestUpdateInfo);
      }
    } finally {
      updateInFlight = false;
      if (nowBtn) nowBtn.disabled = false;
    }
  }

  function attach() {
    const later = document.getElementById("btn-update-close");
    if (later && !later.dataset.wired) {
      later.dataset.wired = "1";
      later.addEventListener("click", hideUpdateModal);
    }
    const manual = document.getElementById("btn-update-manual");
    if (manual && !manual.dataset.wired) {
      manual.dataset.wired = "1";
      manual.addEventListener("click", openManualDownload);
    }
    const now = document.getElementById("btn-update-now");
    if (now && !now.dataset.wired) {
      now.dataset.wired = "1";
      now.addEventListener("click", runDownload);
    }
    runUpdateCheck(false);
  }

  if (document.readyState === "loading") {
    window.addEventListener("DOMContentLoaded", attach);
  } else {
    attach();
  }
  window.zfCheckUpdate = () => runUpdateCheck(true);
}
