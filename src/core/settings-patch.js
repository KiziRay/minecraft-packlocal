/**
 * 設定補丁：前端只送「改了哪個路徑、改成什麼」，由後端 patch_app_settings_cmd
 * 讀現檔、合併、原子寫回。兩個視窗各改各的欄位，不會再整份互相蓋掉。
 *
 * 規則（與 src-tauri/src/engine/app_settings.rs 一致）：
 *  - `{ path, value }` 設值；value 是 null／undefined 時不寫（讀不到值不可蓋掉原本的設定）。
 *  - `{ path, delete: true }` 明確刪除，用在「清除」「改回每次詢問」。
 */

import { isAllowedSettingPath } from "./settings-paths.js";

export function opSet(path, value) {
  return { path: String(path), value: value === undefined ? null : value };
}

export function opDelete(path) {
  return { path: String(path), delete: true };
}

export function getPath(root, dotted) {
  return String(dotted)
    .split(".")
    .reduce((node, part) => (node && typeof node === "object" ? node[part] : undefined), root);
}

/** 把補丁套到記憶體裡的設定物件（前端快取用），回傳實際改動筆數。
 *  白名單外的路徑（含 __proto__／constructor／prototype）一律略過；中間節點不是物件時不覆蓋。 */
export function applyOps(root, ops) {
  if (!root || typeof root !== "object") return 0;
  let changed = 0;
  for (const op of ops || []) {
    if (!op || !isAllowedSettingPath(op.path)) continue;
    const parts = op.path.split(".");
    const last = parts.pop();
    if (op.delete) {
      let node = root;
      for (const part of parts) {
        node = node && typeof node === "object" && Object.prototype.hasOwnProperty.call(node, part) ? node[part] : undefined;
      }
      if (node && typeof node === "object" && Object.prototype.hasOwnProperty.call(node, last)) {
        delete node[last];
        changed += 1;
      }
      continue;
    }
    if (op.value === null || op.value === undefined) continue;
    let node = root;
    let blocked = false;
    for (const part of parts) {
      if (!Object.prototype.hasOwnProperty.call(node, part)) node[part] = {};
      if (!node[part] || typeof node[part] !== "object") {
        blocked = true;
        break;
      }
      node = node[part];
    }
    if (blocked) continue;
    if (node[last] !== op.value) {
      node[last] = op.value;
      changed += 1;
    }
  }
  return changed;
}

/**
 * 舊版只存在 localStorage 的設定，搬進設定檔時要送的補丁。
 *
 * 只補「設定檔還沒有」的欄位：設定檔裡已經有的值代表使用者在新版改過，不能被舊值蓋掉。
 * `legacyAliases` 是 1.0.6／1.0.7 等舊鍵名 → 目前鍵名，目前鍵沒有值時才讀舊鍵。
 */
export function planLocalStorageMigration(fileSettings, keyMap, legacyAliases, readLocal) {
  const ops = [];
  const settings = fileSettings && typeof fileSettings === "object" ? fileSettings : {};
  const legacyFor = {};
  for (const [legacy, current] of Object.entries(legacyAliases || {})) {
    (legacyFor[current] ||= []).push(legacy);
  }
  const planned = new Set();
  for (const [key, dotted] of Object.entries(keyMap || {})) {
    if (planned.has(dotted)) continue;
    const existing = getPath(settings, dotted);
    if (existing !== undefined && existing !== null) continue;
    let raw = readLocal(key);
    if (raw === null || raw === undefined) {
      for (const legacy of legacyFor[key] || []) {
        raw = readLocal(legacy);
        if (raw !== null && raw !== undefined) break;
      }
    }
    if (raw === null || raw === undefined) continue;
    ops.push(opSet(dotted, String(raw)));
    planned.add(dotted);
  }
  return ops;
}
