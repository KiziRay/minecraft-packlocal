/**
 * B5a-1 主視窗的流程接線（審查要求從 app.js 移出，行為零變更）：
 * 狀態卡輸入與重畫、翻譯中切到其他分頁的共用一行、D 區（S20、移除翻譯）、S19 動作、
 * 刪除結果並重翻、橫幅（N-01、N-02）、浮層焦點登記、狀態卡按鈕接線。
 * B5d：選資料夾就判定的狀態與動作交給 deps.folderChecks（folder-checks.js）；接續卡已刪。
 *
 * app.js 的狀態與既有函式一律由 deps 注入（getter／callback），這裡不 import app.js。
 */
import { ACTION, computePackState, folderAreaLock, removeTranslationControl, runElsewhereLine } from "./pack-state.js";
import { GENERIC_PRIMARY_ID, applyStatusCard, isAriaDisabled, planStatusCard } from "./status-card.js";
import { createBannerArea, parentFolder, updateBannerDecision } from "../ui/banner.js";
import { watchOverlay } from "../ui/modal-scope.js";

export const UPDATE_BANNER_DISMISSED_KEY = "mcpl-banner-update-dismissed";

/**
 * 匯入（貼回翻譯）結果的照實說明：toast 一句、紀錄寫完整清單（審查低 b）。
 * @returns {{toast: string, log: string, level: "info"|"warn"}}
 */
export function describeImportReport(report) {
  const r = report && typeof report === "object" ? report : {};
  const accepted = Number(r.accepted) || 0;
  const rejected = Array.isArray(r.rejected) ? r.rejected : [];
  const unknown = Array.isArray(r.unknownKeys) ? r.unknownKeys : Array.isArray(r.unknown_keys) ? r.unknown_keys : [];
  const parts = [String(r.summary || "").trim()].filter(Boolean);
  if (rejected.length) parts.push(`格式檢查未過（保留原文，共 ${rejected.length} 條）：\n` + rejected.join("\n"));
  if (unknown.length) parts.push(`找不到對應項目（共 ${unknown.length} 條）：\n` + unknown.join("\n"));
  let toast;
  if (accepted <= 0) toast = "沒有併入任何翻譯（見紀錄）";
  else if (rejected.length || unknown.length) toast = "已併入，部分退回（見紀錄）";
  else toast = "已併入翻譯";
  return { toast, log: parts.join("\n\n"), level: rejected.length || unknown.length || accepted <= 0 ? "warn" : "info" };
}

export function createPackActions(deps) {
  const { $, doc, invoke, getSetting, setSetting } = deps;
  /** 剛移除翻譯的結果（狀態卡 S19a／S19b）；只屬於移除時的那個遊戲資料夾。 */
  let lastRemoval = null;
  let bannerAreaInstance = null;
  let pendingUpdateBannerInfo = null;
  let updateChecker = null;
  /** 最近一次畫出的狀態（B5b：按「重新翻譯」時要知道是不是要先進確認模式）。 */
  let lastState = null;
  const runFlow = () => deps.runFlow || null;

  const instancePath = () => ($("instance")?.value || "").trim();
  const folders = () => deps.folderChecks || null;

  function isRemovalShownFor(path) {
    return !!lastRemoval && !!path && lastRemoval.instancePath === path;
  }

  function clearRemoval() {
    lastRemoval = null;
  }

  function packStateInput() {
    const s = deps.getState();
    const path = instancePath();
    return {
      consentAccepted: s.consentAccepted,
      instancePath: path,
      validation: s.validation,
      versionBlocked: s.versionBlocked,
      versionBlockReason: s.versionBlockReason,
      busy: s.progressBusy || s.shareUploadInFlight || !!s.dataMigrating,
      busyKind: s.progressBusy ? s.busyJobKind || "translate" : s.shareUploadInFlight ? "share" : s.dataMigrating ? "migrate" : "",
      hasResult: !!s.localCacheProbe,
      removal: isRemovalShownFor(path) ? lastRemoval : null,
      pickFolderFresh: deps.disclosure.isShown("pickFolder"),
      translationComplete: s.translationState === "complete",
      // B5d：選資料夾就判定
      folder: folders() ? folders().gateInput(path) : null,
      packChanged: !!s.packChangeProbe,
      hasTranslationRecord: !!s.hasApplyBackups,
      versionUnknown: !!s.versionUnknown,
      hasOptions: folders() ? folders().hasOptions(path) : null,
      extraShown: (key) => deps.disclosure.isShown(key),
      // B5b：開始前確認（§3.1 列）、S12、翻譯中進度與停止中
      prestart: runFlow() ? runFlow().prestartInput() : null,
      failure: runFlow() ? runFlow().failureInput() : null,
      progress: runFlow() ? runFlow().progressInput(deps.pathLeaf(path || "")) : null,
      stopping: runFlow() ? runFlow().isStopping() : false,
      // B5c：這一包的完成結果（依包保存）、本機結果探測、待套用（S11／S14／S17）
      ...(deps.resultActions ? deps.resultActions.stateInput(path, { busy: !!s.progressBusy }) : {}),
    };
  }

  /** 依目前狀態畫狀態卡；回傳狀態讓 syncUiState 決定 AI 列要不要出現。 */
  function renderStatusCard() {
    const input = packStateInput();
    const state = computePackState(input);
    lastState = state;
    if (deps.resultActions) deps.resultActions.noteCardShown(state);
    try {
      applyStatusCard(planStatusCard(state), {
        $,
        doc,
        onAction: onStatusCardAction,
        onRowChange: (id, value, ack) => runFlow() && runFlow().onRowChange(id, value, ack),
      });
      syncRunElsewhere(state, input.instancePath);
      if (folders()) folders().syncBanners(state.id, input.instancePath);
    } catch (e) {
      console.warn("[status-card]", e);
    }
    return state;
  }

  /**
   * 翻譯中切到其他分頁：共用區顯示一行＋把同一顆 #btn-stop 搬過去（文字仍由 stop-button.js 管，G4.25）；
   * 回到翻譯分頁就搬回狀態卡。翻譯分頁鈕加「翻譯中」標記。
   */
  function syncRunElsewhere(state, path) {
    const page = doc.body.dataset.appPage || "translate";
    const line = runElsewhereLine({ page, state, packName: deps.pathLeaf(path || "") });
    const row = $("run-elsewhere");
    const slot = $("run-elsewhere-slot");
    const stop = $("btn-stop");
    const home = doc.querySelector("#status-card .status-card-actions");
    if (row) row.hidden = !line.shown;
    const text = $("run-elsewhere-text");
    if (text) text.textContent = line.sentence;
    if (stop && slot && home) {
      const target = line.shown ? slot : home;
      if (stop.parentElement !== target) target.insertBefore(stop, target === home ? $("btn-card-apply") : null);
    }
    const badge = $("tab-translate-badge");
    if (badge) badge.hidden = state.id !== "S09";
  }

  function onStatusCardAction(action, item) {
    if (folders() && folders().handles(action)) return void folders().onAction(action, item || {});
    // B5c：完成卡（套用、當成新的、人工補翻、分享、開啟結果資料夾、併入用詞建議）
    if (deps.resultActions && deps.resultActions.onAction(action)) return;
    if (action === "run-new-copy") return void (deps.runNewCopy && deps.runNewCopy());
    // B5b：§3.1 列（更換、這次不用 AI、返回、AI 修正按鈕）、S12 與 AI-BLOCKED 的動作
    if (runFlow() && runFlow().onAction(action)) return;
    if (action === "supplement") return void deps.onSupplement();
    if (action === "repair") return void deps.onRepair();
    if (action === "issue-report") return void deps.openIssueReport();
    if (action === ACTION.deleteAndRestart) return void deleteResultAndRestart();
    if (action === ACTION.pickFolder) return void deps.onPickInstance();
    if (action === ACTION.applyResult) return void applyRemovedResult();
    if (action === ACTION.run) return void deps.onRun();
  }

  /** D 區：S20 停用原因（翻譯中）與「移除翻譯」按鈕（全工具唯一入口）。 */
  function syncFolderArea() {
    const s = deps.getState();
    const path = instancePath();
    const busyKind = s.progressBusy ? s.busyJobKind || "translate" : "";
    const lock = folderAreaLock({ busy: s.progressBusy, busyKind, instancePath: path });
    if (folders()) folders().syncLastButton({ locked: lock.locked, lockReason: lock.reason });
    const reason = $("folder-lock-reason");
    if (reason) {
      reason.textContent = lock.reason;
      reason.hidden = !lock.locked;
    }
    const pick = $("btn-inst");
    if (pick) {
      pick.disabled = false;
      pick.setAttribute("aria-disabled", lock.locked ? "true" : "false");
      if (lock.locked) pick.setAttribute("aria-describedby", "folder-lock-reason");
      else pick.removeAttribute("aria-describedby");
    }
    const control = removeTranslationControl({
      instancePath: path,
      hasTranslationRecord: s.hasApplyBackups,
      busy: s.progressBusy,
      busyKind,
    });
    const row = $("remove-translation-row");
    if (row) row.hidden = !control.visible;
    const restore = $("btn-restore");
    if (restore) {
      restore.setAttribute("aria-disabled", control.disabledReason ? "true" : "false");
      if (control.disabledReason) restore.setAttribute("aria-describedby", "folder-lock-reason");
      else restore.removeAttribute("aria-describedby");
    }
  }

  function setRestoreStatus(text) {
    const el = $("restore-status");
    if (!el) return;
    el.textContent = String(text || "");
    el.hidden = !text;
  }

  /** D 區「移除翻譯」：D-05（危險、預設取消）→ 結果在狀態卡 S19a／S19b，失敗原因就地。 */
  async function onRemoveTranslation() {
    if (deps.getState().progressBusy || isAriaDisabled($("btn-restore"))) return;
    const path = instancePath();
    if (!path) return;
    setRestoreStatus("");
    const hadResult = !!deps.getState().localCacheProbe;
    const out = await deps.removalFlow.removeTranslation({ instancePath: path, outputDir: deps.selectedOutputDir() || null });
    if (out.status === "cancelled") return;
    if (out.status === "failed") {
      setRestoreStatus(out.message);
      deps.appendLog("移除翻譯失敗：" + out.message, "error");
      deps.offerRecordResetIfBroken(out.error, path);
      return;
    }
    const result = out.result || {};
    deps.appendLog(result.playerSummary || result.player_summary || "已移除翻譯。", "warn");
    const warnings = Array.isArray(result.warnings) ? result.warnings : [];
    if (warnings.length) deps.appendLog("移除翻譯時的提醒：\n" + warnings.join("\n"), "warn");
    const quarantined = Array.isArray(result.quarantined) ? result.quarantined : [];
    if (quarantined.length) deps.appendLog("隔離區裡你原本的版本：\n" + quarantined.join("\n"), "warn");
    lastRemoval = { instancePath: path, result, hasResult: hadResult };
    if (deps.resultActions) deps.resultActions.forget(path);
    deps.setTranslationState("ready");
    await deps.refreshBackupState();
    deps.syncUiState();
  }

  /** S19a 主要按鈕：把這台電腦上的翻譯結果再套用到遊戲（B5c：同一套套用流程，只套到目前這包）。 */
  async function applyRemovedResult() {
    const path = instancePath();
    if (deps.resultActions) await deps.resultActions.apply();
    // 套用有結果（成功或回到 S11）就收掉 S19a，狀態卡改說套用結果
    if (deps.resultActions && deps.resultActions.stateInput(path).result) lastRemoval = null;
    deps.syncUiState();
  }

  /** 狀態卡「更多」→ 刪除結果並重翻（D-08，危險＋勾選，翻譯中不會出現）。 */
  async function deleteResultAndRestart() {
    const s = deps.getState();
    if (s.progressBusy) return;
    // S15（有變動）時沒有「可用」的結果，但舊結果資料夾仍在，一樣可以刪掉重翻
    const probe = s.localCacheProbe || s.packChangeProbe;
    if (!probe) return deps.log("目前沒有偵測到本機翻譯結果。");
    const outputDir = probe.outputDir || probe.output_dir || deps.selectedOutputDir();
    if (!outputDir) return deps.log("找不到這份翻譯結果的位置。");
    const workRoot = probe.workRoot || probe.work_root || deps.resultWorkDir(outputDir);
    const out = await deps.removalFlow.deleteResultAndRestart({ outputDir, workRoot });
    if (out.status === "cancelled") return;
    if (out.status === "failed") {
      deps.appendError("刪除翻譯結果失敗");
      deps.appendError(out.message);
      return;
    }
    if (out.status === "nothing") {
      deps.appendLog("沒有刪除任何檔案。");
      return;
    }
    deps.appendLog("已刪除翻譯結果，按「開始翻譯」可以從頭翻一次。", "warn");
    if (deps.resultActions) deps.resultActions.forget(instancePath());
    deps.hideLocalCacheCard();
    deps.clearShareableFiles();
    deps.setTranslationState("ready");
    deps.syncUiState();
  }

  /** B 區橫幅（規格 §3.4）。 */
  function bannerArea() {
    if (!bannerAreaInstance) {
      bannerAreaInstance = createBannerArea({
        $,
        doc,
        onAction: (banner) => {
          if (banner.id === "N-01") updateChecker?.showUpdateModal();
          if ((banner.id === "N-05" || banner.id === "N-09") && deps.onBannerAction) deps.onBannerAction(banner);
          if (banner.id === "N-02" && banner.folder) {
            void invoke("open_path", { path: parentFolder(banner.folder) }).catch((e) =>
              deps.appendLog("無法開啟資料夾：" + deps.formatInvokeError(e), "warn")
            );
          }
        },
        onDismiss: (banner) => {
          // N-01 關閉後這個版本不再出現
          if (banner.id === "N-01" && banner.version) setSetting(UPDATE_BANNER_DISMISSED_KEY, banner.version);
          if (folders()) folders().noteBannerDismissed(banner);
        },
      });
    }
    return bannerAreaInstance;
  }

  /** N-01：有新版。等同意頁完成、翻譯結束才出現（不與同意頁疊）。 */
  function maybeShowUpdateBanner(info = pendingUpdateBannerInfo) {
    if (!info) return;
    const s = deps.getState();
    const decision = updateBannerDecision({
      info,
      dismissedVersion: getSetting(UPDATE_BANNER_DISMISSED_KEY, "") || "",
      consentDone: s.consentAccepted,
      busy: s.progressBusy,
    });
    if (decision.kind === "defer") {
      pendingUpdateBannerInfo = info;
      return;
    }
    pendingUpdateBannerInfo = null;
    if (decision.kind === "show") bannerArea().show(decision.banner);
  }

  function setUpdateChecker(checker) {
    updateChecker = checker || null;
  }

  /** 對話框與浮層共用一套焦點規範（規格 §6）：inert、Tab 鎖框、Esc＝最安全選項、焦點回觸發按鈕。 */
  function wireOverlayFocus() {
    const click = (id) => () => $(id)?.click();
    watchOverlay($("consent-overlay"), {
      onEscape: null, // 同意頁：Esc 不作用、不算同意
      initialFocus: () => $("btn-consent-accept"),
    });
    watchOverlay($("onboard-root"), {
      onEscape: () => deps.stopOnboarding(true, { skipped: true }),
      initialFocus: () => $("onboard-next"),
    });
    watchOverlay($("issue-overlay"), { onEscape: () => deps.hideIssueOverlay(), backdropEscape: true });
    watchOverlay($("update-overlay"), { onEscape: click("btn-update-close"), backdropEscape: true });
    watchOverlay($("feedback-overlay"), { onEscape: click("btn-feedback-close"), backdropEscape: true });
    watchOverlay($("gpt-login-overlay"), { onEscape: click("btn-gpt-overlay-close") });
    watchOverlay($("local-llm-overlay"), { onEscape: click("btn-local-llm-overlay-close") });
    watchOverlay($("pack-options-modal"), { onEscape: () => deps.closeMoreDrawer() });
    // B5c：分享給朋友、人工補翻（關閉＝放棄本次流程、不改任何設定）
    watchOverlay($("share-overlay"), { onEscape: click("btn-share-close"), backdropEscape: true });
    watchOverlay($("manual-fix-overlay"), { onEscape: click("btn-manual-fix-close"), backdropEscape: true });
  }

  function wireStatusCardActions() {
    const bind = (id, fn) => {
      const el = $(id);
      if (!el) return;
      deps.markWired(el);
      el.onclick = () => {
        if (isAriaDisabled(el)) return;
        const out = fn();
        if (out && typeof out.then === "function") out.catch((e) => console.warn("[status-card]", id, e));
      };
    };
    bind("btn-card-pick", () => deps.onPickInstance());
    bind("btn-card-apply", () => applyRemovedResult());
    // B5c：S11「套用到遊戲」與其他結果動作走同一顆 generic 主要按鈕（data-action）
    // B5d：其他主要動作共用一顆（動作與路徑寫在 data-*）
    bind(GENERIC_PRIMARY_ID, () => {
      const el = $(GENERIC_PRIMARY_ID);
      return onStatusCardAction(el.dataset.action || "", { path: el.dataset.path || "" });
    });
    bind("btn-last-instance", () => {
      if (deps.getState().progressBusy) return syncFolderArea();
      return folders() && folders().useLast();
    });
    bind("btn-restore", () => onRemoveTranslation());
    // 附加說明的「不再顯示」與「？」對應目前狀態那一則（B5d 起不只 pickFolder）
    const disclosureKey = () => ($("status-card") && $("status-card").dataset.disclosure) || "pickFolder";
    bind("btn-status-extra-dismiss", () => {
      deps.disclosure.retire(disclosureKey());
      deps.syncUiState();
    });
    bind("btn-status-extra-help", () => {
      deps.disclosure.recall(disclosureKey());
      deps.syncUiState();
    });
    bind("btn-issue-open-discord", () => deps.openIssueDiscord());
    ["issue-display-kind", "issue-attach-mc", "issue-attach-pack"].forEach((id) => {
      $(id)?.addEventListener("change", () => deps.refreshIssueReportUi());
    });
  }

  return {
    currentState: () => lastState,
    isRemovalShownFor,
    clearRemoval,
    renderStatusCard,
    syncFolderArea,
    bannerArea,
    maybeShowUpdateBanner,
    setUpdateChecker,
    wireOverlayFocus,
    wireStatusCardActions,
  };
}
