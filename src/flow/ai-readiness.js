/**
 * B5b E1 AI 列（規格 §3.1 AI 列、ux-persona-3 §2.1）：依「實際缺的第一項」說一句話＋一顆修正按鈕。
 *
 * 純函式、不碰 DOM：輸入是 ai_status 的結果（lib.rs::ai_status）與前端已知的補充資訊。
 * 順序：AI 狀態讀不到 → Discord → 各來源自己的準備（本地模型下載、自訂 API 金鑰、ChatGPT 登入）。
 * 失效安全：狀態讀不到時當成「需要處理」並給「重新檢查」，不當成就緒（避免翻到一半才壞）。
 */

export const AI_MODES = Object.freeze(["local", "custom", "gpt", "none"]);

/** 四個選項各一句「適合誰」（ux-persona-3 §2.1；≤40 字）。 */
export const AI_FIT = Object.freeze({
  local: "不想花錢、有獨立顯示卡；沒有顯示卡也能用，但一包可能要好幾小時",
  custom: "想要最快、願意用自己的服務商帳戶付少量費用",
  gpt: "已經有 ChatGPT 帳號、不想另外申請金鑰",
  none: "先試試看，或模組整合包大多已有社群翻譯",
});

/** 選中後的準備清單（只在展開時顯示一次）。 */
export const AI_PREP = Object.freeze({
  local: "要準備：登入 Discord；第一次下載數 GB 的模型",
  custom: "要準備：登入 Discord；服務商的 API 金鑰，帳戶裡要有餘額",
  gpt: "要準備：登入 Discord；在瀏覽器登入 ChatGPT",
  none: "什麼都不用準備；沒有現成翻譯的句子會保留英文",
});

export const AI_LABEL = Object.freeze({
  local: "本地模型",
  custom: "自訂 API",
  gpt: "ChatGPT",
  none: "不使用 AI",
});

const PROVIDER_NAMES = Object.freeze({
  deepseek: "DeepSeek",
  glm: "智譜 GLM",
  openai: "OpenAI",
  qwen: "通義千問",
  other: "服務商",
});

/** 修正按鈕的動作（狀態卡 AI 列用）。 */
export const AI_FIX = Object.freeze({
  recheck: "ai-recheck",
  discordLogin: "ai-discord-login",
  discordJoin: "ai-discord-join",
  localDownload: "ai-local-download",
  localSetDir: "ai-local-set-dir",
  customKey: "ai-custom-key",
  gptLogin: "ai-gpt-login",
});

export function normalizeMode(mode) {
  const m = String(mode || "").trim().toLowerCase();
  return AI_MODES.includes(m) ? m : "local";
}

function gb(bytes) {
  const n = Number(bytes) || 0;
  if (n <= 0) return "";
  const v = n / (1024 * 1024 * 1024);
  return v >= 10 ? `${Math.round(v)} GB` : `${v.toFixed(1)} GB`;
}

/**
 * @param {{
 *   useAi?: boolean, mode?: string, status?: object|null, provider?: string,
 *   localNeedBytes?: number, localDirRemembered?: boolean, gptUsable?: boolean,
 * }} input
 * @returns {{mode: string, ready: boolean, missing: string, sentence: string, fix: {action: string, label: string}|null, paidNote: string, summary: string}}
 */
export function aiReadiness(input) {
  const src = input && typeof input === "object" ? input : {};
  if (src.useAi === false) {
    return view("none", true, "", "", null, "");
  }
  const status = src.status && typeof src.status === "object" ? src.status : null;
  const mode = normalizeMode(status ? status.aiMode || status.ai_mode || src.mode : src.mode);
  const provider = PROVIDER_NAMES[String(src.provider || "")] || "服務商";
  const paidNote = mode === "custom" ? `會用到你的 ${provider} 額度` : mode === "gpt" ? "會用到你的 ChatGPT 額度" : "";

  if (!status) {
    return view(mode, false, "status", "AI 狀態讀不到，檢查網路後重新檢查", fix(AI_FIX.recheck, "重新檢查"), paidNote);
  }
  const bool = (a, b) => !!(status[a] ?? status[b]);
  const discordReady = bool("discordReady", "discord_ready");
  if (!discordReady) {
    const serviceUp = (status.serviceAvailable ?? status.service_available) !== false;
    if (!serviceUp) {
      return view(mode, false, "discord", "Discord 登入服務連不上，檢查網路後重新檢查", fix(AI_FIX.recheck, "重新檢查"), paidNote);
    }
    const loggedIn = bool("loggedIn", "logged_in");
    return view(
      mode,
      false,
      "discord",
      "用 AI 翻要先登入 Discord 並加入官方伺服器",
      loggedIn ? fix(AI_FIX.discordJoin, "加入官方伺服器") : fix(AI_FIX.discordLogin, "登入 Discord"),
      paidNote
    );
  }
  if (mode === "local") {
    const installed = bool("localInstalled", "local_installed") || bool("localReady", "local_ready");
    if (!installed && (src.localDirRemembered || bool("localDirMissing", "local_dir_missing"))) {
      // 記住的安裝位置找不到（移走、外接硬碟沒接）：照實說，不說成還沒下載或記憶體不夠（審查 3b，推測情境）
      return view(mode, false, "local-dir", "找不到本地模型的安裝位置，請重新設定", fix(AI_FIX.localSetDir, "重新設定位置"), paidNote);
    }
    if (!installed) {
      const size = gb(src.localNeedBytes);
      return view(
        mode,
        false,
        "local-model",
        size ? `本地模型還沒下載（約 ${size}）` : "本地模型還沒下載（要下載數 GB）",
        fix(AI_FIX.localDownload, "下載本地模型"),
        paidNote
      );
    }
    return view(mode, true, "", "", null, paidNote);
  }
  if (mode === "custom") {
    const hasKey = bool("providerReady", "provider_ready") || bool("usingOwnKey", "using_own_key");
    if (!hasKey) return view(mode, false, "custom-key", "還沒填 API 金鑰", fix(AI_FIX.customKey, "填入金鑰"), paidNote);
    return view(mode, true, "", "", null, paidNote);
  }
  // gpt
  const gptOk = bool("providerReady", "provider_ready") && (status.expired ?? false) !== true;
  if (!gptOk || src.gptUsable === false) {
    return view(mode, false, "gpt", "ChatGPT 還沒登入或已過期", fix(AI_FIX.gptLogin, "登入 ChatGPT"), paidNote);
  }
  return view(mode, true, "", "", null, paidNote);
}

function fix(action, label) {
  return { action, label };
}

function view(mode, ready, missing, sentence, fixButton, paidNote) {
  const label = AI_LABEL[mode] || AI_LABEL.local;
  const summary =
    mode === "none"
      ? "用誰翻：不使用 AI"
      : ready
        ? `用誰翻：${label}（就緒）`
        : `用誰翻：${label}（需要處理）`;
  return { mode, ready, missing, sentence, fix: fixButton, paidNote, summary };
}

/** 錯誤訊息是不是 Discord 會籍擋下的（後端 login_required 等）。 */
export function isDiscordGateText(text) {
  return /login_required|guild_required|client_upgrade_required|請先完成 Discord 驗證|先登入 Discord/i.test(String(text || ""));
}
