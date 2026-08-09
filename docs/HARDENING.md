# 反編譯／逆向工程 — 我們做了什麼、為什麼不做某些事

目標：降低整個 exe 被逆向、抄襲的機率——**同時不增加防毒誤判**（見 `ANTIVIRUS.md`）。
這兩個目標會打架，取捨如下。

## 現實：Rust exe 本來就難逆向

- 本工具主體是 **Rust 編譯成原生機器碼**，不是 .NET／Java 的位元組碼。
  原生碼**無法「反編譯」回原始碼**，只能反組譯成組合語言——門檻遠高於位元組碼。
- 翻譯引擎、術語表、佔位符邏輯、代管切換等「工具怎麼運作」的核心全在 Rust 這一側，
  也就是最難被還原的那一半。

## 已做的硬化（AV 安全）

| 措施 | 效果 | 位置 |
|------|------|------|
| `strip = true` | 移除除錯符號，反組譯少掉函式名等線索 | `Cargo.toml [profile.release]` |
| `lto = true` + `codegen-units = 1` | 跨模組內聯／最佳化，控制流被打散更難讀 | 同上 |
| `opt-level = "s"` | 最佳化過的碼比 debug 難對應回邏輯 | 同上 |
| **前端 `app.js` 最小化＋變數混淆**（terser） | 唯一容易被抽出的部分（HTML/JS）被壓成不可讀；不動 GPT 的原始 `src/`，改由建置時複製到暫存目錄最小化 | `.build-frontend/`（建置暫存） |
| 移除未引用的 TS 原始碼 | 不再把 `main.ts` 之類的原始檔打包進 exe | 建置時剔除 |
| **金鑰不在 exe** | 沒有機密可被抽出（DeepSeek 金鑰在 Worker secret；exe 只有公開的 Worker URL） | `engine/secrets.rs` |

前端沒有機密（沒有金鑰；Worker URL 本來就是公開的），所以最小化屬於「抬高抄襲門檻」，
不是「藏機密」。

## 刻意不做的（會害到你）

- **不加殼／保護器（UPX、Themida、VMProtect…）**：加殼是惡意軟體最典型的規避手法，
  一加殼防毒誤判率飆升——**與「降低防毒誤刪」的要求直接衝突**。一個被防毒刪掉的工具，
  逆向難度再高也沒意義。故不採用。
- **不用 `panic = "abort"`**：雖然能少掉一點展開資訊，但會破壞我們「單一檔案 panic 只記錄、
  不拖垮整個程式」的執行緒隔離設計（`jar_scan` 的平行掃描靠 join 收集錯誤）。穩定性優先。

## 重現「免安裝版」建置（最小化前端 + 重命名）

免安裝版＝`target/release/<mainBinaryName>.exe` 這支獨立執行檔（Win10/11 內建或已裝
WebView2 即可直接跑，不必安裝）。`mainBinaryName` 已設為 `Minecraft 模組包翻譯工具`。

```bash
cd modpack-i18n-tool
# 1) 從 src/ 複製到暫存目錄並最小化前端（不動 GPT 的原始 src/）
rm -rf .build-frontend && cp -r src .build-frontend
npx --yes terser .build-frontend/app.js -c -m -o .build-frontend/app.js
rm -f .build-frontend/main.ts        # 未引用的 TS 原始碼不打包
# 2) 用最小化後的前端建置
npx tauri build --config '{"build":{"frontendDist":"../.build-frontend"}}'
# 3) 取免安裝版 exe
#    src-tauri/target/release/Minecraft 模組包翻譯工具.exe
```

`terser -c -m`（不加 `--toplevel`）只混淆函式內區域變數，保留頂層名稱與 `window.*`／
`invoke("…")` 字串，對這支程式安全。

## 想再往上一階（可選，成本／風險較高）

- **程式碼簽章**：不直接防逆向，但建立信譽、擋竄改（改過的 exe 簽章失效）。見 `ANTIVIRUS.md`。
- **把前端整包搬進 Rust 側自繪 UI**：能消滅可抽取的 HTML/JS，但等於重寫 UI，成本極高、
  且與目前 GPT 維護的視覺工作衝突，不建議。
- **授權／完整性檢查**：可加，但決心破解者總能繞過；對「隨手抄襲」有嚇阻，對高手無效。
