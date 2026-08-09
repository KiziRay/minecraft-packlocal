# 專屬 Logo／Icon 提示詞與套用方式

依你的要求：**不在此用程式／AI 工具直接生圖**。請把下方提示詞貼到你常用的繪圖工具（Midjourney、SD、DALL·E、Leonardo 等），出圖後再套進專案。

## 設計方向（給你對稿用）

| 項目 | 建議 |
|------|------|
| 用途 | Windows 桌面 exe 圖示 + 視窗左上角 |
| 形狀 | 正方形，可安全裁成圓角方塊 |
| 風格 | 深色工具感 + 一點 Minecraft 方塊語彙，**不要**抄官方 Minecraft 標誌 |
| 可讀性 | 16×16 仍能看出「書／語言／方塊」其中一角 |
| 品牌 | 可淡淡帶 ZeitFrei 紫藍（#5e6ad2 / #8b5cf6），不要寫長字 |

**不要**在圖裡寫完整中文工具名（小圖會糊）。需要文字時最多一個「譯」或「TW」。

---

## 主提示詞（英文，多數模型較穩）— 1:1 圖示

複製整段：

```
App icon for a Minecraft modpack Chinese localization desktop tool, square composition, centered subject. Soft isometric soft-edged cube block with subtle grass-top and stone sides (original design, not Mojang logo, not Creeper face). In front of the cube, a clean open book or language sheet with a few simplified CJK-like brush strokes (abstract, not real readable text). Small violet-indigo glow (#5e6ad2 to #8b5cf6) around the book, dark charcoal background #0b0f14, flat modern utility app style, high contrast silhouette, minimal detail so it stays clear at 32px and 16px, no photorealism, no clutter, no watermark, no long text, no Windows UI chrome. Vector-like edges, soft ambient occlusion.
```

### 進階變體 A（更工具向）

```
Square desktop application icon, dark UI utility style. Abstract glyph combining a soft isometric block and a translation mark: two overlapping speech bubbles or a book with a small arrow between simplified Latin letter and abstract CJK stroke. Palette: deep navy background, indigo-violet accent, soft mint highlight. Flat design, thick readable shapes, centered, generous padding from edges, works as Windows .ico, no copyrighted Minecraft branding, no creeper, no real company logos.
```

### 進階變體 B（ZeitFrei 紫調）

```
Premium dark-mode app icon, square. A polished matte cube slightly tilted, with a translucent language ribbon wrapping it in soft purple-blue gradient (#5e6ad2, #8b5cf6). Tiny spark of light suggesting “converted text”. Minimal, elegant, ZeitFrei-adjacent tech aesthetic, soft studio lighting, high contrast, no text labels, no photoreal clutter, safe margins for rounded Windows icon mask.
```

### 負向提示（若工具有 negative）

```
Mojang logo, official Minecraft logo, Creeper face, Steve, pixel-perfect game screenshot, photoreal, busy background, tiny unreadable text, watermark, Windows taskbar mockup, 3D glossy skeuomorphism overload, low contrast gray
```

---

## 建議出圖尺寸

| 檔名（放到 `src-tauri/icons/`） | 尺寸 | 說明 |
|--------------------------------|------|------|
| `icon.png` | **1024×1024** | 主圖（最重要） |
| `128x128.png` | 128×128 | 列表／中等 |
| `128x128@2x.png` | 256×256 | 高 DPI |
| `32x32.png` | 32×32 | 小圖測試清晰度 |
| `icon.ico` | 多尺寸 ICO | Windows exe 實際用 |

**ICO 內建議含：** 16, 24, 32, 48, 64, 128, 256（可只做 16/32/48/256）。

### 用線上／本機轉 ICO（任選）

- https://icoconvert.com / https://convertio.co/png-ico/
- 或 ImageMagick：  
  `magick icon.png -define icon:auto-resize=256,128,64,48,32,16 icon.ico`

macOS 若需要 `icon.icns` 可之後再轉；Windows 免安裝 exe 以 **`.ico` + png 清單**為主。

---

## 套用步驟（給你或之後 session）

1. 用上面提示詞出 **1024 透明或深底** 主圖。  
2. 匯出／縮放成上表各 PNG。  
3. 做成 `icon.ico`。  
4. **覆蓋**  
   `modpack-i18n-tool/src-tauri/icons/`  
   內同名檔（先備份舊的也行）。  
5. 專案目錄執行：  
   `npm run build`  
6. 產物 exe 名稱應為：  
   `Minecraft 模組包專用翻譯工具.exe`  
   （已設 `productName` + `mainBinaryName`）

把做好的 `icon.png` / `icon.ico` 丟進該資料夾後跟我說一聲，可再幫你重編一版確認圖示有進 exe。

---

## 視窗標題／產品名（已改）

- 產品名／exe：`Minecraft 模組包專用翻譯工具`  
- 識別碼仍：`uk.zeitfrei.modpack-i18n`（勿亂改，避免設定資料夾分裂）
