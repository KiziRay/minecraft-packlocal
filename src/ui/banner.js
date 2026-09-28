/**
 * 橫幅（規格 §1.1 B 區、§3.4）：需要知道但不用立刻決定的事。
 *
 * - 同時最多 2 則，依序號優先（N-01 最優先）。
 * - 可關閉、0–1 顆按鈕；不得和狀態卡講同一件事。
 * - B5a-1 只接 N-01（有新版）與 N-02（設定檔損壞）；其餘由各批次加。
 */
import { describeUpdateCheck } from "../core/update-status.js";

export const MAX_VISIBLE_BANNERS = 2;
/** 序號＝優先序。 */
export const BANNER_ORDER = Object.freeze(["N-01", "N-02", "N-03", "N-04", "N-05", "N-06", "N-07", "N-08", "N-09"]);

/** 依優先序取前 2 則。 */
export function visibleBanners(banners) {
  const list = (Array.isArray(banners) ? banners : []).filter((b) => b && b.id && b.text);
  const rank = (id) => {
    const i = BANNER_ORDER.indexOf(id);
    return i < 0 ? BANNER_ORDER.length : i;
  };
  return list.slice().sort((a, b) => rank(a.id) - rank(b.id)).slice(0, MAX_VISIBLE_BANNERS);
}

/**
 * N-01 有新版：非測試版（G0.4）、同意頁已完成（不與同意頁疊）、非翻譯中、這個版本沒被關過。
 * 回傳 show／defer（晚點再判斷）／none。
 */
export function updateBannerDecision({ info, dismissedVersion = "", consentDone = false, busy = false } = {}) {
  const described = describeUpdateCheck(info);
  if (described.kind !== "available") return { kind: "none" };
  const latest = String(info.latest || "").trim();
  if (latest && latest === String(dismissedVersion || "").trim()) return { kind: "none" };
  if (!consentDone || busy) return { kind: "defer" };
  const current = String(info.current || "").trim();
  return {
    kind: "show",
    banner: {
      id: "N-01",
      text: current ? `有新版 ${latest}（目前 ${current}）` : `有新版 ${latest}`,
      actionLabel: "更新",
      version: latest,
    },
  };
}

/** N-02 設定檔損壞改用預設。原檔保留時才寫「原檔已保留」（不說沒做到的事）。 */
export function settingsHealthBanner(notice) {
  if (!notice) return null;
  const kept = !!notice.backupKept;
  return {
    id: "N-02",
    text: kept ? "設定檔讀不出來，已先用預設值（原檔已保留）" : "設定檔讀不出來，已先用預設值",
    actionLabel: notice.folder ? "開啟所在資料夾" : "",
    folder: String(notice.folder || ""),
  };
}

/** 取路徑的上一層資料夾（開啟所在資料夾用）。 */
export function parentFolder(path) {
  const text = String(path || "").trim().replace(/[\\/]+$/, "");
  const cut = Math.max(text.lastIndexOf("/"), text.lastIndexOf("\\"));
  return cut > 0 ? text.slice(0, cut) : text;
}

/**
 * B 區的畫面。`onAction(banner)` 收按鈕、`onDismiss(banner)` 收關閉。
 */
export function createBannerArea({ $, doc, onAction = () => {}, onDismiss = () => {} }) {
  const active = new Map();

  function render() {
    const area = $("banner-area");
    if (!area) return;
    area.textContent = "";
    const shown = visibleBanners([...active.values()]);
    area.hidden = shown.length === 0;
    for (const banner of shown) {
      const row = doc.createElement("div");
      row.className = "banner";
      row.dataset.banner = banner.id;
      const text = doc.createElement("p");
      text.className = "banner-text";
      text.textContent = banner.text;
      row.appendChild(text);
      if (banner.actionLabel) {
        const action = doc.createElement("button");
        action.type = "button";
        action.className = "secondary-button banner-action";
        action.textContent = banner.actionLabel;
        action.addEventListener("click", () => onAction(banner));
        row.appendChild(action);
      }
      const close = doc.createElement("button");
      close.type = "button";
      close.className = "text-button banner-close";
      close.textContent = "關閉";
      close.setAttribute("aria-label", `關閉：${banner.text}`);
      close.addEventListener("click", () => {
        active.delete(banner.id);
        render();
        onDismiss(banner);
      });
      row.appendChild(close);
      area.appendChild(row);
    }
  }

  return {
    show(banner) {
      if (!banner || !banner.id) return;
      active.set(banner.id, banner);
      render();
    },
    hide(id) {
      if (active.delete(id)) render();
    },
    has: (id) => active.has(id),
  };
}
