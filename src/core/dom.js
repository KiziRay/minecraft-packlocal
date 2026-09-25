const TAURI = window.__TAURI__ || {};
const invoke =
  (TAURI.core && TAURI.core.invoke) ||
  (() => Promise.reject(new Error("程式尚未就緒，請用免安裝版開啟。")));
const dialog = TAURI.dialog || {};
const listen =
  (TAURI.event && TAURI.event.listen) ||
  (async () => () => {});
// 跨視窗溝通用。設定是**獨立的作業系統視窗**（見 lib.rs open_settings_window），
// 兩個視窗各有各的 DOM，本包選項要靠事件同步。沒有 Tauri 時當成無事發生。
const emit =
  (TAURI.event && TAURI.event.emit) ||
  (async () => {});

const $ = (id) => document.getElementById(id);

/**
 * 這個視窗是誰。
 *
 * `index.html` 同時服務兩個視窗：主視窗（工作台）與設定視窗。
 * 兩者載入同一份 HTML 與同一份 app.js，靠**視窗 label** 區分——
 * 這樣七十幾個設定控制項的程式只需要維護一份。
 *
 * # 為什麼是 label 而不是網址參數
 *
 * 第一版用 `index.html?window=settings`，結果設定視窗開出來是**整片白畫面**：
 * `WebviewUrl::App` 收的是路徑，查詢字串塞進去會讓資產協定找不到檔案。
 * label 是建立視窗時就決定的識別，前端直接讀得到，沒有字串拼接可以出錯。
 *
 * 仍保留網址參數作為後備，方便在瀏覽器裡開 `index.html?window=settings` 檢視版面。
 */
const windowRole = () => {
  try {
    const api = (TAURI && TAURI.window) || (window.__TAURI__ && window.__TAURI__.window);
    const current =
      api &&
      ((typeof api.getCurrentWindow === "function" && api.getCurrentWindow()) ||
        (typeof api.getCurrent === "function" && api.getCurrent()) ||
        api.appWindow);
    if (current && typeof current.label === "string" && current.label) {
      return current.label === "settings" ? "settings" : "main";
    }
  } catch (_) {
    /* 沒有 Tauri（瀏覽器預覽）就往下走網址參數 */
  }
  try {
    return new URLSearchParams(window.location.search).get("window") === "settings"
      ? "settings"
      : "main";
  } catch (_) {
    return "main";
  }
};

export { $, TAURI, dialog, emit, invoke, listen, windowRole };
