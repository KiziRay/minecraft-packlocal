/**
 * 分頁列的鍵盤操作（規格 §6：分頁方向鍵；WAI-ARIA tabs 模式）。
 * 左右（或上下）切到相鄰分頁並循環，Home／End 到頭尾；只有目前分頁可被 Tab 聚焦（roving tabindex）。
 * 切換本身沿用分頁按鈕既有的 click 處理，這裡不改換頁邏輯。
 */

const NEXT = new Set(["ArrowRight", "ArrowDown"]);
const PREV = new Set(["ArrowLeft", "ArrowUp"]);

/** 按了 key 之後要到第幾個分頁；-1＝不處理這個鍵。 */
export function nextTabIndex(key, index, count) {
  if (!Number.isInteger(count) || count <= 0) return -1;
  const current = Number.isInteger(index) && index >= 0 && index < count ? index : 0;
  if (NEXT.has(key)) return (current + 1) % count;
  if (PREV.has(key)) return (current - 1 + count) % count;
  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  return -1;
}

/** 依「哪個分頁被選中」算出每個分頁的 tabindex；都沒選中時讓第一個可聚焦。 */
export function rovingTabindex(selected) {
  const list = Array.isArray(selected) ? selected : [];
  const active = list.indexOf(true);
  return list.map((on, i) => (i === (active >= 0 ? active : 0) ? "0" : "-1"));
}

function tabsOf(tablist) {
  return Array.from(tablist.querySelectorAll('[role="tab"]')).filter((tab) => !tab.hidden);
}

function syncTabindex(tablist) {
  const tabs = tabsOf(tablist);
  const values = rovingTabindex(tabs.map((tab) => tab.getAttribute("aria-selected") === "true"));
  tabs.forEach((tab, i) => tab.setAttribute("tabindex", values[i]));
}

/** 接上一個 role="tablist"。回傳 sync()：程式換頁後呼叫，讓 tabindex 跟上。 */
export function wireTabKeys(tablist) {
  if (!tablist || typeof tablist.addEventListener !== "function") return () => {};
  tablist.addEventListener("keydown", (event) => {
    const tabs = tabsOf(tablist);
    const index = tabs.indexOf(event.target);
    if (index < 0) return;
    const next = nextTabIndex(event.key, index, tabs.length);
    if (next < 0) return;
    event.preventDefault();
    tabs[next].click();
    tabs[next].focus();
    syncTabindex(tablist);
  });
  // 滑鼠點的、程式切的：下一輪再同步（等既有的 click 處理把 aria-selected 改好）
  tablist.addEventListener("click", () => setTimeout(() => syncTabindex(tablist), 0));
  syncTabindex(tablist);
  return () => syncTabindex(tablist);
}
