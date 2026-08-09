# 第三方素材與技術出處

本工具為原創，但以下部分參考或取用了其他開源專案，於此致謝並標示授權。

## Koudesuk/Modpack_Translator（MIT License）

- 來源：<https://github.com/Koudesuk/Modpack_Translator>
- 授權：MIT License, Copyright (c) 2026 Koudesuk

取用與參考的內容：

1. **官方繁中術語表資料**：`src-tauri/assets/minecraft_glossary_zh_tw.json`
   （1,945 條 Minecraft 官方繁體中文譯名）取自該專案
   `assets/glossary/minecraft_zh_tw.json`，原樣併入。
2. **佔位符遮罩技術（mask/unmask）**：`src-tauri/src/engine/placeholder.rs` 的
   `mask`／`unmask` 與其 token 正則，移植自該專案 `pipeline/preprocessor.py`
   的 `encode`／`decode` 與 `_PLACEHOLDERS`（以 Rust 重寫）。
3. **分層驗證概念（硬性／軟性 token）**：`placeholder.rs` 的 guard 設計參考自
   該專案 `pipeline/postprocessor.py`。

MIT 授權允許取用、修改與再散布，惟需保留原始著作權與授權聲明——本檔即為此聲明。
術語表本身為 Minecraft 官方繁體中文譯名（事實性資料）。

## Cloudflare Worker 更新／代理模式

`worker/` 的「桌面版更新端點 + AI 代理」架構參考自同生態系的
ZeitFrei-Tool（`check_update` → Worker `/api/desktop/latest`）。
