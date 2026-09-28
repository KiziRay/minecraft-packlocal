/**
 * 新手引導 4 步（規格 §4.2 tour）：依狀態出現。
 *
 * 第 1 步框「選擇遊戲資料夾」；選好資料夾後才出第 2 步（AI 列）與第 3 步（狀態卡的主要按鈕）；
 * 第 4 步指頁尾「問題回報」與「字體工具」分頁。還沒選資料夾時走完第 1 步就先暫停
 * （不算看完），選好資料夾後從第 2 步接著。Esc＝跳過整個引導並 toast 告知在哪找回。
 */

export const TOUR_STEPS = Object.freeze([
  Object.freeze({
    key: "pick",
    selector: "#btn-card-pick",
    fallback: "#btn-inst",
    needsInstance: false,
    title: "第一步：選遊戲資料夾",
    body: "按「知道了」後，再按框起來的「選擇遊戲資料夾」，選要翻譯的模組整合包遊戲資料夾（裡面有 mods）。",
  }),
  Object.freeze({
    key: "ai",
    selector: "#ai-options-group",
    needsInstance: true,
    title: "第二步：選誰來翻",
    body: "這一列決定用哪種 AI 翻；選「不使用 AI」也能翻，只用其他玩家分享的翻譯。",
  }),
  Object.freeze({
    key: "start",
    selector: "#btn-run",
    fallback: "#status-card",
    needsInstance: true,
    title: "第三步：按主要按鈕",
    body: "上面一句永遠寫著現在的情況，這顆按鈕就是下一步；現在按「開始翻譯」。",
  }),
  Object.freeze({
    key: "help",
    selector: "#btn-issue-report",
    needsInstance: false,
    title: "遇到問題時",
    body: "中文變方框多半是字體，用上方「字體工具」分頁；其他問題按頁尾「問題回報」。",
  }),
]);

/**
 * 決定現在要顯示哪一步。
 * @returns {{kind: "show", index: number} | {kind: "pause"} | {kind: "done"}}
 */
export function tourPlan({ progress = 0, instanceReady = false } = {}) {
  const index = Number.isFinite(progress) && progress > 0 ? Math.floor(progress) : 0;
  if (index >= TOUR_STEPS.length) return { kind: "done" };
  const step = TOUR_STEPS[index];
  // 第 2、3 步要先有資料夾；第 4 步排在它們後面，所以也一起等
  if (index > 0 && !instanceReady) return { kind: "pause" };
  if (step.needsInstance && !instanceReady) return { kind: "pause" };
  return { kind: "show", index };
}

/** 按「下一步／知道了／完成」之後的進度。 */
export function tourAdvance({ index = 0 } = {}) {
  return Math.min(TOUR_STEPS.length, Math.max(0, index) + 1);
}

/** 按鈕文字：第 1 步「知道了」（接著去選資料夾）、最後一步「完成」、其餘「下一步」。 */
export function tourNextLabel(index) {
  if (index === 0) return "知道了";
  return index >= TOUR_STEPS.length - 1 ? "完成" : "下一步";
}

export function tourMeta(index) {
  return `${index + 1} / ${TOUR_STEPS.length}`;
}
