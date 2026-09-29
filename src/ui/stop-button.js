/**
 * 停止按鈕的文字（審查 2）。
 *
 * 第一次按停止：後端不會直接丟掉，而是「寫出已翻好的部分並裝進遊戲」。
 * 這時再按一次會真的中斷——寫出與套用都會放棄，所以按鈕要講清楚，不能只寫「再次停止」。
 */
export const STOP_LABELS = Object.freeze({
  idle: "停止翻譯",
  sending: "停止中…",
  sent: "停止已送出",
  afterFirstStop: "正在寫出已翻部分…（再按會放棄套用）",
  /** 滑鼠停在按鈕上時的完整說明 */
  afterFirstStopTitle: "目前正在把已翻好的部分寫出並套用到遊戲。再按一次會放棄這些步驟、立刻中斷。",
});

/** 恢復成一般的「停止翻譯」（審查 F6：一併清掉第一次停止後加的提示）。 */
export function resetStopButton(btn) {
  if (!btn) return;
  btn.disabled = false;
  btn.textContent = STOP_LABELS.idle;
  btn.title = "";
}
