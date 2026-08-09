# ZeitFrei Modpack Translation Tool — 美術資產

生成方式：GPT Image 2 點陣原圖（非程式繪製）。  
輸出目錄：`img/`

## 視覺系統

| 要素 | 規範 |
|------|------|
| 色 | 深石墨、炭灰、暖銅橙、柔象牙紙、霧鼠尾草綠 |
| 質感 | 低調印刷紙紋、克制像素細節 |
| 氣氛 | 編輯／文件／Markdown 工具感；成熟安靜實用 |
| 禁止 | 可讀文字／字母數字、Minecraft 官方素材、霓虹、紫藍光、玻璃擬態、炫光、奇幻 |

## 資產清單

| 檔名 | 尺寸 | 用途 |
|------|------|------|
| `zeitfrei-modpack-icon-gpt-image-2.png` | 2048×2048 | 桌面應用圖示 |
| `workspace-background-dark.png` | 3840×2160 | 深色工作區背景 |
| `workspace-background-light.png` | 3840×2160 | 淺色工作區背景 |
| `translation-service-illustration.png` | 1536×1024 | 翻譯服務插圖 |
| `font-service-illustration.png` | 1536×1024 | 字體服務插圖 |
| `translation-status-illustration.png` | 1024×1024 | 翻譯流程狀態插圖 |
| `resource-pack-output-illustration.png` | 1024×1024 | 資源包產出插圖 |
| `guide-background-dark.png` | 2048×1152 | 說明頁深色背景 |
| `guide-background-light.png` | 2048×1152 | 說明頁淺色背景 |

## 接入建議

- 圖示：縮成 `src-tauri/icons/` 各尺寸（另開建置時再打包）
- 工作區背景：可選 CSS `background-image`（目前 UI 以純色為主，可日後接）
- 分頁插圖：翻譯頁／字體頁 header 旁
- 說明頁：guide overlay 背景

## 備註

- 原圖由 image 模型產出；此處 PNG 為依目標解析度高品質縮放後的成品檔名。
- 若需 100% 原生解析度再產一輪，可對單一資產重跑生成。

