# 使用說明頁面 — 完整規劃（已落地於主視窗 overlay）

> 實作位置：`src/index.html` 的 `#guide-overlay` + `src/styles.css`  
> 開啟：主畫面「完整使用說明」  
> 玩家大綱鏡像：`docs/USER-GUIDE.md`  
> `src/guide.html` 僅「已遷移」提示，勿當主文案

## 資訊架構

| 分組 | 錨點 | 讀者問題 |
|------|------|----------|
| 認識工具 | `g-what` `g-before` `g-quick` | 是什麼？要準備什麼？最快怎麼跑？ |
| 主畫面 | `g-ui` | 每個欄位／按鈕幹嘛？ |
| 翻譯流程 | `g-translate` `g-pipeline` `g-cover` `g-ref` `g-ai` `g-supplement` `g-repair` | 怎麼翻？管線？邊界？AI？續跑？ |
| 裝進遊戲 | `g-apply` `g-output` `g-ingame` | 怎麼套用？目錄？進遊戲檢查？ |
| 其他 | `g-font` `g-close` `g-log` `g-faq` `g-security` `g-community` `g-disclaimer` | 字體、長跑、排錯、安全、紅線、免責 |

## UX 原則

1. 給一般玩家：步驟編號、表格、少術語。
2. 誠實邊界：會／不會翻、不吹 100%、閃退非翻譯。
3. 與 0.3.9 功能對齊：多來源、並行 AI、一鍵套用備份、覆蓋範圍說明。
4. 左側目錄分組；窄螢幕藏 TOC。
5. 維持主視窗 overlay（不開第二 WebView 防全白）。

## 維護

改功能時同步改對應錨點段落；大改時更新本檔分組表。
