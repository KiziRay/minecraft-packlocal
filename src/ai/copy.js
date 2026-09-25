// 模型選單已移除：ChatGPT 帳號只支援 Luna。
// 實測 400：{"detail":"The 'gpt-5.6-sol' model is not supported when using
// Codex with a ChatGPT account."} 選了另外兩個的人只會在跑到一半時失敗。
export const GPT_COPY = Object.freeze({
  noteManaged:
    "共享庫免登入可查詢。翻譯前請在 Discord 列完成登入。",
  noteCustom:
    "使用你自己的 API 金鑰與額度。推薦 DeepSeek。金鑰只存本機。",
  noteGpt:
    "使用你的 ChatGPT 帳號翻譯，會消耗你的 ChatGPT 帳號額度。開始翻譯前會先試翻一句確認能用；登入資料只存在這台電腦。",
  noteLocal:
    "兩步：先同意並偵測這台電腦，再下載一套執行套件與翻譯模型。速度隨電腦而異。",
  // 給 #ai-source-note 用的短版：舊版把上面那句「兩步」說明同時塞進 ai-source-note
  // 與 local-llm-panel 內的 local-llm-note，畫面上兩個相鄰的位置逐字重複同一句話。
  // 這裡跟 noteCustom／noteGpt 用同樣的「一句話說明這個來源是什麼」格式，
  // 安裝步驟的細節留給面板內的 noteLocal 講。
  noteLocalShort:
    "使用這台電腦執行的本地模型，不佔用外部額度。免費，但速度隨電腦而異。",
  statusLoggedOut: "尚未登入 GPT",
  statusPending: "請在瀏覽器完成登入…（可按取消）",
  loginFailed: "無法完成 GPT 登入。請重試，或改用自訂 API。",
});
