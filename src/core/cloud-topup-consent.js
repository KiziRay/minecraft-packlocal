/**
 * 「本地翻不好時，改用線上 AI 補完」的同意說明。
 *
 * 主視窗第一次需要時會問；設定視窗打開這個開關時也要看到同一段說明，
 * 不可繞過——這會把句子送到線上 AI，並用掉玩家自己的額度。兩邊共用這一份文字。
 */
export const CLOUD_TOPUP_CONSENT = Object.freeze({
  title: "本地翻不好時，要不要改用線上 AI 補完？",
  body:
    "本地模型遇到長句或罕見用語時可能翻得不夠好。你可以讓工具在這種情況改用線上 AI 補完那幾句。\n\n" +
    "選「使用線上 AI 補完」會發生什麼：\n" +
    "• 那幾句原文會送到你設定的線上 AI（不是全部文字）\n" +
    "• 會用掉你自己的 API 額度或 ChatGPT 帳號額度\n" +
    "• 品質通常比較接近線上 AI\n\n" +
    "選「只用本地」會發生什麼：\n" +
    "• 任何文字都不會離開這台電腦，不會有任何費用\n" +
    "• 翻不好的句子會保留原文並列進待補清單，之後可以再補翻\n\n" +
    "兩個選擇都可以隨時在設定裡改。",
  confirmLabel: "使用線上 AI 補完",
  cancelLabel: "只用本地（不送出、不花錢）",
});

/** 設定視窗沒有主視窗的對話框元件，用系統確認視窗顯示同一段說明。 */
export function cloudTopUpConsentText() {
  return `${CLOUD_TOPUP_CONSENT.title}\n\n${CLOUD_TOPUP_CONSENT.body}\n\n按「確定」＝${CLOUD_TOPUP_CONSENT.confirmLabel}；按「取消」＝維持原本的選擇。`;
}
