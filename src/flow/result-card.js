/**
 * B5c 完成模式（規格 §2.2 S11／S14／S17、§3.3）：依後端實際結果寫結論，不再用前端 headline。
 *
 * - 資料來源：OneClickResult（coveragePercent、pendingCount、interruption.cause、displaySafety、
 *   applyStatus／applyMessage、applyResult 各欄）或本機結果探測（completionPercent、pendingCount）。
 * - 中途停下、還有英文時絕不寫「完成」；原因句依 interruption.cause（後端分類碼）＋條數。
 * - 次要按鈕 ≤3（依 §3.3 順序取），其餘進「更多」。
 * 純函式、不碰 DOM；畫面由 status-card.js 依回傳的狀態畫。
 */

import { updateDoneLines } from "./pack-update.js";

export const RESULT_STATE = Object.freeze({ applyPending: "S11", partial: "S14", done: "S17" });

export const RESULT_ACTION = Object.freeze({
  apply: "result-apply",
  fork: "result-fork-apply",
  supplement: "supplement",
  changeAi: "ai-change",
  manualFix: "manual-fix",
  share: "share",
  openResult: "open-result",
  reTranslate: "run",
  issueReport: "issue-report",
  mergeTerms: "merge-consistency",
  reapply: "result-apply",
});

const SENTENCE_MAX = 40;

function clip(text, max = SENTENCE_MAX) {
  const chars = Array.from(String(text || "").replace(/\s+/g, " ").trim());
  return chars.length > max ? chars.slice(0, max - 1).join("") + "…" : chars.join("");
}

const num = (v) => {
  const n = Number(v);
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
};
const list = (v) => (Array.isArray(v) ? v.filter((x) => x != null && x !== "").map(String) : []);
const pick = (obj, a, b) => (obj ? obj[a] ?? obj[b] : undefined);

/** 後端 applyStatus 的值（翻譯結果與單獨套用兩種形狀都認得；沒有欄位＝已套用）。 */
function statusOf(result) {
  const raw = result && (result.applyStatus ?? result.apply_status ?? result.status);
  return typeof raw === "string" && raw ? raw : "applied";
}

/**
 * 把一次翻譯／接續補完／修復／套用的結果整理成完成卡要的資料。
 * @param {object} result 後端回傳
 * @param {{origin?: string, finishedNow?: boolean, localModelClosed?: boolean, backupChoice?: string}} opts
 */
export function summarizeRun(result, { origin = "run", finishedNow = true, localModelClosed = false, backupChoice = "" } = {}) {
  const r = result && typeof result === "object" ? result : {};
  const intr = r.interruption && typeof r.interruption === "object" ? r.interruption : {};
  const safety = pick(r, "displaySafety", "display_safety") || {};
  const apply = pick(r, "applyResult", "apply_result") || (r.status ? r : null);
  const coverageRaw = pick(r, "coveragePercent", "coverage_percent");
  const coverage = Number.isFinite(Number(coverageRaw)) && coverageRaw !== null && coverageRaw !== undefined ? Math.max(0, Math.min(100, Math.round(Number(coverageRaw)))) : null;
  const plan = pick(r, "runPlan", "run_plan");
  const overrides =
    pick(r, "runPlanHasOverrides", "run_plan_has_overrides") && plan && Array.isArray(plan.decisions)
      ? plan.decisions.filter((d) => d && d.overridden && d.reason).map((d) => String(d.reason))
      : [];
  return {
    origin,
    fresh: !!finishedNow,
    applyStatus: statusOf(r),
    applyMessage: String(pick(r, "applyMessage", "apply_message") || (r.status ? pick(r, "playerSummary", "player_summary") : "") || ""),
    pendingOverwrites: list(pick(r, "pendingOverwrites", "pending_overwrites")),
    coverage,
    pending: num(pick(r, "pendingCount", "pending_count")),
    stopped: !!(intr.aiStopped || intr.ai_stopped || intr.stoppedByUser || intr.stopped_by_user),
    cause: String(intr.cause || (intr.stoppedByUser || intr.stopped_by_user ? "user_stop" : intr.aiStopped || intr.ai_stopped ? "other" : "")),
    noAnswer: num(pick(intr, "noAnswer", "no_answer")),
    qualityDeferred: num(pick(intr, "qualityDeferred", "quality_deferred")),
    rejected: num(pick(safety, "rejectedCount", "rejected_count")) || list(safety.rejected).length,
    unverified: num(pick(safety, "unverifiedCount", "unverified_count")) || list(safety.unverified).length,
    needsOriginal: list(pick(safety, "needsOriginal", "needs_original")).length,
    fontPacks: list(pick(safety, "fontMayNotSupportChinese", "font_may_not_support_chinese")),
    apply: apply ? normalizeApply(apply) : null,
    runPlanNotes: overrides,
    shareable: finishedNow && statusOf(r) === "applied" ? true : !!r.shareable,
    localModelClosed: !!localModelClosed,
    backupChoice: String(backupChoice || ""),
    resourcePackRepaired: false,
    // B6a-1：這一輪因為模組整合包更新做了什麼（拿掉的模組舊翻譯已清掉）
    packUpdate: pick(r, "packUpdate", "pack_update") || null,
  };
}

function normalizeApply(a) {
  return {
    backupDir: String(pick(a, "backupDir", "backup_dir") || ""),
    backupCreated: !!pick(a, "backupCreated", "backup_created"),
    backupReused: !!pick(a, "backupReused", "backup_reused"),
    zipCopied: String(pick(a, "zipCopied", "zip_copied") || ""),
    jarsCopied: num(pick(a, "jarsCopied", "jars_copied")),
    langSet: !!pick(a, "langSet", "lang_set"),
    originalLang: String(pick(a, "originalLang", "original_lang") || ""),
    quarantined: list(pick(a, "quarantinedFiles", "quarantined_files")),
    unconfirmed: list(pick(a, "unconfirmedFiles", "unconfirmed_files")).concat(list(pick(a, "retireSkipped", "retire_skipped"))),
    unknown: list(pick(a, "unknownFiles", "unknown_files")),
    skippedChanged: list(pick(a, "skippedChanged", "skipped_changed")),
    outdatedMods: list(pick(a, "outdatedMods", "outdated_mods")),
    outdatedTexts: list(pick(a, "outdatedTexts", "outdated_texts")),
    stale: list(pick(a, "staleOutputs", "stale_outputs")),
    retired: list(pick(a, "retiredFiles", "retired_files")),
    sourceRemoved: list(pick(a, "sourceRemovedTexts", "source_removed_texts")),
    unverifiable: list(pick(a, "unverifiableTexts", "unverifiable_texts")),
  };
}

/** 本機已有結果（選資料夾時探測到，沒有這次的套用明細）：S14／S17 只寫比例與條數。 */
export function summarizeProbe(probe) {
  const p = probe && typeof probe === "object" ? probe : {};
  const pct = pick(p, "completionPercent", "completion_percent");
  // 審查 3a：上一輪沒跑完（Aborted）時條數與比例不可信 → 不寫數字；最新一輪沒套用 → S11
  const trusted = pick(p, "countsTrusted", "counts_trusted") !== false;
  const lastApplied = pick(p, "lastApplied", "last_applied");
  return {
    origin: "cached",
    fresh: false,
    applyStatus: lastApplied === false ? "notApplied" : "applied",
    applyMessage: "",
    pendingOverwrites: [],
    coverage: trusted && Number.isFinite(Number(pct)) && pct !== null && pct !== undefined ? Math.round(Number(pct)) : null,
    pending: trusted ? num(pick(p, "pendingCount", "pending_count")) : 0,
    stopped: String(p.status || "") === "partial" || !trusted,
    cause: "",
    noAnswer: 0,
    qualityDeferred: 0,
    rejected: 0,
    unverified: 0,
    needsOriginal: 0,
    fontPacks: [],
    apply: null,
    runPlanNotes: [],
    shareable: !!p.shareable,
    localModelClosed: false,
    backupChoice: "",
    resourcePackRepaired: false,
  };
}

/** 部分完成：還有英文，或這一輪中途停下。 */
export function isPartial(summary) {
  const s = summary || {};
  return s.pending > 0 || !!s.stopped;
}

const LANG_NAMES = { en_us: "英文", zh_cn: "簡體中文", zh_tw: "繁體中文", ja_jp: "日文", ko_kr: "韓文" };
function langName(code) {
  const key = String(code || "").toLowerCase();
  return LANG_NAMES[key] || (key ? key : "英文");
}

/**
 * 原因句（§3.3；≤40 字）與主要按鈕。只在部分完成時用。
 * @param {{aiMode?: string, onlineConfigured?: boolean}} ctx
 */
export function reasonFor(summary, { aiMode = "", onlineConfigured = false } = {}) {
  const s = summary || {};
  const supplement = { action: RESULT_ACTION.supplement, label: "接續補完" };
  const pct = s.coverage != null ? s.coverage : null;
  switch (s.cause) {
    case "user_stop":
      return { text: "你按了停止，翻好的部分已套用", primary: supplement, aiRelated: false };
    case "quota":
      return aiMode === "gpt"
        ? { text: "ChatGPT 暫時不接受翻譯；額度重設後按接續補完", primary: supplement, aiRelated: true }
        : { text: "服務商帳戶沒餘額或到上限；儲值後按接續補完", primary: supplement, aiRelated: true };
    case "auth":
      return { text: "金鑰被服務商拒絕；重新填金鑰後接續補完", primary: { action: RESULT_ACTION.changeAi, label: "重新填金鑰" }, aiRelated: true, primaryIsAi: true };
    case "relogin":
      return { text: "Discord 登入失效；重新登入後接續補完", primary: supplement, aiRelated: true };
    case "local_stuck":
      return {
        text: pct != null ? `本地模型在這台電腦跑不動，停在 ${pct}%` : "本地模型在這台電腦跑不動，先停下",
        primary: onlineConfigured ? { action: RESULT_ACTION.changeAi, label: "改用線上 AI 接續" } : supplement,
        aiRelated: true,
        primaryIsAi: onlineConfigured,
      };
    case "local_gone":
      return { text: "本地模型意外關閉，通常是記憶體不夠", primary: supplement, aiRelated: true };
    case "no_output":
      return { text: "AI 一直給不出可用的翻譯，先停下", primary: supplement, aiRelated: true };
    case "network":
      return { text: "網路斷太久，先停下", primary: supplement, aiRelated: false };
    case "other":
      return { text: "AI 中途停下，已翻好的部分已套用", primary: supplement, aiRelated: true };
    default:
      return { text: "", primary: supplement, aiRelated: false };
  }
}

/** 「還是英文的部分」（0 條不顯示；第一次每類附原因，之後只條數）。 */
export function englishLines(summary, { firstTime = true } = {}) {
  const s = summary || {};
  const a = s.apply || {};
  const out = [];
  const add = (count, short, why) => {
    if (count > 0) out.push(firstTime && why ? `${short}：${why}` : short);
  };
  add(s.pending, `還沒翻 ${s.pending} 條`, "接續補完會再翻");
  add(s.qualityDeferred, `翻得不夠好先保留英文 ${s.qualityDeferred} 條`, "可用人工補翻");
  add(s.rejected, `安全檢查退回 ${s.rejected} 條`, "避免遊戲出現方框或亂碼，明細在紀錄");
  add(s.unverified, `缺原文未完整檢查 ${s.unverified} 條`, "之後重新翻譯一次可補完整檢查");
  add(s.needsOriginal, `需要原檔 ${s.needsOriginal} 個檔`, "先移除翻譯或重裝模組整合包再接續補完");
  const mods = (a.outdatedMods || []).length;
  const texts = (a.outdatedTexts || []).length;
  if (mods || texts) out.push(`模組已更新：${mods} 個模組、${texts} 處文字要重翻`);
  add((a.stale || []).length, `舊版結果 ${(a.stale || []).length} 個檔沒放進遊戲`, "要重新翻譯才能套用");
  if (out.length && firstTime) out.push("模組寫死在程式裡的字、伺服器提供的任務書翻不到");
  return out;
}

/** 「已幫你做的事」（自動做了、可逆的事；一行一項）。 */
export function doneLines(summary) {
  const s = summary || {};
  const a = s.apply;
  const out = [];
  if (a) {
    if (a.zipCopied) out.push("啟用翻譯資源包並排最上面");
    if (a.langSet) out.push(`語言設為繁體中文（原本是${langName(a.originalLang)}，移除翻譯會改回）`);
    if (a.backupCreated || a.backupReused) out.push(a.backupDir ? `備份了會被換掉的原檔（路徑：${a.backupDir}）` : "備份了會被換掉的原檔");
    else if (s.backupChoice === "never") out.push("你選了不備份");
    if (a.quarantined.length) out.push(`隔離 ${a.quarantined.length} 個來源不明的檔（詳見紀錄）`);
    if (a.jarsCopied) out.push(`覆蓋了 ${a.jarsCopied} 個模組檔`);
    if (a.retired.length) out.push(`${a.retired.length} 個不再需要的舊翻譯已清掉`);
    if (a.sourceRemoved.length) out.push(`${a.sourceRemoved.length} 個譯文的原文已被移除，沒有放進遊戲`);
    if (a.unconfirmed.length) out.push(`${a.unconfirmed.length} 個舊翻譯檔無法確認來源，保持原樣`);
    if (a.unverifiable.length) out.push(`${a.unverifiable.length} 個檔這次讀不到，沒處理（可再套用一次）`);
    if (a.skippedChanged.length) out.push(`${a.skippedChanged.length} 個檔被模組整合包更新改過，工具不碰`);
  }
  if (s.resourcePackRepaired) out.push("已修好資源包清單");
  out.push(...updateDoneLines(s.packUpdate));
  if (s.localModelClosed) out.push("已關閉本地模型");
  return out;
}

/**
 * 次要按鈕規則（§3.3）：依序取符合者 ≤3，其餘進「更多」。
 * @param {Array<{action: string, label: string}|null|false>} candidates 已依規格順序排好
 */
export function pickSecondary(candidates, extraMore = []) {
  const all = (Array.isArray(candidates) ? candidates : []).filter((c) => c && c.action);
  return { secondary: all.slice(0, 3), more: [...all.slice(3), ...(Array.isArray(extraMore) ? extraMore.filter(Boolean) : [])] };
}

function manualFixWanted(s) {
  return s.pending > 0 || s.qualityDeferred > 0 || s.rejected > 0;
}

const S11_REASON = Object.freeze({
  gameRunning: "Minecraft 還開著",
  noOptionsTxt: "遊戲還沒啟動過",
  needsBackupChoice: "還沒選要不要備份",
  needsOverwriteConfirm: "要確認覆蓋原本的檔案",
  forkNeeded: "這份是複製來的，要先分開記錄",
  notApplied: "按「套用到遊戲」就好",
});

function base(id, fields) {
  return {
    id,
    tone: "neutral",
    sentence: "",
    extraLine: "",
    disclosureKey: "",
    detailLines: [],
    primary: null,
    secondary: [],
    more: [],
    disabledReason: "",
    showAiRow: false,
    showVersionRow: false,
    ...fields,
  };
}

/**
 * 完成卡狀態。
 * @param {object} summary summarizeRun／summarizeProbe 的結果
 * @param {{packName?: string, aiMode?: string, onlineConfigured?: boolean, firstTime?: boolean,
 *   applyFailure?: string, gameStillRunning?: boolean, canShare?: boolean, canMergeTerms?: boolean}} ctx
 */
export function resultCardState(summary, ctx = {}) {
  const s = summary || {};
  const name = ctx.packName || "這個模組整合包";
  const firstTime = ctx.firstTime !== false;
  const pct = s.coverage != null ? s.coverage : null;
  const partial = isPartial(s);
  const english = s.fresh ? englishLines(s, { firstTime }) : [];
  const done = s.fresh ? doneLines(s) : [];
  const detail = [];
  const pushSection = (title, lines) => {
    if (!lines.length) return;
    detail.push(title);
    for (const line of lines) detail.push("・" + line);
  };

  // S11：翻完、還沒套用（或套用失敗）
  if (s.applyStatus !== "applied" || ctx.applyFailure || ctx.gameStillRunning) {
    const failure = String(ctx.applyFailure || "").trim();
    const reason = failure ? "" : S11_REASON[s.applyStatus] || "還沒套用";
    const head = partial && pct != null ? `翻了約 ${pct}%（${s.pending} 條英文），` : "已翻完，";
    // 審查 3c：按下後遊戲仍開著時，「Minecraft 還開著」只在按鈕旁說（狀態句不重複）
    const sentence = failure
      ? `套用沒有完成：${clip(failure, 32)}`
      : ctx.gameStillRunning
        ? `${head}還沒套用`
        : `${head}還沒套用：${reason}`;
    const fork = s.applyStatus === "forkNeeded" && !failure;
    const primary = fork
      ? { action: RESULT_ACTION.fork, label: "當成新的模組整合包" }
      : { action: RESULT_ACTION.apply, label: failure ? "再試一次" : "套用到遊戲" };
    pushSection("還是英文的部分：", english);
    return base(RESULT_STATE.applyPending, {
      tone: failure ? "error" : "block",
      sentence: clip(sentence),
      extraLine: s.applyStatus === "noOptionsTxt" && !failure ? "先用啟動器開一次遊戲再回來按" : "",
      detailLines: detail,
      primary,
      // 按下先檢查遊戲，仍開著就在按鈕旁寫（R-4 就地原因，不跳對話框）
      disabledReason: ctx.gameStillRunning ? "Minecraft 還開著，請先關閉遊戲" : "",
      secondary: [
        { action: RESULT_ACTION.openResult, label: "開啟結果資料夾" },
        ...(failure ? [{ action: RESULT_ACTION.issueReport, label: "問題回報" }] : []),
      ],
      reasonInline: !!ctx.gameStillRunning,
    });
  }

  const manual = manualFixWanted(s) ? { action: RESULT_ACTION.manualFix, label: "人工補翻" } : null;
  const share = ctx.canShare ?? s.shareable ? { action: RESULT_ACTION.share, label: "分享給朋友" } : null;
  const open = { action: RESULT_ACTION.openResult, label: "開啟結果資料夾" };
  const merge = ctx.canMergeTerms ? { action: RESULT_ACTION.mergeTerms, label: "併入用詞建議" } : null;

  if (partial) {
    const why = reasonFor(s, ctx);
    const sentence =
      s.pending > 0
        ? `「${name}」翻了約 ${pct != null ? pct : "?"}%，還有 ${s.pending} 條是英文`
        : `「${name}」這一輪中途停下${pct != null ? `，翻了約 ${pct}%` : ""}`;
    if (why.text) detail.push(why.text);
    pushSection("還是英文的部分：", english);
    pushSection("已幫你做的事：", done);
    const aiOther = why.aiRelated && !why.primaryIsAi ? { action: RESULT_ACTION.changeAi, label: "改用其他 AI 接續" } : null;
    const picked = pickSecondary([aiOther, manual, share, open], [merge]);
    return base(RESULT_STATE.partial, {
      tone: "block",
      sentence,
      detailLines: detail,
      primary: why.primary,
      ...picked,
      showAiRow: !!(why.primaryIsAi || aiOther),
      aiRowOnly: true,
    });
  }

  pushSection("還是英文的部分：", english);
  pushSection("已幫你做的事：", done);
  const picked = pickSecondary([{ action: RESULT_ACTION.reTranslate, label: "重新翻譯" }, manual, share, open], [merge]);
  const pctText = pct != null ? `（中文約 ${pct}%）` : "";
  return base(RESULT_STATE.done, {
    sentence: firstTime || s.fresh ? `「${name}」已套用到遊戲${pctText}，開遊戲就好` : `已翻完並套用${pctText}`,
    detailLines: detail,
    primary: null,
    ...picked,
    reTranslate: true,
  });
}

/** N-05：字體可能不支援中文（只在橫幅說一次，完成卡不重複；規格 §3.4）。 */
export function fontBanner(summary) {
  const packs = (summary && summary.fontPacks) || [];
  if (!packs.length) return null;
  const label = Array.from(String(packs[0])).slice(0, 16).join("");
  return {
    id: "N-05",
    text: `已啟用的「${label}」會換掉字體，中文可能變方框`,
    actionLabel: "開啟字體工具",
    key: packs.slice().sort().join("|"),
  };
}

/** §3.1「更多」裡一行：上次這包的特別設定（runPlan 有覆寫時）。 */
export function runPlanNote(summary) {
  const notes = (summary && summary.runPlanNotes) || [];
  return notes.length ? clip(`上次的特別設定：${notes.join("；")}`, 60) : "";
}
