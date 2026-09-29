/**
 * B5b 開始前的 AI 閘門與選擇落盤（取代 app.js 舊 ensureAiReadyForAction 的彈窗與 ensureCloudTopUpConsent 的同意框）。
 *
 * 規則：這裡**不開任何對話框或浮層**。AI 還沒就緒就回「缺哪一項」，由狀態卡的 AI 列說出真正原因並給對應按鈕。
 * 翻譯（run）、修復（repair）、接續補完（supplement）三個入口共用同一個閘門。
 * deps 由 app.js 注入（不 import app.js），測試用 mock。
 */

import { aiReadiness } from "./ai-readiness.js";
import { prestartCommits } from "./prestart.js";

/**
 * @param {{
 *   useAi: () => boolean,
 *   skipAiOnce?: () => boolean,
 *   waitModeChange?: () => Promise<unknown>,
 *   refreshAiStatus: () => Promise<object|null>,
 *   provider?: () => string,
 *   localNeedBytes?: () => number,
 *   gptUsable: () => Promise<boolean>,
 * }} deps
 */
export function createAiGate(deps) {
  const provider = () => (deps.provider ? deps.provider() : "");
  const need = () => (deps.localNeedBytes ? deps.localNeedBytes() : 0);

  /** @returns {Promise<ReturnType<typeof aiReadiness> & {useAi: boolean}>} */
  async function ensure(_origin = "run", { skipAi = false } = {}) {
    if (!deps.useAi() || skipAi || (deps.skipAiOnce && deps.skipAiOnce())) {
      return { ...aiReadiness({ useAi: false }), useAi: false };
    }
    if (deps.waitModeChange) await Promise.resolve(deps.waitModeChange()).catch(() => null);
    let status = null;
    try {
      status = await deps.refreshAiStatus();
    } catch (_) {
      status = null;
    }
    const base = { status, provider: provider(), localNeedBytes: need() };
    const first = aiReadiness(base);
    if (!first.ready) return { ...first, useAi: true };
    if (first.mode === "gpt") {
      let usable = false;
      try {
        usable = !!(await deps.gptUsable());
      } catch (_) {
        usable = false;
      }
      if (!usable) return { ...aiReadiness({ ...base, gptUsable: false }), useAi: true };
    }
    // 本地模型：已安裝就算就緒；這裡不啟動（G0.5：啟動要在輪次開始之後，見 startLocalForRound）
    return { ...first, useAi: true };
  }

  return { ensure };
}

export const LOCAL_START_FAILED = "本地模型啟動不起來（詳見紀錄），可以改用其他 AI 或這次不用 AI";

/** 啟動等待期間按了停止：照停止收尾（訊息含「已依你的要求停止」，app.js 的 isCancellation 認得；不套用、不算失敗）。 */
export const LOCAL_START_STOPPED = "已依你的要求停止（本地模型啟動期間按了停止，這一輪沒有開始翻譯）";

/**
 * 輪次開始（localModelRounds.begin）之後才啟動本地模型（審查 1c，G0.5／G4.17）：
 * 不用 AI 或不是本地模型就不動；啟動前先讓狀態卡說「正在啟動本地模型」（1c-2）；
 * 啟動不起來把原因寫進紀錄（1c-3）並丟錯，呼叫端的 catch 走 S12、finally 照樣關閉這一輪；
 * 等待啟動期間按了停止（1c-1：後端命令開頭會 reset_cancel，停止會被吞）→ 丟停止，不呼叫後端。
 */
export async function startLocalForRound({ useAi, localMode, start, isStopping = () => false, onStarting = () => {}, log = () => {} }) {
  if (!useAi || !localMode) return;
  onStarting("正在啟動本地模型");
  let ok = false;
  try {
    ok = !!(await start());
  } catch (e) {
    ok = false;
    log(`本地模型啟動不起來：${String((e && e.message) || e)}`);
  }
  if (isStopping()) throw new Error(LOCAL_START_STOPPED);
  if (!ok) throw new Error(LOCAL_START_FAILED);
}

/**
 * 按下開始時把 §3.1 的選擇記住（備份、本地翻不好時），第一次記住時 toast 告訴玩家在哪改（§4.3）。
 * 寫入失敗不擋翻譯：備份沒寫進去時後端照舊回「還沒選備份」，由套用流程的 D-04 補問（失效安全）。
 * @returns {Promise<string[]>} 實際寫入的 kind
 */
export async function commitPrestartChoices(rows, { saveBackupChoice, saveCloudChoice, toast = () => {}, log = () => {} }) {
  const done = [];
  for (const c of prestartCommits(rows)) {
    try {
      if (c.kind === "backup") await saveBackupChoice(c.value);
      else if (c.kind === "cloud") await saveCloudChoice(c.value);
      else continue;
      done.push(c.kind);
      toast(c.toast);
    } catch (e) {
      log(`沒有記住這次的選擇（${c.kind === "backup" ? "備份" : "本地翻不好時"}）：${String((e && e.message) || e)}`);
    }
  }
  return done;
}
