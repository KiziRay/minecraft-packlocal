import { isSafeOutboundUrl } from "./security.mjs";

// Footer 問題回報的兩條路：
//
// 1. **首選**：設了 `DISCORD_BOT_TOKEN` 就由這支 Worker 直接呼叫 Discord API，
//    在 MCPL_REPORT_CHANNEL_ID 開私人討論串，完全不依賴 bot 主機在不在線。
// 2. **備援**：沒設 token 時轉發給 Azatosz bot 的 `/api/mcpl-issue-thread`。
//
// ⚠️ 備援**不是** `/api/contact-staff`。那是 bot 上另一個功能（聯絡站長 #12），
// 有自己的頻道與會籍檢查。這個檔案的註解曾經寫著「走 contact-staff」，
// 於是回報功能連修三輪都沒好——每一輪修的都是 `/api/mcpl-issue-thread`
// 底下的程式碼，而那條路從來沒有被呼叫到。改路徑前先讀 BOT_ISSUE_PATH 的說明。
//
// MCPL 這支 Worker 要自己 put BOT_API_URL 與 BOT_API_SECRET（Worker 之間不共用）。
export const MCPL_BOT_SETUP_HINT =
  "要讓「問題回報」建立真正的私人討論串：1) 到 Cloudflare Zero Trust 後台找出 " +
  "Azatosz bot 的 Tunnel 公開網域；2) 在 MCPL 這個 Worker（不是 Azatosz 的）執行 " +
  "`npx wrangler secret put BOT_API_URL`（值＝該網域）與 " +
  "`npx wrangler secret put BOT_API_SECRET`（值＝bot .env 的 BOT_API_SECRET，" +
  "與 Azatosz Worker 的同名 secret 不是同一份，需要另外設定）。";

export const ISSUE_SUMMARIES = [
  "翻譯結果不對或沒翻到",
  "套用後遊戲異常",
  "本地模型／AI 無法使用",
  "介面或縮放",
  "分享給其他玩家",
  "其他",
];

export const ISSUE_CAUSES = [
  "剛更新工具或整合包",
  "操作後立刻發生",
  "只有特定整合包",
  "看不懂畫面上的說明",
  "不確定",
];

export const ISSUE_DAILY_LIMIT = 3;
export const ISSUE_DETAIL_MAX = 500;
const JSON_HEADERS = { "content-type": "application/json; charset=utf-8" };
// P0-08：這裡曾經硬編一個 `http://<IP>:3001` 的備援端點。
// 送往 bot 的每一個請求都帶 `Authorization: Bearer BOT_API_SECRET`，
// 走明文 HTTP 等於把服務憑證交給路徑上的任何人（可攔截、可重放）。
// 已移除該備援；備援端點只能由 secret 設定且必須是 https（見 resolveBotOrigins）。

export function sanitizeIssueText(raw) {
  return String(raw || "")
    .replace(/@everyone/gi, "")
    .replace(/@here/gi, "")
    .replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

export function taipeiYmd(nowMs = Date.now()) {
  return new Date(nowMs + 8 * 3600 * 1000).toISOString().slice(0, 10);
}

export function issueDayKey(userId, ymd) {
  return `issue-thread:${userId}:${ymd}`;
}

export function parseIssueBody(body) {
  const src = body && typeof body === "object" ? body : {};
  const summary = String(src.summary || "").trim();
  if (!ISSUE_SUMMARIES.includes(summary)) {
    return { ok: false, status: 400, error: { message: "請選擇問題概要", type: "invalid_summary" } };
  }
  const cause = String(src.cause || "").trim();
  if (!ISSUE_CAUSES.includes(cause)) {
    return { ok: false, status: 400, error: { message: "請選擇問題原因", type: "invalid_cause" } };
  }
  const rawDetail = src.detail == null ? "" : String(src.detail);
  let detail = null;
  if (rawDetail.trim()) {
    const cleaned = sanitizeIssueText(rawDetail).slice(0, ISSUE_DETAIL_MAX);
    if (!cleaned) {
      detail = null;
    } else if (cleaned.length <= 10) {
      return { ok: false, status: 400, error: { message: "詳細說明請超過十個字", type: "detail_too_short" } };
    } else {
      detail = cleaned;
    }
  }
  const toolVersion = sanitizeIssueText(String(src.toolVersion || "")).slice(0, 32) || "未知";
  // 客戶端逾時重送時，帶同一把 key 就不會多開一條討論串（P0-09）。
  // 格式不合就當沒帶，不因此擋下回報——去重是最佳化，不是門檻。
  const rawIdem = String(src.idempotencyKey || "").trim();
  const idempotencyKey = /^[A-Za-z0-9_-]{8,64}$/.test(rawIdem) ? rawIdem : null;
  return { ok: true, summary, cause, detail, toolVersion, idempotencyKey };
}

/** 可對外引用的案件編號。不含使用者身分，可安全貼在任何地方。 */
export function buildCaseId(userId, nowMs = Date.now()) {
  const ymd = taipeiYmd(nowMs).replace(/-/g, "");
  let h = 2166136261 >>> 0;
  for (const ch of `${userId}:${nowMs}`) {
    h = (h ^ ch.charCodeAt(0)) >>> 0;
    h = Math.imul(h, 16777619) >>> 0;
  }
  return `MCPL-${ymd}-${h.toString(16).padStart(8, "0").slice(0, 6).toUpperCase()}`;
}

export function idempotencyCacheKey(userId, key) {
  return `issue-idem:${userId}:${key}`;
}

function json(obj, status = 200) {
  return new Response(JSON.stringify(obj), { status, headers: JSON_HEADERS });
}

function playerFail(status, type, message) {
  return json({ ok: false, error: { message, type } }, status);
}

export async function submitIssueThread(request, env, access) {
  let body = {};
  try {
    body = await request.json();
  } catch {
    return playerFail(400, "invalid_json", "資料格式錯誤");
  }
  const parsed = parseIssueBody(body);
  if (!parsed.ok) return playerFail(parsed.status, parsed.error.type, parsed.error.message);

  const userId = String(access?.userId || "");
  if (!/^\d{5,25}$/.test(userId)) {
    return playerFail(401, "login_required", "請先登入 Discord");
  }

  if (env?.USAGE) {
    const key = issueDayKey(userId, taipeiYmd());
    const used = parseInt((await env.USAGE.get(key)) || "0", 10) || 0;
    if (used >= ISSUE_DAILY_LIMIT) {
      return playerFail(429, "rate_limited", "今天已回報三次，請明天再試");
    }
  }

  // 直接 Bot API 開私人串時，不預先做會籍請求：建立後的 addMember 本身就是更準確
  // 的權限驗證，而且少一次最多 6 秒的 Discord 往返。v30 原本依序做「查會籍 →
  // 建串 → 加兩人 → 送訊息」，桌面端先逾時、討論串卻在之後建立，使用者就看到
  // 假的「連線失敗」。走 Azatosz 備援時才維持會籍檢查。
  const directBotConfigured = !!String(env?.DISCORD_BOT_TOKEN || "").trim();
  const membership = directBotConfigured ? null : await checkGuildMembership(env, userId);
  if (membership === false) {
    return playerFail(
      403,
      "guild_required",
      "這個 Discord 帳號不在官方伺服器內，請先加入後再回報。"
    );
  }

  const report = {
    userId,
    username: String(access?.displayName || "").slice(0, 40),
    summary: parsed.summary,
    cause: parsed.cause,
    detail: parsed.detail || "",
    toolVersion: parsed.toolVersion,
    membershipVerified: directBotConfigured ? null : membership === true,
  };

  // 逾時重送保護：同一把 idempotencyKey 只會真的開一次討論串。
  // 沒有這道保護時，客戶端每重試一次就多一條孤兒討論串（P0-09）。
  const idemKey = parsed.idempotencyKey
    ? idempotencyCacheKey(userId, parsed.idempotencyKey)
    : null;
  if (idemKey && env?.USAGE) {
    const cached = await env.USAGE.get(idemKey);
    if (cached) {
      try {
        return json(JSON.parse(cached));
      } catch {
        // 快取壞掉就當沒有，重新走一次；不因為快取問題擋下回報
      }
    }
  }

  // 優先直接開私人討論串（只需要 DISCORD_BOT_TOKEN，不經第三方 bot）。
  const direct = await createPrivateThread(env, report);
  if (direct.ok) {
    return finish(env, userId, {
      // 私人討論串（type 12）只有被加進去的人看得到。加不進去就是看不到，
      // 不能回「已建立可追蹤的私人討論串」——那是騙人。
      reporterVisible: direct.reporterAdded === true,
      channel: "private_thread",
      idemKey,
    });
  }
  if (direct.reason !== "no_token") {
    // 有 token 卻失敗：把 Discord 講的原因帶回去，不要用我們自己猜的訊息蓋掉
    const why = [direct.reason, direct.status, direct.detail].filter(Boolean).join(" ");
    return playerFail(503, "thread_failed", `無法建立回報討論串（${why}）。請直接到 Discord 告訴我們。`);
  }

  // 沒設 DISCORD_BOT_TOKEN 時，退回舊的「聯絡站長」通道。
  const viaBot = await tryBotApi(env, report);
  if (viaBot === "ok") {
    // 走 bot 通道時，我們拿不到「回報者有沒有被加進討論串」的答案。
    // 不知道就不能宣稱看得到——一律以 partial 呈現並給替代聯絡方式。
    return finish(env, userId, {
      reporterVisible: false,
      channel: "via_bot",
      idemKey,
    });
  }
  if (viaBot && viaBot.kind === "guild_required") {
    // 把 bot 實際回的原因附上，否則使用者只會看到「請先加入」卻不知道自己明明已經加入
    const detail = String(viaBot.detail || "").trim();
    return playerFail(
      403,
      "guild_required",
      detail
        ? `Discord 端拒絕了這次回報：${detail}`
        : "Discord 端拒絕了這次回報（可能是機器人讀不到成員名單）。請直接到 Discord 告訴我們。"
    );
  }
  // 失敗訊息帶上**實際打過的位址**。這一輪就是因為「打錯端點」完全沒有跡象，
  // 才會連修三輪都沒好；下次任何人看到這句話就能立刻比對路徑對不對。
  const tried = `${resolveBotOrigins(env)[0] || "(未設定)"}${BOT_ISSUE_PATH}`;
  if (viaBot === "bot_down") {
    return playerFail(
      503,
      "bot_unavailable",
      `回報通道暫時連不上，請稍後再試，或直接到官方 Discord 告訴我們。（已嘗試：${tried}）`
    );
  }
  return playerFail(
    503,
    "thread_not_available",
    `回報通道還沒接通，請直接到官方 Discord 告訴我們。（已嘗試：${tried}）`
  );
}

/**
 * 每日次數只在真的送出去之後才記，避免失敗也吃掉額度。
 *
 * 回應必須誠實區分兩種成功（P0-09）：
 * - `delivered`：回報已送達**且**回報者本人看得到那條討論串，可以追後續。
 * - `partial`：回報已送達，但回報者看不到討論串。此時**不得**說「已建立可追蹤的私人討論」，
 *   要給案件編號與站外聯絡方式，讓他仍然有辦法追。
 */
async function finish(env, userId, outcome = {}) {
  if (env?.USAGE) {
    const key = issueDayKey(userId, taipeiYmd());
    const used = parseInt((await env.USAGE.get(key)) || "0", 10) || 0;
    await env.USAGE.put(key, String(used + 1), { expirationTtl: 172800 });
  }

  const reporterVisible = outcome.reporterVisible === true;
  const payload = {
    ok: true,
    caseId: buildCaseId(userId),
    delivery: reporterVisible ? "delivered" : "partial",
    reporterVisible,
    channel: outcome.channel || "unknown",
    message: reporterVisible
      ? "已送出，並已建立你看得到的私人討論串，後續回覆會在那裡。"
      : "已送出並會有人看到，但這次沒能把你加進討論串，你不會收到串內回覆。請記下案件編號，到官方 Discord 報這個編號就能接上。",
  };

  if (outcome.idemKey && env?.USAGE) {
    // 保留一天：客戶端逾時重送會拿到同一個 caseId，不會多開一條討論串。
    await env.USAGE.put(outcome.idemKey, JSON.stringify(payload), { expirationTtl: 86400 });
  }
  return json(payload);
}

export function buildContactStaffMessage(report) {
  const summary = String(report?.summary || "其他").slice(0, 40);
  const cause = String(report?.cause || "不確定").slice(0, 40);
  const version = String(report?.toolVersion || "未知").slice(0, 32);
  const detail = String(report?.detail || "").trim() || "（未填詳細說明）";
  return [`【模組包翻譯工具 ${version}】`, `問題概要：${summary}`, `可能原因：${cause}`, detail]
    .join("\n")
    .slice(0, 1000);
}

export function normalizeBotOrigin(raw) {
  return String(raw || "")
    .trim()
    .replace(/^["']+|["']+$/g, "")
    .replace(/\/+$/, "")
    .replace(/\/api$/i, "");
}

/**
 * 這些 origin 之後每一個請求都會帶 `Authorization: Bearer BOT_API_SECRET`。
 *
 * 因此 **https 是硬性條件**，不是偏好：`isSafeOutboundUrl` 只擋內網位址，
 * 它同時放行 http，對「不可外洩的憑證要送去哪裡」這個判斷來說不夠。
 * 任何 http origin 一律丟棄——寧可回報功能暫時不可用，也不外洩服務憑證。
 */
export function resolveBotOrigins(env) {
  const seen = new Set();
  const out = [];
  const add = (raw) => {
    const origin = normalizeBotOrigin(raw);
    if (!origin || seen.has(origin)) return;
    if (!isHttpsOrigin(origin)) return;
    if (!isSafeOutboundUrl(origin)) return;
    seen.add(origin);
    out.push(origin);
  };
  add(env?.BOT_API_URL);
  add(env?.BOT_API_FALLBACK_URL);
  return out;
}

/** 只有 https 才可以承載 Bearer 憑證。 */
export function isHttpsOrigin(raw) {
  try {
    return new URL(String(raw || "")).protocol === "https:";
  } catch (_) {
    return false;
  }
}

/**
 * 設定健康檢查：部署後用瀏覽器就能確認 secret 有沒有設對，
 * 不必等使用者回報「送不出去」才發現。
 *
 * 刻意不回傳 secret 本身，只回「有沒有設」與「上游通不通」。
 */
export async function issueThreadHealth(env) {
  const origins = resolveBotOrigins(env);
  const hasSecret = !!String(env?.BOT_API_SECRET || "").trim();
  const hasBotToken = !!String(env?.DISCORD_BOT_TOKEN || "").trim();
  const result = {
    mode: hasBotToken ? "direct-thread" : "via-bot",
    discordBotTokenConfigured: hasBotToken,
    reportChannelId: String(env?.MCPL_REPORT_CHANNEL_ID || MCPL_REPORT_CHANNEL_ID),
    botApiUrlConfigured: origins.length > 0,
    botApiSecretConfigured: hasSecret,
    originCount: origins.length,
    upstreamReachable: null,
    // 這一輪的教訓：設定與程式碼不一致時完全沒有跡象可查。
    // 把「現在實際會做什麼、打到哪裡」直接寫出來，下次一眼就能比對。
    activePath: hasBotToken
      ? `Discord API 直接在頻道 ${String(env?.MCPL_REPORT_CHANNEL_ID || MCPL_REPORT_CHANNEL_ID)} 開私人討論串`
      : `${origins[0] || "(未設定)"}${BOT_ISSUE_PATH}`,
    devUserId: String(env?.MCPL_DEV_USER_ID || MCPL_DEV_USER_ID),
    hint: "",
  };
  if (hasBotToken) {
    result.hint =
      "已設定 DISCORD_BOT_TOKEN，會直接在指定頻道開私人討論串。請確認 bot 在該伺服器有「建立私人討論串」與「發送訊息」權限。";
    return json(result);
  }
  if (!result.botApiUrlConfigured || !result.botApiSecretConfigured) {
    result.hint =
      "尚未設定完成：設 DISCORD_BOT_TOKEN 直接開討論串（建議），或同時設好 BOT_API_URL 與 BOT_API_SECRET 走舊通道。";
    return json(result);
  }
  // 只探活，不送實際回報內容
  try {
    const upstream = await fetch(`${origins[0]}${BOT_ISSUE_PATH}`, {
      method: "OPTIONS",
      signal: AbortSignal.timeout(5000),
    });
    result.upstreamReachable = upstream.status < 500;
  } catch {
    result.upstreamReachable = false;
  }
  result.hint = result.upstreamReachable
    ? "設定看起來正常，可以實際送一次回報驗證。"
    : "兩個 secret 都設了，但連不到 bot——請確認 BOT_API_URL 指向的網域目前是通的。";
  return json(result);
}

/** 使用者指定的官方伺服器與回報頻道。 */
export const MCPL_GUILD_ID = "308120017201922048";
export const MCPL_REPORT_CHANNEL_ID = "609381371390984202";
/** 每一則回報都要標記並拉進討論串的開發人員帳號。 */
export const MCPL_DEV_USER_ID = "305389581304463360";

/**
 * 回報內容的 Components V2 訊息。
 *
 * 為什麼 `content` 與 `components` 都要放人：`content` 那一行負責**觸發通知**，
 * V2 容器負責**排版**。只放容器不放 content，兩個人都不會收到提及通知。
 *
 * `allowed_mentions` 只列這兩個 id 且 `parse: []`——使用者的自由文字裡就算寫了
 * `@everyone` 或別人的 id，也不會真的 ping 到任何人。
 */
export function buildIssueV2Message(report, opts = {}) {
  const userId = String(report?.userId || "").replace(/[^0-9]/g, "");
  const devId = String(opts.devUserId || MCPL_DEV_USER_ID).replace(/[^0-9]/g, "");
  const version = sanitizeIssueText(report?.toolVersion || "").slice(0, 32) || "未知";
  const who = sanitizeIssueText(report?.username || "").slice(0, 40) || userId;
  const taipei = new Date(Date.now() + 8 * 3600 * 1000).toISOString().slice(0, 16).replace("T", " ");
  const lines = [
    "## 模組包翻譯工具 · 問題回報",
    `> **回報者**：${who}（\`${userId}\`）`,
    `> **時間**：${taipei}（台灣時間）`,
    `> **工具版號**：${version}`,
    "",
    "**問題概要**",
    sanitizeIssueText(report?.summary || "").slice(0, 100) || "（未選）",
    "",
    "**可能原因**",
    sanitizeIssueText(report?.cause || "").slice(0, 100) || "（未選）",
  ];
  const detail = sanitizeIssueText(report?.detail || "").slice(0, 1000);
  if (detail) lines.push("", "**詳細說明**", detail);
  if (opts.membershipVerified === false) {
    lines.push("", "-# ⚠️ 未能確認此帳號的伺服器會籍（查詢失敗，已照常建立）");
  }
  // 去重：回報者自己就是開發人員時（站長本人回報），兩個 id 相同，
  // Discord 會用 SET_TYPE_ALREADY_CONTAINS_VALUE 打回整則訊息（400 / 50035）。
  // 實測結果：討論串開出來了、兩個成員也加進去了，只有最後一則訊息送不出去。
  const mentions = [...new Set([userId, devId].filter(Boolean))];
  return {
    // 通知靠這一行；排版靠下面的容器
    content: mentions.map((id) => `<@${id}>`).join(" "),
    flags: 1 << 15, // IS_COMPONENTS_V2
    components: [
      {
        type: 17, // Container
        accent_color: 0xe67e22,
        components: [{ type: 10, content: lines.join("\n").slice(0, 3900) }],
      },
    ],
    allowed_mentions: { parse: [], users: mentions },
  };
}

/**
 * 直接用 Discord Bot API 在指定頻道開一條私人討論串。
 *
 * 為什麼不繼續走 Azatosz bot 的 /api/contact-staff：那條路實測回 403
 * （「請先加入官方 Discord 伺服器」），但使用者確實在伺服器裡——問題出在
 * 那支 bot 的會籍檢查，而那是另一個專案，從這裡改不到。直接打 Discord API
 * 少一層轉手，也少一個會壞的地方。
 *
 * 需要的 secret 只有 DISCORD_BOT_TOKEN，且 bot 要在該伺服器有
 * 「建立私人討論串」與「發送訊息」權限。
 */
export async function createPrivateThread(env, report) {
  const token = String(env?.DISCORD_BOT_TOKEN || "").trim();
  if (!token) return { ok: false, reason: "no_token" };
  const channelId = String(env?.MCPL_REPORT_CHANNEL_ID || MCPL_REPORT_CHANNEL_ID).trim();
  const headers = {
    Authorization: `Bot ${token}`,
    "Content-Type": "application/json",
  };
  // 1) 開私人討論串（type 12 = GUILD_PRIVATE_THREAD）
  let thread;
  try {
    const res = await fetch(`https://discord.com/api/v10/channels/${channelId}/threads`, {
      method: "POST",
      headers,
      body: JSON.stringify({
        name: buildThreadName(report),
        type: 12,
        invitable: false,
        auto_archive_duration: 10080,
      }),
      signal: AbortSignal.timeout(8000),
    });
    if (!res.ok) {
      const detail = await res.text().catch(() => "");
      return { ok: false, reason: "thread_create_failed", status: res.status, detail: detail.slice(0, 300) };
    }
    thread = await res.json();
  } catch (e) {
    return { ok: false, reason: "network", detail: String(e).slice(0, 200) };
  }
  const threadId = String(thread?.id || "");
  if (!threadId) return { ok: false, reason: "no_thread_id" };

  // 2) 把回報者**和開發人員**都加進討論串。
  //
  // 私人討論串（type 12）**只有被加進去的人看得到**。少加開發人員的話，
  // 回報會靜悄悄地躺在一個沒人看得到的討論串裡，使用者卻收到「已送出」——
  // 這比直接失敗還糟，因為沒有人會發現。
  const devId = String(env?.MCPL_DEV_USER_ID || MCPL_DEV_USER_ID).trim();
  // 開串成功但後面失敗時，把剛開的空討論串收掉。
  // 不收的話每試一次就多留一條沒有內容的私人討論串，愈積愈多。
  // 只刪這一輪剛剛建立的 threadId，不碰任何既有討論串。
  const discardThread = async () => {
    try {
      await fetch(`https://discord.com/api/v10/channels/${threadId}`, {
        method: "DELETE",
        headers,
        signal: AbortSignal.timeout(5000),
      });
    } catch {
      // 收不掉就算了，不能因為清理失敗而蓋掉真正的錯誤原因
    }
  };
  const addMember = async (memberId) => {
    try {
      const res = await fetch(
        `https://discord.com/api/v10/channels/${threadId}/thread-members/${memberId}`,
        { method: "PUT", headers, signal: AbortSignal.timeout(5000) }
      );
      return res.ok;
    } catch {
      return false;
    }
  };
  // 加入回報者、加入開發人員、送出回報內容三件事都只依賴 thread 已建立，彼此沒有
  // 前後依賴。必須同時發出，否則 Discord 變慢時會串成 8 + 5 + 5 + 8 秒，桌面端
  // 已超時但討論串仍在背景被建立。
  const reporterAddedTask = addMember(String(report.userId));
  const devAddedTask = addMember(devId);
  const sendMessageTask = (async () => {
    try {
      const res = await fetch(`https://discord.com/api/v10/channels/${threadId}/messages`, {
        method: "POST",
        headers,
        body: JSON.stringify(
          buildIssueV2Message(report, {
            devUserId: devId,
            membershipVerified: report.membershipVerified !== false,
          })
        ),
        signal: AbortSignal.timeout(8000),
      });
      if (res.ok) return { ok: true };

      const detail = await res.text().catch(() => "");
      // V2 被拒（元件旗標不支援之類）就退回純文字，但人一樣要標到。
      const plainMentions = [...new Set([String(report.userId), devId].filter(Boolean))];
      const plain = await fetch(`https://discord.com/api/v10/channels/${threadId}/messages`, {
        method: "POST",
        headers,
        body: JSON.stringify({
          content: `${plainMentions.map((id) => `<@${id}>`).join(" ")}\n${buildContactStaffMessage(report)}`,
          allowed_mentions: { parse: [], users: plainMentions },
        }),
        signal: AbortSignal.timeout(8000),
      });
      if (plain.ok) return { ok: true };
      return { ok: false, reason: "message_failed", status: res.status, detail: detail.slice(0, 300) };
    } catch (e) {
      return { ok: false, reason: "network", detail: String(e).slice(0, 200) };
    }
  })();
  const [reporterAdded, devAdded, message] = await Promise.all([
    reporterAddedTask,
    devAddedTask,
    sendMessageTask,
  ]);
  if (!devAdded) {
    // 開發人員進不去就等於沒人收得到。與其假裝成功，不如講清楚。
    await discardThread();
    return {
      ok: false,
      reason: "dev_not_added",
      detail: `無法把開發人員 ${devId} 加進私人討論串；私人討論串只有成員看得到，這樣回報不會有人收到。`,
    };
  }
  if (!message.ok) {
    await discardThread();
    return message;
  }
  return { ok: true, threadId, reporterAdded, devAdded };
}

/**
 * 這個 Discord 帳號在官方伺服器裡嗎？
 *
 * 回 `true`／`false`／`null`（查不出來）。**只有明確的 404 才算 false**——
 * 權限不足、逾時、Discord 端 5xx 都回 null，由呼叫端決定放不放行。
 * 站長實測過：人在伺服器裡卻被擋，就是因為把「查詢失敗」當成「不在伺服器」。
 */
export async function checkGuildMembership(env, userId) {
  const token = String(env?.DISCORD_BOT_TOKEN || "").trim();
  if (!token) return null;
  const guildId = String(env?.MCPL_GUILD_ID || MCPL_GUILD_ID).trim();
  const id = String(userId || "").replace(/[^0-9]/g, "");
  if (!id) return null;
  try {
    const res = await fetch(`https://discord.com/api/v10/guilds/${guildId}/members/${id}`, {
      headers: { Authorization: `Bot ${token}` },
      signal: AbortSignal.timeout(6000),
    });
    if (res.ok) return true;
    if (res.status === 404) return false;
    return null;
  } catch {
    return null;
  }
}

/**
 * 這個回應是 bot 自己講的，還是中間層（反向代理／Cloudflare Access）擋下來的？
 *
 * bot 一律回 JSON 且帶 `error` 欄位；代理擋下來的是 HTML 錯誤頁或空白。
 * 分得出來才能決定「這是使用者的問題」還是「這個位址不通、換下一個」。
 */
export function looksLikeBotJsonError(body) {
  const text = String(body || "").trim();
  if (!text || text.startsWith("<")) return false;
  try {
    const parsed = JSON.parse(text);
    return !!(parsed && typeof parsed === "object" && "error" in parsed);
  } catch {
    return false;
  }
}

/**
 * bot 上處理 MCPL 回報的端點。
 *
 * **不要改回 `/api/contact-staff`**——那是 bot 上另一個功能（聯絡站長 #12），
 * 會開在別的頻道、走它自己的會籍檢查。回報功能連續三輪修不好，就是因為
 * 修的是這個路徑指不到的程式碼。
 */
export const BOT_ISSUE_PATH = "/api/mcpl-issue-thread";

async function tryBotApi(env, report) {
  const botSecret = String(env?.BOT_API_SECRET || "").trim().replace(/^["']+|["']+$/g, "");
  const origins = resolveBotOrigins(env);
  if (!origins.length || !botSecret) return "unconfigured";
  // 打 MCPL 自己的端點，不是舊的「聯絡站長」。
  //
  // 這是回報功能修了三輪都沒好的真正原因：/api/contact-staff 是 bot 上另一個
  // 功能（聯絡站長 #12），有它自己的頻道與會籍檢查；而我們一直在修的
  // runMcplIssueThread 掛在 /api/mcpl-issue-thread，從來沒有被呼叫到。
  // 兩個端點的授權相同（Bearer BOT_API_SECRET），所以不需要新增任何密鑰。
  const payload = JSON.stringify({
    userId: report.userId,
    username: report.username,
    summary: report.summary,
    cause: report.cause,
    detail: report.detail || "",
    toolVersion: report.toolVersion,
  });
  let last = "skip";
  for (let i = 0; i < origins.length; i++) {
    const origin = origins[i];
    // 第二道防線：resolveBotOrigins 已經濾過，但送 Bearer 的地方自己再確認一次。
    // 這行是最後一個能阻止憑證離開的位置，不倚賴上游有沒有濾乾淨。
    if (!isHttpsOrigin(origin)) {
      last = "insecure_origin";
      continue;
    }
    const attempts = 1;
    const timeoutMs = i === 0 ? 6000 : 8000;
    for (let attempt = 0; attempt < attempts; attempt++) {
      let upstream;
      try {
        upstream = await fetch(`${origin}${BOT_ISSUE_PATH}`, {
          method: "POST",
          headers: { Authorization: `Bearer ${botSecret}`, "Content-Type": "application/json" },
          body: payload,
          signal: AbortSignal.timeout(timeoutMs),
        });
      } catch {
        last = "bot_down";
        continue;
      }
      if (upstream.status === 403) {
        // 403 不一定來自 bot 本身。
        //
        // 實測：Tunnel（videobot.zeitfrei.uk）掛掉時回 502，而 Cloudflare Access
        // 之類的中間層則會回 403——舊版一收到 403 就直接判定「使用者沒加入伺服器」
        // 並**中止整個重試鏈**，於是明明還有一個活著的備援位址（VPS）也不會去試，
        // 使用者則被叫去做「重新加入伺服器」這件完全沒用的事。
        //
        // 判斷依據改成「這個 403 是不是 bot 自己講的」：bot 會回 JSON 且帶
        // error 欄位；中間層擋下來的是 HTML 或空的。不是 bot 講的就當成這個
        // 位址不可用，換下一個位址繼續試。
        const why = await upstream.text().catch(() => "");
        const fromBot = looksLikeBotJsonError(why);
        if (fromBot) {
          return { kind: "guild_required", detail: String(why || "").slice(0, 200) };
        }
        last = "bot_down";
        continue;
      }
      if (upstream.ok) return "ok";
      const text = await upstream.text().catch(() => "");
      console.warn(`${BOT_ISSUE_PATH} ${upstream.status}`);
      if (upstream.status === 401) return "unconfigured";
      if (upstream.status === 502 || upstream.status === 503 || upstream.status === 504 || /error code:\s*\d+/.test(text)) {
        last = "bot_down";
        continue;
      }
      last = "skip";
      break;
    }
  }
  return last;
}

/** 討論串標題：看得出是誰、什麼問題，但不放使用者自由文字（避免 mention 注入與亂碼標題）。 */
export function buildThreadName(report) {
  const who = sanitizeIssueText(report?.username || "").slice(0, 20) || "玩家";
  const what = String(report?.summary || "問題回報").slice(0, 40);
  return `${what}｜${who}`.slice(0, 90);
}

export function buildIssueEmbed(report) {
  return {
    title: "MCPL 問題回報",
    color: 0xe67e22,
    fields: [
      { name: "問題概要", value: String(report.summary || "—").slice(0, 200), inline: true },
      { name: "可能原因", value: String(report.cause || "—").slice(0, 200), inline: true },
      { name: "工具版本", value: String(report.toolVersion || "—").slice(0, 40), inline: true },
      { name: "詳細說明", value: (String(report.detail || "").slice(0, 1000) || "（未填）") },
      { name: "回報者", value: `<@${String(report.userId || "").replace(/[^0-9]/g, "")}>`, inline: true },
    ],
    timestamp: new Date().toISOString(),
  };
}

/**
 * 用論壇 webhook 建立討論串。
 *
 * Discord 的論壇頻道 webhook 帶 `thread_name` 就會開一則新貼文（＝討論串），不需要 bot。
 * 若 webhook 其實指向一般文字頻道，帶 `thread_name` 會被回 400——那就退回送一般訊息，
 * 至少內容會抵達，不會像舊版那樣整包石沉大海。
 *
 * 優先用 `DISCORD_ISSUE_THREAD_WEBHOOK`（建議指向獨立的論壇頻道）；沒設定才退回沿用
 * `DISCORD_REPORT_WEBHOOK`——但那個 webhook**早就被「診斷回報」功能用來送純文字訊息**
 * （見 report.mjs 的 notifyDiscord()），代表那個頻道多半不是論壇頻道，帶 thread_name
 * 過去幾乎必定 400、退回一般訊息，不會真的建立討論串。這是使用者回報「沒有開討論串，
 * 只是透過 webhook 回報」的根本原因：兩個不相干的功能共用同一個為文字訊息設計的頻道。
 *
 * 誠實澄清：Discord webhook **在 API 層級就無法建立「私人」討論串**——那是 bot token
 * 才有的權限（POST /channels/{id}/threads 指定 PRIVATE_THREAD 需要 bot 身分＋
 * MANAGE_THREADS 權限）。要讓討論串只有幕僚看得到，唯一可行的路是把 webhook
 * 指向的論壇頻道，在 Discord 伺服器的頻道權限設定成幕僚限定——那是 Discord 端的
 * 設定，不是這支程式能代勞的事，因此這裡與所有訊息都不使用「私人」字眼。
 */
async function createForumThread(env, report) {
  const hook = String(
    env?.DISCORD_ISSUE_THREAD_WEBHOOK || env?.DISCORD_REPORT_WEBHOOK || ""
  ).trim();
  if (!hook.startsWith("https://")) return "not_configured";
  const embeds = [buildIssueEmbed(report)];
  // 使用者自由文字只放在 embed 欄位，且全域關掉 mention 解析。
  const base = { embeds, allowed_mentions: { parse: [] } };

  const post = async (payload) => {
    try {
      return await fetch(`${hook}?wait=true`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
        signal: AbortSignal.timeout(12000),
      });
    } catch {
      return null;
    }
  };

  const withThread = await post({ ...base, thread_name: buildThreadName(report) });
  if (withThread?.ok) return "thread";
  // 400＝這個 webhook 不是論壇頻道。一般訊息可以送到，但不是討論串。
  if (withThread && withThread.status === 400) {
    const plain = await post(base);
    if (plain?.ok) return "plain";
  }
  return "failed";
}
