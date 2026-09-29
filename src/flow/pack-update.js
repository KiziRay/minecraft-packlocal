/**
 * B6a-1 模組整合包更新（規格 §2.2 S15 B6a-1 起、S16；§8.2、§8.3）。
 *
 * 純函式、不碰 DOM。輸入是 probe 的 packUpdate（後端 pack_update::UpdateView，唯讀偵測）：
 * { modsChanged, countsKnown, newMods, updatedMods, sentences, textsChanged, mcBefore, mcNow, mcChanged }。
 * 失效安全：沒有 packUpdate（舊後端或探測失敗）→ 回 null，由 folder-state 的 S15 暫行接手（舊行為）。
 */

export const PACK_UPDATE_STATE = Object.freeze({ updated: "S15", mcChanged: "S16" });

/** S15「翻譯更新的部分」：從狀態卡按一次就開跑（R-8，走接續補完＝後端先重掃）。 */
export const PACK_UPDATE_ACTION = Object.freeze({ updatePart: "supplement" });

function count(v) {
  const n = Number(v);
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
}

function fmt(n) {
  return count(n).toLocaleString("zh-TW");
}

/** 附加行（第一次）：新增 N 個模組、M 處任務文字改了；都沒有時說舊的會沿用。 */
export function updateExtraLine(update) {
  const u = update && typeof update === "object" ? update : {};
  const parts = [];
  if (u.countsKnown && count(u.newMods)) parts.push(`新增 ${fmt(u.newMods)} 個模組`);
  if (u.countsKnown && count(u.updatedMods)) parts.push(`更新 ${fmt(u.updatedMods)} 個模組`);
  if (count(u.textsChanged)) parts.push(`${fmt(u.textsChanged)} 處任務文字改了`);
  return parts.length ? parts.join("、") : "翻過的句子會直接沿用，只翻有變的";
}

/** S15 現況句（≤40 字）。 */
export function updateSentence(name, update) {
  const u = update && typeof update === "object" ? update : {};
  if (u.countsKnown && count(u.sentences)) return `「${name}」已更新：約 ${fmt(u.sentences)} 句要翻，其他照舊`;
  if (count(u.textsChanged) && !u.modsChanged) return `「${name}」已更新：${fmt(u.textsChanged)} 處任務文字要重翻`;
  return `「${name}」已更新，只翻有變的部分，其他照舊`;
}

/**
 * S16（MC 版本變了，優先）或 S15（B6a-1 起）。沒有更新差異回 null。
 * @param {{packName?: string, packUpdate?: object|null, extraShown?: (key: string) => boolean}} input
 */
export function packUpdateState(input) {
  const src = input && typeof input === "object" ? input : {};
  const u = src.packUpdate && typeof src.packUpdate === "object" ? src.packUpdate : null;
  if (!u) return null;
  const name = src.packName || "這個模組整合包";
  const shown = typeof src.extraShown === "function" ? src.extraShown : () => true;
  const base = {
    tone: "neutral",
    extraLine: "",
    detailLines: [],
    secondary: [],
    more: [],
    disabledReason: "",
    showAiRow: true,
    showVersionRow: false,
  };
  if (u.mcChanged && u.mcBefore && u.mcNow) {
    return {
      ...base,
      id: PACK_UPDATE_STATE.mcChanged,
      sentence: `Minecraft 版本變了（${u.mcBefore}→${u.mcNow}），要重新翻譯`,
      disclosureKey: "mcChanged",
      extraLine: shown("mcChanged") ? "翻譯記憶與共享庫會讓它比第一次快" : "",
      primary: { action: "run", label: "重新翻譯" },
      reTranslate: true,
    };
  }
  if (!u.modsChanged && !count(u.textsChanged)) return null;
  return {
    ...base,
    id: PACK_UPDATE_STATE.updated,
    sentence: updateSentence(name, u),
    disclosureKey: "packChanged",
    extraLine: shown("packChanged") ? updateExtraLine(u) : "",
    primary: { action: PACK_UPDATE_ACTION.updatePart, label: "翻譯更新的部分" },
    more: [{ action: "delete-and-restart", label: "刪除結果並重翻", danger: true }],
    // R-8：直接開跑，只帶 AI 列（同 S14 接續補完）
    aiRowOnly: true,
    reTranslate: false,
  };
}

/** 完成卡「已幫你做的事」：模組整合包拿掉的模組，舊翻譯已清掉（後端 OneClickResult.packUpdate）。 */
export function updateDoneLines(packUpdate) {
  const u = packUpdate && typeof packUpdate === "object" ? packUpdate : {};
  const removed = count(u.removedMods ?? u.removed_mods);
  const unsure = count(u.unconfirmedRemoved ?? u.unconfirmed_removed);
  const out = [];
  if (removed) out.push(`模組整合包拿掉的 ${fmt(removed)} 個模組，舊翻譯已清掉`);
  // 審查 F1：讀不到或暫時停用（.jar.disabled）的模組無法確認是被拿掉，舊翻譯保留
  if (unsure) out.push(`${fmt(unsure)} 個模組這次讀不到或暫時停用，舊翻譯先保留`);
  return out;
}
