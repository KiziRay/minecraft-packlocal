/** 右欄日誌與步驟列文案（純函式，供單元測試）。 */

/** 右欄日誌一次只呈現最新幾則；完整紀錄寫進結果資料夾，由「報告」查看。 */
export const VISIBLE_LOG_LIMIT = 10;

export function visibleLogLines(lines, limit = VISIBLE_LOG_LIMIT) {
  if (!Array.isArray(lines)) return [];
  const n = Number(limit);
  if (!Number.isFinite(n) || n <= 0) return lines.slice();
  return lines.slice(-Math.trunc(n));
}

/** 被折疊掉的則數；0 代表沒有隱藏任何一則。 */
export function hiddenLogCount(lines, limit = VISIBLE_LOG_LIMIT) {
  if (!Array.isArray(lines)) return 0;
  const n = Number(limit);
  if (!Number.isFinite(n) || n <= 0) return 0;
  return Math.max(0, lines.length - Math.trunc(n));
}

export function formatCount(n) {
  const num = Number(n);
  if (!Number.isFinite(num)) return "0";
  return String(Math.max(0, Math.trunc(num)));
}

export function formatStepMeta(done, total, unit) {
  if (done == null || total == null) return "";
  const d = Number(done);
  const t = Number(total);
  if (!Number.isFinite(d) || !Number.isFinite(t)) return "";
  const suffix = String(unit || "").trim();
  return `${formatCount(Math.max(0, d))}／${formatCount(Math.max(0, t))}${suffix ? ` ${suffix}` : ""}`;
}

/**
 * 步驟索引的「單調前進」夾取：翻譯中途文案偶爾會誤判回較早的步驟（例如
 * 多輪 AI 批次、補充漏翻接續同一輪時文案又出現「準備中」字樣），畫面不該
 * 因此倒退——但這個夾取結果同時也決定「時間該算在哪個步驟」，兩者必須用
 * 同一個答案。用同一個 lastStepIdx 只在「這一輪還沒有真正重新開始」時才夾
 * （`lastStepIdx` 由呼叫端在真正開新一輪時重置成 -1，那種情況不受這裡影響，
 * 允許步驟合理地再跑一次並繼續累加時間）。
 */
export function clampStepIndexForward(idx, lastStepIdx, percent, currentKey, failed) {
  const p = Number(percent) || 0;
  const shouldClamp =
    !failed &&
    idx >= 0 &&
    lastStepIdx >= 0 &&
    idx < lastStepIdx &&
    p > 0 &&
    p < 100 &&
    currentKey !== "done";
  return shouldClamp ? lastStepIdx : idx;
}

/**
 * 「內部訊息 → 一句人話」對照表。
 *
 * 舊版只做少數幾條特例，其餘一律超過 48 字就從中間切斷加「…」，經常切在
 * 半句話中間，使用者看到的是殘句而不是「現在在做什麼」。這裡把每個階段固定
 * 對應到一句完整的話：寧可少講細節，也不要給看不懂的殘句。
 *
 * 順序有意義：由具體到一般，第一個命中的就採用。
 */
const PHASE_PATTERNS = [
  // 全部錨定在開頭：後端訊息常把細節寫在冒號後面（「補充：語言表仍缺 433 條」），
  // 不錨定的話「語言表」這種詞會在別的階段誤命中。
  [/^等待 Discord|^重新登入 Discord|^登入已恢復|^重新載入 Discord/, "等待 Discord 驗證"],
  [/^正在停止|^取消中/, "正在停止…"],
  [/^AI 翻譯中…等待本輪回應/, "AI 等待回應"],
  [/^AI 翻譯中/, "AI 翻譯中"],
  [/^AI 限流/, "AI 忙碌中，已自動放慢"],
  [/^AI 這一輪沒有新譯文/, "這一批沒有新譯文"],
  [/^AI 有.*批失敗/, "有幾批沒成功，正在重送"],
  [/^補強/, "重試先前沒通過的句子"],
  [/^補充/, "補上沒翻到的句子"],
  [/^覆寫文字|^ZIP 文字|^JAR 書本|^JAR 內 Patchouli|^快捷選單|^任務/, "翻譯模組內的說明文字"],
  [/^打包|^重建資源包|^資源包/, "打包翻譯資源包"],
  [/^套用/, "套用到遊戲"],
  [/^檢查/, "檢查資料夾與模組"],
  [/^掃描|^語言表/, "掃描模組文字"],
  [/^合併|^參考包/, "合併既有中文"],
];

/**
 * 訊息裡「有資訊量」的尾巴：檔名、進度數字、略過原因。
 *
 * 上一輪把所有訊息硬對應成固定短語，結果把
 * 「本地整理：資源包（124/162）Mythicmounts-Spanish-v1.0.0.zip」這種真的有用的
 * 細節，壓成沒有資訊量的「補上沒翻到的句子」，而且連續出現三次——使用者反映
 * 「看不出在翻哪個檔案」。這裡改成：階段名稱用人話，但**保留後面的細節**。
 */
function detailTail(m) {
  const colon = m.search(/[：:]/);
  if (colon < 0) return "";
  const tail = m.slice(colon + 1).trim();
  if (!tail) return "";
  // 只保留看得出「哪個檔案／第幾個／為什麼」的尾巴，去掉純粹的狀態重述
  if (!/[0-9（(]|\.zip|\.jar|\.json|略過|失敗|跳過/.test(tail)) return "";
  return tail.length > 34 ? tail.slice(0, 32) + "…" : tail;
}

export function shortenProgressMessage(message) {
  const m = String(message || "").trim();
  if (!m) return m;
  for (const [pattern, label] of PHASE_PATTERNS) {
    if (pattern.test(m)) {
      const tail = detailTail(m);
      return tail ? `${label}：${tail}` : label;
    }
  }
  // 沒有對應的階段才退回截斷；切在標點處，避免斷在半個詞中間。
  if (m.length > 48) {
    const head = m.slice(0, 45);
    const cut = Math.max(head.lastIndexOf("："), head.lastIndexOf("，"), head.lastIndexOf("、"));
    return (cut > 20 ? head.slice(0, cut) : head) + "…";
  }
  return m;
}

/**
 * 「AI 翻譯中」與「AI 等待回應」是同一件事（本地模型／AI 批次處理）的兩個
 * 子狀態，會隨著每一次批次的送出／收到回應持續交替——實測一次本地模型翻譯，
 * 這兩個狀態在 63%～68% 之間交替了超過 60 次、跨了 17 分鐘。原本的去重鍵只跟
 * 「上一行」比對，交替一次文字就換一次，等於完全沒有去重效果。這裡先把這組
 * 交替狀態收斂成同一個標記，再去比對——這一步只影響「日誌該不該再寫一行」，
 * 不影響 `shortenProgressMessage()` 本身：進度條下方的即時訊息仍然分開顯示
 * 「翻譯中」或「等待回應」，那是有意義的即時狀態指示，只是不值得在日誌裡
 * 各自留一行。
 */
function collapseOscillatingAiStates(shortened) {
  if (shortened === "AI 翻譯中" || shortened === "AI 等待回應") return "AI 運作中";
  return shortened;
}

/**
 * 主日誌去重鍵：舊版拿「1% 精確度＋只去掉秒數」的原始訊息當鍵，於是像
 * 「AI 翻譯中…N／M 批…」這種每批次都觸發、但批次數／已得句數每次都不同的訊息，
 * 永遠被判定成「不同行」，主日誌被同一件事的幾十行變體洗版（縮寫後全部長得
 * 一樣只有百分比在跳）。改成拿「精簡後顯示文字」＋「10% 一格」當鍵：文字沒變，
 * 同一個十位數區間內只留一行；跨進下一個十位數區間才會再寫一行。
 */
export function progressLogDedupeKey(percent, message) {
  const msg = collapseOscillatingAiStates(shortenProgressMessage(message))
    .replace(/本輪\s*\d+\s*秒/g, "本輪*秒")
    .replace(/合計\s*\d+\s*秒/g, "合計*秒")
    .replace(/已進行\s*\d+\s*秒/g, "已進行*秒")
    .replace(/預估剩餘[^·]*/g, "")
    .replace(/\s+/g, " ")
    .trim();
  const bucket = Math.floor((Number(percent) || 0) / 10);
  return bucket + "|" + msg;
}
