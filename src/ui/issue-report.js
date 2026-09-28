/**
 * 問題回報浮層（規格 §3.8「四、問題回報」、§9 待確認 16，B5a-1）。
 *
 * - 新增「遊戲內顯示問題」＋子分類（方框、亂碼、文字超出框、閃退、某些字沒翻）。
 * - 送出前預覽：畫面上看到的就是實際送出的內容。
 * - 結果在浮層內就地顯示：案件編號，或如實說明沒送出＋內容已複製＋開啟 Discord。
 *
 * Worker 與後端只接受固定的「問題概要」清單（worker/src/issue-thread.mjs、engine/issue_report.rs），
 * 這一批不改 Worker：新分類在前端對映到既有概要，子分類與附帶資料寫進「詳細說明」。
 * 工具版本由後端一律附上（issue_report.rs 的 toolVersion），前端關不掉，所以預覽照實寫出、不給勾選。
 */

/** 與 Worker／後端 ISSUE_SUMMARIES 完全相同（測試會比對）。 */
export const SERVER_SUMMARIES = Object.freeze([
  "翻譯結果不對或沒翻到",
  "套用後遊戲異常",
  "本地模型／AI 無法使用",
  "介面或縮放",
  "分享給其他玩家",
  "其他",
]);

export const DISPLAY_CATEGORY = "遊戲內顯示問題";
export const DISPLAY_KINDS = Object.freeze(["方框", "亂碼", "文字超出框", "閃退", "某些字沒翻"]);
export const DETAIL_MAX = 500;
const USER_DETAIL_MIN = 11;

function chars(text) {
  return Array.from(String(text || ""));
}

/** 畫面上的概要 → 實際送出的概要。 */
export function serverSummaryFor(summary, displayKind) {
  if (summary === DISPLAY_CATEGORY) {
    return displayKind === "某些字沒翻" ? "翻譯結果不對或沒翻到" : "套用後遊戲異常";
  }
  return SERVER_SUMMARIES.includes(summary) ? summary : "";
}

function attachLine(attach, info) {
  const a = attach && typeof attach === "object" ? attach : {};
  const i = info && typeof info === "object" ? info : {};
  const parts = [];
  if (a.mc && String(i.mcVersion || "").trim()) parts.push(`Minecraft ${String(i.mcVersion).trim()}`);
  if (a.pack && String(i.packName || "").trim()) parts.push(`模組整合包 ${String(i.packName).trim()}`);
  return parts.length ? `附帶：${parts.join("；")}` : "";
}

/**
 * 組出要送的內容與預覽。
 * @returns {{ok: boolean, error: string, summary: string, cause: string, detail: string|null, preview: string}}
 */
export function buildIssuePayload({ summary = "", displayKind = "", cause = "", detail = "", attach = {}, info = {} } = {}) {
  const uiSummary = String(summary || "").trim();
  const kind = uiSummary === DISPLAY_CATEGORY ? String(displayKind || "").trim() : "";
  const userDetail = String(detail || "").trim();
  const serverSummary = serverSummaryFor(uiSummary, kind);
  const tag = kind ? `【${DISPLAY_CATEGORY}：${kind}】` : "";
  const extra = attachLine(attach, info);

  // 說明＝子分類標籤＋玩家寫的＋附帶資料；總長不超過 Worker 上限，超過時截玩家寫的那段
  const fixed = [tag, extra].filter(Boolean).join(" ");
  const room = Math.max(0, DETAIL_MAX - chars(fixed).length - 2);
  const clipped = chars(userDetail).slice(0, room).join("");
  const joined = [tag, clipped, extra].filter(Boolean).join(" ");
  const finalDetail = chars(joined).length >= USER_DETAIL_MIN ? joined : null;

  let error = "";
  if (!uiSummary || !String(cause || "").trim()) error = "請選擇問題概要與原因。";
  else if (uiSummary === DISPLAY_CATEGORY && !DISPLAY_KINDS.includes(kind)) error = "請選擇哪一種顯示問題。";
  else if (!serverSummary) error = "請選擇問題概要與原因。";
  else if (userDetail && chars(userDetail).length < USER_DETAIL_MIN) error = "詳細說明請超過十個字。";

  const preview = [
    `問題概要：${serverSummary || "（未選）"}`,
    `問題原因：${String(cause || "").trim() || "（未選）"}`,
    `詳細說明：${finalDetail || "（未填）"}`,
    `另附：你的 Discord 帳號（用來開討論串）、工具版本${String((info && info.toolVersion) || "").trim() ? " " + String(info.toolVersion).trim() : ""}`,
  ].join("\n");

  return {
    ok: !error,
    error,
    summary: serverSummary,
    cause: String(cause || "").trim(),
    detail: finalDetail,
    preview,
  };
}

/**
 * 送出結果的就地說明：成功寫案件編號；失敗如實說明，內容已複製時請玩家貼到 Discord。
 */
export function describeIssueResult(result, { copied = false } = {}) {
  const r = result && typeof result === "object" ? result : {};
  if (r.ok) {
    const caseId = String(r.caseId || r.case_id || "").trim();
    const msg = String(r.message || "").trim();
    const head = caseId ? `已送出，案件編號：${caseId}` : "已送出";
    return { ok: true, text: msg ? `${head}。${msg}` : `${head}。`, showDiscord: false };
  }
  const reason = String(r.message || "").trim() || "目前無法自動送出。";
  const tail = copied
    ? "你填的內容已複製，可以按「開啟 Discord」貼給我們。"
    : "可以按「開啟 Discord」直接告訴我們。";
  return { ok: false, text: `沒有送出：${reason} ${tail}`, showDiscord: true };
}
