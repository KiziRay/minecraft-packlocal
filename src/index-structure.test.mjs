import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
// 統一成 \n 再比對：repo 內檔案是 CRLF，編輯工具可能混入 LF，斷言不該因行尾而時過時不過。
const read = (name) => readFileSync(join(here, name), "utf8").replace(/\r\n/g, "\n");
const html = read("index.html");
const app = read("app.js");
const settingsHtml = read("settings.html");
const settingsScript = [read("settings-window.js"), read("settings-window-actions.js")].join("\n");
const lib = read("../src-tauri/src/lib.rs");

test("AI 來源選擇仍在可見區域，關閉 AI 後能改回任何來源", () => {
  const panelAt = html.indexOf('<div id="ai-panel"');
  const segmentedAt = html.indexOf('class="ai-source-segmented"');
  const noteAt = html.indexOf('id="ai-source-note"');
  assert.ok(panelAt > 0, "找不到 #ai-panel");
  assert.ok(segmentedAt > 0 && segmentedAt < panelAt, "AI 來源選擇不可被 #ai-panel 一起隱藏");
  assert.ok(noteAt > 0 && noteAt < panelAt, "AI 來源說明不可被 #ai-panel 一起隱藏");
  for (const id of ["ai-source-local", "ai-source-custom", "ai-source-gpt", "ai-source-none"]) {
    assert.ok(html.includes(`id="${id}"`), `缺少 AI 來源 ${id}`);
  }
});

test("GPT 模型選單維持單一已驗證模型，避免使用者選到已知不相容模型", () => {
  assert.ok(!html.includes('id="gpt-model"'), "不應讓使用者選擇未驗證 GPT 模型");
  assert.ok(!html.includes("gpt-5.6-sol"));
  assert.ok(!html.includes("gpt-5.6-terra"));
});

test("設定與說明共用專用靜態視窗，不再重開完整工作台", () => {
  const openSettings = lib.slice(lib.indexOf("fn open_settings_window"), lib.indexOf("fn focus_main_window"));
  assert.ok(lib.includes("async fn open_settings_window"),
    "建立第二個 WebView 的 command 必須是 async，否則會卡住事件迴圈並留下 about:blank 視窗");
  assert.ok(openSettings.includes('WebviewUrl::App("settings.html".into())'));
  assert.ok(!openSettings.includes('WebviewUrl::App("index.html".into())'));
  assert.ok(openSettings.includes("initialization_script"), "建立前必須注入初始分頁，不能賭事件時序");
  assert.ok(openSettings.includes('w.emit("settings-pane"'), "既有設定視窗要能切到說明分頁");
  assert.ok(settingsHtml.includes('id="pane-general"'));
  assert.ok(settingsHtml.includes('id="pane-help"'));
  assert.ok(!settingsHtml.includes('src="app.js"'), "設定頁不可再次啟動完整工作台");
});

test("設定頁即使互動腳本失效仍有可讀 HTML，並提供系統視窗關閉鈕", () => {
  assert.ok(settingsHtml.includes("設定與使用說明"));
  assert.ok(settingsHtml.includes('id="close-window"'));
  assert.ok(settingsScript.includes('switchPane(boot.pane)'));
  assert.ok(settingsScript.includes('listen("settings-pane"'));
  assert.ok(settingsScript.includes('invoke("focus_main_window")'));
});

test("使用說明命令只切換同一個設定視窗，不會另開第三個 WebView", () => {
  const guide = lib.slice(lib.indexOf("fn open_guide_window"), lib.indexOf("fn open_settings_window"));
  assert.ok(lib.includes("async fn open_guide_window"),
    "使用說明也必須非同步等待設定視窗建立，不能重新引入主事件迴圈死鎖");
  assert.ok(guide.includes('open_settings_window(app, Some("help".into()), None)'));
  assert.ok(!guide.includes("WebviewWindowBuilder"));
});

test("主工具開設定時明確傳入主題，不靠第二個 WebView 猜 localStorage", () => {
  assert.ok(app.includes('invoke("open_settings_window", {'));
  assert.ok(app.includes('theme: document.documentElement.dataset.theme === "light" ? "light" : "dark"'));
  assert.ok(lib.includes("fn focus_main_window"));
});

test("本包選項只存在主工具 modal，且只能在已驗證資料夾時開啟", () => {
  assert.ok(html.includes('id="pack-options-modal"'));
  assert.ok(html.includes('id="pack-options-modal-slot"'));
  assert.ok(html.includes('id="btn-pack-options-close"'));
  const more = app.slice(app.indexOf("function isMoreDrawerOpen"), app.indexOf("function isAppSettingsOpen"));
  assert.ok(more.includes('document.body.dataset.instanceReady === "1"'));
  assert.ok(more.includes("slot.appendChild(host)"));
  assert.ok(more.includes("modal.hidden = false"));
  assert.ok(more.includes("modal.hidden = true"));
  assert.ok(!more.includes('openAppSettings("translate")'));
});

test("本包選項可以用關閉鈕、Esc 與遮罩回到工作台", () => {
  assert.ok(app.includes('$("btn-pack-options-close")?.addEventListener("click", closeMoreDrawer)'));
  assert.ok(app.includes('$("pack-options-modal-shade")?.addEventListener("click", closeMoreDrawer)'));
  const shell = app.slice(app.indexOf("function wireShellChrome"), app.indexOf("function syncUiState"));
  assert.ok(shell.includes("if (isMoreDrawerOpen())"));
  assert.ok(shell.includes("closeMoreDrawer();"));
});

test("設定視窗使用系統關閉鈕，且 Tauri 已授權設定視窗的命令", () => {
  const eventHook = lib.slice(
    lib.indexOf(".on_window_event(|window, event|"),
    lib.indexOf(".invoke_handler(tauri::generate_handler![")
  );
  const otherWindowReturn = eventHook.indexOf('if window.label() != "main"');
  const closeIntercept = eventHook.indexOf("WindowEvent::CloseRequested");
  assert.ok(otherWindowReturn >= 0 && otherWindowReturn < closeIntercept,
    "非 main 視窗必須交回系統處理，關閉鈕才會真的關窗");
  const caps = JSON.parse(read("../src-tauri/capabilities/default.json"));
  assert.ok(caps.windows.includes("settings"), "settings 視窗必須具備 invoke 權限");
});

test("AI 預檢在三條翻譯入口之前；未使用 AI 不會被預檢擋住", () => {
  assert.ok(lib.includes("fn preflight_selected_ai"));
  for (const name of ["run_one_click", "run_supplement", "run_repair"]) {
    const start = lib.indexOf(`fn ${name}(`);
    const body = lib.slice(start, start + 2600);
    assert.ok(start >= 0 && body.includes("preflight_selected_ai(app, use_ai"), `${name} 缺少 AI 預檢`);
  }
  assert.ok(lib.includes("if !use_ai {\n        return Ok(());"));
  assert.ok(lib.includes("此測試不會寫入翻譯結果、翻譯記憶或共享庫"));
  assert.ok(lib.includes("AI 實際翻譯測試通過（未寫入任何翻譯資料）"));
});

test("舊的內嵌設定頁已刪除，本包選項容器改放在本包選項視窗裡", () => {
  // 字串拆開寫，讓「src/ 內 0 處」的 grep 驗收不會被測試本身命中
  assert.ok(!html.includes(['id="page', 'settings"'].join("-")), "index.html 不可再有舊設定頁");
  assert.ok(!app.includes(["IS", "SETTINGS", "WINDOW"].join("_")), "設定視窗不再載入 app.js，不需要這個分支");
  assert.ok(!app.includes("enterSettingsWindowMode"));
  assert.ok(!app.includes("closeSettingsWindow"));
  const slotAt = html.indexOf('id="pack-options-modal-slot"');
  const hostAt = html.indexOf('id="pack-options-host"');
  assert.ok(slotAt > 0 && hostAt > slotAt, "#pack-options-host 必須在本包選項視窗裡");
  for (const id of ["output", "pack-name", "target-version", "reference-pack", "backup-before-apply"]) {
    assert.ok(html.includes(`id="${id}"`), `本包選項欄位 ${id} 不可跟著舊設定頁一起消失`);
  }
});

test("設定視窗分四類，並補齊從舊設定頁搬過來的每一項", () => {
  for (const pane of ["general", "translate", "data", "help"]) {
    assert.ok(settingsHtml.includes(`id="pane-${pane}"`), `缺少分類 ${pane}`);
    assert.ok(settingsHtml.includes(`data-pane="${pane}"`), `缺少分類按鈕 ${pane}`);
  }
  for (const label of ["外觀與操作", "翻譯與 AI", "資料與備份", "關於"]) {
    assert.ok(settingsHtml.includes(`>${label}</button>`), `分類名稱要叫「${label}」`);
  }
  // B5a-2：刪「發現已翻過時提醒我」（cache-remind）；「改回每次詢問」併入備份三選一（backup-choice）
  const required = [
    "output-storage-mode", "pick-output-custom-root", "clear-output-custom-root",
    "remember-api-key", "clear-api-key", "local-cloud-topup", "ui-autoscale", "ui-scale-reset",
    "check-update", "replay-onboarding", "delete-backups", "backup-choice",
  ];
  const wiring = settingsScript + read("settings/data-pane.js");
  for (const id of required) {
    assert.ok(settingsHtml.includes(`id="${id}"`), `設定視窗缺少 ${id}`);
    assert.ok(wiring.includes(`$("${id}")`), `設定視窗沒有接上 ${id}`);
  }
  assert.ok(!settingsHtml.includes("keep-local-model"), "本地模型常駐設定已移除");
});

test("設定視窗縮放範圍 80–170，並與主視窗共用同一套縮放規則", () => {
  assert.ok(settingsHtml.includes('id="ui-scale" type="range" min="80" max="170" step="10"'));
  assert.ok(settingsScript.includes('from "./ui-scale-logic.js"'));
  assert.ok(settingsScript.includes("computeAutoScalePercent("));
});

test("設定視窗文案不出現開發者術語", () => {
  for (const word of ["html zoom", "WebView", "modal", "不落盤", "實例"]) {
    assert.ok(!settingsHtml.includes(word), `設定視窗出現「${word}」`);
  }
});

test("重看引導、顯示更新交給主視窗；刪除全部備份（D-07）在設定視窗內做（B5a-2）", () => {
  const actions = read("settings-window-actions.js");
  assert.ok(actions.includes('askMain("replay-onboarding")'));
  assert.ok(actions.includes("describeUpdateCheck("));
  assert.ok(!actions.includes('askMain("delete-backups")'));
  const pane = read("settings/data-pane.js");
  assert.ok(pane.includes('invoke("delete_apply_backups_cmd"'), "設定視窗自己呼叫後端刪除");
  assert.ok(pane.includes("confirmDialog(deleteBackupsDialog("), "D-07 在設定視窗內確認");
  // B5a-1：診斷分頁刪除，刪除全部備份只剩設定視窗一個入口（規格 §1.2）
  assert.ok(!html.includes('id="btn-delete-backups"'), "主視窗不再有刪除全部備份的舊按鈕");
});

test("B5a-1：主視窗不再有診斷分頁（DOM 與分頁按鈕），翻譯頁的主要按鈕只在狀態卡", () => {
  for (const id of ["tab-diagnose", "page-diagnose", "rail-diagnose", "rail-diagnose-log", "diagnose-log", "btn-diagnose"]) {
    assert.ok(!html.includes(`id="${id}"`), `還有 #${id}`);
  }
  assert.ok(!html.includes('data-page="diagnose"'));
  assert.ok(!html.includes("workbench-nav-primary"), "頂欄的開始／停止翻譯已移到狀態卡");
  const card = html.slice(html.indexOf('id="status-card"'), html.indexOf("</section>", html.indexOf('id="status-card"')));
  for (const id of ["btn-run", "btn-stop", "btn-card-pick", "btn-card-apply"]) {
    assert.ok(card.includes(`id="${id}"`), `#${id} 要在狀態卡裡`);
    assert.equal(html.split(`id="${id}"`).length - 1, 1, `#${id} 只能有一顆`);
  }
  assert.ok(!app.includes('showAppPage("diagnose")'));
  // 問題回報的日誌收集留在後端（診斷回報與記錄讀取指令仍註冊），只拆掉前端分頁
  assert.ok(lib.includes("fn submit_diagnose_report_cmd"));
  assert.ok(lib.includes("fn diagnose_pack_dir_cmd"));
});

test("B5a-1：紀錄不當提示（移除 aria-live、可聚焦），狀態句 aria-live，「？」寫主題", () => {
  assert.match(html, /<pre id="log" class="log log-empty" tabindex="0">/);
  assert.match(html, /<pre id="font-log" class="log log-empty" tabindex="0">/);
  assert.match(html, /id="status-card-sentence" class="status-card-sentence" aria-live="polite"/);
  assert.ok(!html.includes('aria-label="說明">'), "「？」的 aria-label 要寫主題");
});

test("B5a-1 D-06：移除字體包是危險對話框（預設焦點取消），與移除翻譯分開（G1.26）", () => {
  const block = app.slice(app.indexOf('$("btn-font-remove").onclick'), app.indexOf('remove_font_pack_cmd'));
  assert.ok(block.includes('title: "移除字體包？"'));
  assert.ok(block.includes("danger: true"));
  assert.ok(html.includes('id="btn-font-remove"'), "移除字體包留在字體工具頁");
});

test("B5a-1：翻譯中 D 區用 aria-disabled＋原因（S20），不是直接 disabled", () => {
  const hard = app.slice(app.indexOf("const hardLockIds = ["), app.indexOf("hardLockIds.forEach"));
  assert.ok(!hard.includes('"btn-inst"'), "瀏覽…改由 D 區 aria-disabled 管");
  assert.ok(!hard.includes('"btn-run"'), "開始翻譯改由狀態卡 aria-disabled 管");
  assert.ok(html.includes('id="folder-lock-reason"'));
  assert.ok(read("flow/pack-actions.js").includes("folderAreaLock("));
});
