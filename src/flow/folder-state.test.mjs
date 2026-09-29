/**
 * B5d 選資料夾就判定（規格 §2.2 S02–S07、S15 暫行、S18、§3.1 MC 版本列、§3.4 N-03／N-04、§1.1 D 區「上次」）。
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { ACTION, SENTENCE_MAX, ZERO_PRIMARY_ALLOWED, computePackState } from "./pack-state.js";
import {
  FOLDER_ACTION,
  FOLDER_STATE,
  WRITE_SENTENCES,
  folderBanners,
  hasUsableExistingResult,
  isUsableProbe,
  lastInstanceButton,
  pickStartPath,
  readyDetailLines,
} from "./folder-state.js";
import { GENERIC_PRIMARY_ID, planStatusCard } from "./status-card.js";
import { DISCLOSURES } from "./disclosure.js";
import { SETTING_PATHS } from "../core/settings-paths.js";
import { findForbidden } from "../copy/terms.js";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel) => readFileSync(join(here, rel), "utf8").replace(/\r\n/g, "\n");

const PATH = "C:/Games/ATM10";
const okValidation = { ok: true, reason: "遊戲資料夾可以翻譯。", hints: [] };
const okShape = { kind: "ok", hasOptions: true, candidates: [] };
const okWrite = { writable: true, code: "ok", needsAdmin: false, path: PATH };
const inspection = (over = {}) => ({ reachable: true, validation: okValidation, shape: okShape, write: okWrite, ...over });
const folder = (over = {}) => ({ inspecting: false, inspection: inspection(), identityPending: false, identity: { state: "ok" }, ...over });
const base = { consentAccepted: true, instancePath: PATH, validation: okValidation };
const stateFor = (f, extra = {}) => computePackState({ ...base, folder: f, ...extra });

const bad = (shape, reason = "這裡找不到 mods，可能選到上一層或下一層。") =>
  inspection({ validation: { ok: false, reason }, shape, write: null });
const candidates = ["a", "b", "c", "d", "e"].map((n) => ({ name: `Pack-${n}`, path: `C:/cf/Instances/Pack-${n}` }));

/** B5d 會出現的所有判定輸入。 */
function gateCases() {
  const out = [
    folder({ inspecting: true, inspection: null }),
    folder({ inspection: bad({ kind: "mods_selected", parent: PATH }) }),
    folder({ inspection: bad({ kind: "launcher_list", candidates }) }),
    folder({ inspection: bad({ kind: "no_mods" }) }),
    folder({ inspection: bad({ kind: "invalid" }, "找不到這個資料夾，請重新選擇。") }),
    folder({ inspection: { ...bad({ kind: "invalid" }, "連不到這個網路磁碟上的資料夾，可能是連線不穩。"), reachable: false } }),
    folder({ inspection: inspection({ shape: { kind: "server", hasOptions: false } }) }),
    folder({ inspection: inspection({ shape: { kind: "server", hasOptions: true } }) }),
    folder({ inspection: inspection({ shape: { kind: "server", hasOptions: true } }), serverOverride: true }),
    folder({ identityPending: true, identity: null }),
    ...["record_broken", "marker_broken", "unreachable", "copied", "new", "unknown", "ok"].map((s) =>
      folder({ identity: { state: s, path: "D:/Old/一個名字非常非常非常非常長的原本那份模組整合包", originName: "一個名字非常非常非常非常長的原本那份模組整合包" } })
    ),
  ];
  for (const code of Object.keys(WRITE_SENTENCES).concat(["weird"])) {
    out.push(folder({ inspection: inspection({ write: { writable: false, code, path: PATH, needsAdmin: code === "needs_admin" } }) }));
  }
  return out;
}

function allInputs() {
  const out = [];
  for (const f of gateCases())
    for (const packChanged of [false, true])
      for (const hasTranslationRecord of [false, true])
        for (const hasResult of [false, true])
          for (const versionUnknown of [false, true])
            out.push({ ...base, folder: f, packChanged, hasTranslationRecord, hasResult, versionUnknown, hasOptions: false });
  return out;
}

test("R-1／R-4／字數：B5d 每個判定恰好 0 或 1 顆主要按鈕（0 只在例外）、次要 ≤3、停用有原因、句子 ≤40 字", () => {
  const seen = new Set();
  const longName = "一個名字非常非常非常非常非常非常長的模組整合包";
  for (const input of allInputs()) {
    const s = computePackState({ ...input, packName: longName });
    seen.add(s.id);
    assert.ok(s.primary || ZERO_PRIMARY_ALLOWED.includes(s.id), `${s.id} 沒有主要按鈕`);
    assert.ok(s.secondary.length <= 3, `${s.id} 次要超過 3 顆`);
    if (s.primary && s.primary.disabled) assert.ok(s.disabledReason, `${s.id} 停用沒寫原因`);
    for (const text of [s.sentence, s.extraLine, s.disabledReason]) {
      assert.ok(Array.from(text || "").length <= SENTENCE_MAX, `${s.id} 超過 40 字：${text}`);
    }
    assert.ok(s.sentence, `${s.id} 沒有現況句`);
  }
  for (const id of ["CHECKING", "S02", "S03", "S04", "S05", "S06", "S07", "S15", "S18", "READY"]) {
    assert.ok(seen.has(id), `組合裡沒有走到 ${id}`);
  }
});

test("S02 (a)：選到 mods 本身 → 一鍵改用上一層", () => {
  const s = stateFor(folder({ inspection: bad({ kind: "mods_selected", parent: PATH }) }));
  assert.equal(s.id, FOLDER_STATE.wrongFolder);
  assert.equal(s.sentence, "你選的是 mods，要選它上一層的遊戲資料夾");
  assert.deepEqual(s.primary, { action: FOLDER_ACTION.usePath, label: "改用「ATM10」", path: PATH });
});

test("S02 (b)：選到 CurseForge 的 Instances → 列出底下的模組整合包（3 顆次要＋其餘在「更多」，共 ≤5）", () => {
  const s = stateFor(folder({ inspection: bad({ kind: "launcher_list", candidates }) }));
  assert.equal(s.sentence, "這裡有好幾個模組整合包，請選一個");
  assert.equal(s.primary.action, ACTION.pickFolder);
  assert.deepEqual(s.secondary.map((b) => b.label), ["Pack-a", "Pack-b", "Pack-c"]);
  assert.deepEqual(s.more.map((b) => b.path), ["C:/cf/Instances/Pack-d", "C:/cf/Instances/Pack-e"]);
  assert.ok([...s.secondary, ...s.more].every((b) => b.action === FOLDER_ACTION.usePath));
});

test("S02 (c)：找不到 mods → 白話原因＋重新選擇；其他原因用後端的句子", () => {
  const s = stateFor(folder({ inspection: bad({ kind: "no_mods" }) }));
  assert.equal(s.sentence, "這裡找不到 mods，可能選到上一層或下一層");
  const gone = stateFor(folder({ inspection: bad({ kind: "invalid" }, "找不到這個資料夾，請重新選擇。") }), {
    validation: { ok: false, reason: "找不到這個資料夾，請重新選擇。" },
  });
  assert.equal(gone.id, "S02");
  assert.match(gone.sentence, /找不到這個資料夾/);
});

test("S03：伺服器資料夾一選就說；沒 options.txt 只能重新選擇，有 options.txt 才給「仍要翻這個資料夾」", () => {
  const server = stateFor(folder({ inspection: inspection({ shape: { kind: "server", hasOptions: false } }) }));
  assert.equal(server.id, FOLDER_STATE.server);
  assert.equal(server.sentence, "這是伺服器資料夾，請選玩家電腦上的遊戲資料夾");
  assert.equal(server.primary.action, ACTION.pickFolder);
  assert.deepEqual(server.secondary, []);
  const withOptions = stateFor(folder({ inspection: inspection({ shape: { kind: "server", hasOptions: true } }) }));
  assert.deepEqual(withOptions.secondary, [{ action: FOLDER_ACTION.serverOverride, label: "仍要翻這個資料夾" }]);
  const overridden = stateFor(folder({ inspection: inspection({ shape: { kind: "server", hasOptions: true } }), serverOverride: true }));
  assert.equal(overridden.id, "READY");
});

test("S04：寫入錯誤依分類碼說原因；只有 needs_admin 給管理員鈕，網路磁碟只有「重新檢查」", () => {
  const all = (s) => [s.primary, ...s.secondary, ...s.more].filter(Boolean);
  for (const code of Object.keys(WRITE_SENTENCES)) {
    const s = stateFor(folder({ inspection: inspection({ write: { writable: false, code, path: PATH } }) }));
    assert.equal(s.id, FOLDER_STATE.cannotWrite);
    assert.equal(s.sentence, WRITE_SENTENCES[code]);
    const admin = all(s).some((b) => b.action === FOLDER_ACTION.relaunchAdmin);
    assert.equal(admin, code === "needs_admin", `${code} 的管理員鈕`);
    if (code !== "needs_admin") assert.doesNotMatch(s.sentence, /系統管理員|權限/, code);
  }
  const net = stateFor(folder({ inspection: inspection({ write: { writable: false, code: "network", path: "Y:/packs/ATM10" } }) }));
  assert.equal(net.sentence, "網路磁碟暫時寫不進去，可能是連線不穩");
  assert.deepEqual(net.primary, { action: FOLDER_ACTION.recheck, label: "重新檢查" });
  const unreachable = stateFor(
    folder({ inspection: { ...bad({ kind: "invalid" }, "連不到這個網路磁碟上的資料夾，可能是連線不穩。"), reachable: false } })
  );
  assert.equal(unreachable.primary.action, FOLDER_ACTION.recheck, "連不到這個資料夾本身：重新檢查，不卡畫面");
});

test("S05：紀錄壞了 → 重設套用紀錄（按鈕本身就是動作）；記號壞了 → 開啟記號所在位置＋修復方法", () => {
  const record = stateFor(folder({ identity: { state: "record_broken", path: "C:/tool/apply-records/x/套用紀錄.json" } }));
  assert.equal(record.id, FOLDER_STATE.brokenRecord);
  assert.equal(record.sentence, "套用紀錄壞了，先停下，沒動任何檔案");
  assert.equal(record.primary.action, FOLDER_ACTION.resetRecord);
  const marker = stateFor(folder({ identity: { state: "marker_broken", path: "C:/Games/ATM10/.mcpl/instance.json" } }));
  assert.equal(marker.sentence, "工具記號壞了，先停下，沒動任何檔案");
  assert.equal(marker.primary.action, FOLDER_ACTION.openMarker);
  assert.ok(marker.detailLines.some((l) => l.includes("instance.json")));
});

test("S06、S07：原位置連不到／整份複製來的，一選就說（不是翻完才擋）", () => {
  const unreachable = stateFor(folder({ identity: { state: "unreachable", path: "E:/Packs/ATM10" } }));
  assert.equal(unreachable.id, FOLDER_STATE.originUnreachable);
  assert.equal(unreachable.sentence, "上次的位置連不到（外接硬碟沒接上？）");
  assert.equal(unreachable.primary.label, "接上後重新檢查");
  assert.equal(unreachable.secondary[0].action, FOLDER_ACTION.forkInstance);
  assert.ok(unreachable.detailLines.includes("E:/Packs/ATM10"), "路徑單獨一行");
  const copied = stateFor(folder({ identity: { state: "copied", path: "D:/Packs/ATM10", originName: "ATM10" } }));
  assert.equal(copied.id, FOLDER_STATE.copied);
  assert.equal(copied.sentence, "這份是從「ATM10」複製來的，要先分開記錄");
  assert.equal(copied.primary.action, FOLDER_ACTION.forkInstance);
  assert.deepEqual(copied.secondary, [{ action: FOLDER_ACTION.usePath, label: "改選原本那份", path: "D:/Packs/ATM10" }]);
  for (const s of ["new", "unknown", "ok"]) assert.equal(stateFor(folder({ identity: { state: s } })).id, "READY", s);
});

test("檢查中與身分確認中：開始翻譯停用並寫原因（S07 必須在開始翻譯之前出現）", () => {
  const checking = stateFor(folder({ inspecting: true, inspection: null }));
  assert.equal(checking.id, FOLDER_STATE.checking);
  assert.ok(checking.primary.disabled && checking.disabledReason);
  const pending = stateFor(folder({ identityPending: true, identity: null }));
  assert.equal(pending.id, FOLDER_STATE.checking);
  assert.ok(pending.primary.disabled);
});

test("S15 暫行：模組整合包有變動 → 「上次翻譯後有變動，要重新翻譯」，不當成已有結果（不跳三選一）", () => {
  const s = computePackState({ ...base, packName: "ATM10", packChanged: true, hasResult: false });
  assert.equal(s.id, FOLDER_STATE.packChanged);
  assert.equal(s.sentence, "「ATM10」上次翻譯後有變動，要重新翻譯");
  assert.equal(s.primary.action, ACTION.run);
  assert.equal(s.primary.label, "重新翻譯");
  assert.deepEqual(s.more.map((m) => m.action), [ACTION.deleteAndRestart]);
  assert.equal(computePackState({ ...base, packChanged: true, translationComplete: true }).id, "READY", "重新翻完就不再說有變動");
  const changed = { status: "changed", modsChanged: true, shareable: false };
  assert.equal(hasUsableExistingResult({ probe: changed, hasShareableFiles: true }), false);
  assert.equal(hasUsableExistingResult({ probe: null, hasShareableFiles: true, packChanged: true }), false);
  assert.equal(hasUsableExistingResult({ probe: { status: "ready" } }), true);
  assert.equal(hasUsableExistingResult({ probe: null, hasShareableFiles: true }), true, "沒變動時照舊");
  assert.equal(isUsableProbe(changed), false, "本機已有翻譯卡不顯示有變動的結果");
  assert.equal(isUsableProbe({ status: "partial" }), true);
});

test("S18：已套用、這台電腦沒留結果 → 沒有主要按鈕，次要「重新翻譯」", () => {
  const s = computePackState({ ...base, hasTranslationRecord: true, hasResult: false });
  assert.equal(s.id, FOLDER_STATE.appliedNoResult);
  assert.equal(s.primary, null);
  assert.ok(ZERO_PRIMARY_ALLOWED.includes(s.id));
  assert.deepEqual(s.secondary, [{ action: ACTION.run, label: "重新翻譯" }]);
  assert.equal(computePackState({ ...base, hasTranslationRecord: true, hasResult: true }).id, "READY");
});

test("§3.1 MC 版本列：偵測不到 → 狀態卡出下拉、開始翻譯停用並寫原因", () => {
  const s = computePackState({ ...base, versionUnknown: true });
  assert.equal(s.id, "READY");
  assert.ok(s.primary.disabled);
  assert.equal(s.disabledReason, "偵測不到 Minecraft 版本，請選一個");
  assert.equal(planStatusCard(s).versionRow, true);
  assert.equal(planStatusCard(computePackState(base)).versionRow, false);
});

test("「建議先啟動一次遊戲」只在不知道有沒有 options.txt 時留著；沒有時改由 N-04 說（同一件事只一處）", () => {
  assert.deepEqual(readyDetailLines({ hasOptions: false }), []);
  assert.deepEqual(readyDetailLines({ hasOptions: true }), []);
  assert.equal(readyDetailLines({ hasOptions: null }).length, 1);
  assert.deepEqual(computePackState({ ...base, hasOptions: false }).detailLines, []);
});

test("N-03／N-04：只在可開始／有變動時出現；每次選資料夾最多一次；其他狀態收掉", () => {
  const shown = (o) => folderBanners(o).show.map((b) => b.id);
  assert.deepEqual(shown({ stateId: "READY", gameRunning: true, hasOptions: false }), ["N-03", "N-04"]);
  assert.deepEqual(shown({ stateId: "S15", gameRunning: true, hasOptions: true }), ["N-03"]);
  for (const id of ["S04", "S07", "BUSY", "S09", "S11-card", "S01"]) {
    assert.deepEqual(shown({ stateId: id, gameRunning: true, hasOptions: false }), [], id);
    assert.deepEqual(folderBanners({ stateId: id }).hide.sort(), ["N-03", "N-04"]);
  }
  assert.deepEqual(shown({ stateId: "READY", gameRunning: true, hasOptions: false, dismissed: ["N-03", "N-04"] }), []);
  assert.equal(folderBanners({ stateId: "READY", gameRunning: true }).show[0].text, "遊戲正在執行。可以先翻，套用前要關遊戲");
  assert.equal(folderBanners({ stateId: "READY", hasOptions: false }).show[0].text, "這個遊戲還沒啟動過，套用前要先開一次");
});

test("D 區「上次：<包名>」：只讀路徑與包名；已選同一個時不出現；翻譯中停用並寫原因", () => {
  assert.deepEqual(lastInstanceButton({ lastPath: "C:\\cf\\Instances\\ATM10\\", currentPath: "" }), {
    visible: true,
    label: "上次：ATM10",
    path: "C:\\cf\\Instances\\ATM10\\",
    disabledReason: "",
  });
  assert.equal(lastInstanceButton({ lastPath: "C:/cf/Instances/ATM10", currentPath: "c:\\cf\\instances\\atm10\\" }).visible, false);
  assert.equal(lastInstanceButton({ lastPath: "" }).visible, false);
  assert.equal(lastInstanceButton({ lastPath: "C:/x/A", locked: true, lockReason: "正在翻「B」，翻完才能換資料夾" }).disabledReason, "正在翻「B」，翻完才能換資料夾");
  assert.equal(pickStartPath({ lastPath: "C:/x/A", launcherDir: "C:/cf" }), "C:/x/A");
  assert.equal(pickStartPath({ lastPath: "", launcherDir: "C:/cf" }), "C:/cf");
});

test("狀態卡：B5d 的主要動作共用 #btn-card-action（動作與路徑寫在 data-*），同一時間只露出一顆", () => {
  const s = stateFor(folder({ inspection: bad({ kind: "mods_selected", parent: PATH }) }));
  const plan = planStatusCard(s);
  const visible = Object.entries(plan.buttons).filter(([, b]) => !b.hidden);
  assert.equal(visible.length, 1);
  assert.equal(visible[0][0], GENERIC_PRIMARY_ID);
  assert.equal(plan.buttons[GENERIC_PRIMARY_ID].action, FOLDER_ACTION.usePath);
  assert.equal(plan.buttons[GENERIC_PRIMARY_ID].path, PATH);
  const pick = planStatusCard(stateFor(folder({ inspection: bad({ kind: "no_mods" }) })));
  assert.equal(pick.buttons["btn-card-pick"].hidden, false);
  assert.equal(pick.buttons[GENERIC_PRIMARY_ID].hidden, true);
});

test("附加說明漸進退場：新說明有獨立 key，設定路徑在前後端共用白名單（G0.2）與 KEY_MAP", () => {
  const store = read("../core/settings-store.js");
  for (const key of ["server", "brokenRecord", "copied", "packChanged"]) {
    const d = DISCLOSURES[key];
    assert.ok(d && d.topic, key);
    assert.ok(SETTING_PATHS.includes(d.settingPath), d.settingPath);
    assert.ok(store.includes(`"${d.storageKey}": "${d.settingPath}"`), d.storageKey);
  }
  const shown = computePackState({ ...base, packChanged: true, extraShown: () => true });
  assert.equal(shown.disclosureKey, "packChanged");
  assert.ok(shown.extraLine);
  const retired = computePackState({ ...base, packChanged: true, extraShown: () => false });
  assert.equal(retired.extraLine, "");
  assert.equal(planStatusCard(retired).extra.canRecall, true, "退場後留「？」");
});

test("畫面結構：接續卡、寫入權限卡已刪；「上次」在 D 區；新主要按鈕與版本列在狀態卡內", () => {
  const html = read("../index.html");
  for (const id of ["resume-card", "write-access-card", "btn-resume-continue", "btn-relaunch-admin", "btn-write-access-dismiss"]) {
    assert.ok(!html.includes(`id="${id}"`), `${id} 應已刪除`);
  }
  const pathBlock = html.slice(html.indexOf('<div class="path-block">'), html.indexOf('<section id="status-card"'));
  assert.ok(pathBlock.includes('id="btn-last-instance"'), "「上次」在 D 區");
  const card = html.slice(html.indexOf('<section id="status-card"'), html.indexOf("</section>", html.indexOf('<section id="status-card"')));
  assert.ok(card.includes('id="btn-card-action"') && card.includes('id="status-card-version"'));
  const app = read("../app.js");
  for (const gone of ["offerResumeUnfinishedRun", "checkWriteAccessFor", "wireWriteAccessCard", "wireResumeCard", '"check_write_access_cmd"']) {
    assert.ok(!app.includes(gone), `app.js 不應再有 ${gone}`);
  }
  const css = read("../styles/translate.css");
  assert.ok(!css.includes(".resume-card") && !css.includes(".write-access-card"), "舊卡的 CSS 一起刪");
});

test("手動輸入與瀏覽同一套檢查；開始翻譯的三選一與本機已有翻譯卡都不把「有變動」當成可用結果", () => {
  const app = read("../app.js");
  const body = (sig) => {
    const at = app.indexOf(sig);
    assert.ok(at >= 0, sig);
    return app.slice(at, app.indexOf("\n}\n", at));
  };
  assert.ok(body("async function adoptInstancePath(").includes("checkSelectedFolder("));
  assert.ok(body("async function onInstanceTypedPath(").includes("checkSelectedFolder("));
  assert.ok(body("async function checkSelectedFolder(").includes("folderChecks.inspect("));
  assert.ok(body("async function onRunInner(").includes("hasUsableExistingResult("));
  assert.ok(body("function showLocalCacheCard(").includes("isUsableProbe("));
  assert.ok(body("async function onPickInstance(").includes("folderChecks.pickStart()"));
  assert.ok(!body("async function restoreLastInstanceOnStartup(").includes("probe_local_pack_cache_cmd"), "啟動時不探測（百分比按下後才算）");
});

test("詞表：B5d 的判定文案（前端狀態、後端驗證與寫入原因、D-10）禁用詞 0 命中", () => {
  const literals = (text) =>
    [...String(text).replace(/\/\*[\s\S]*?\*\//g, " ").replace(/(^|[^:"'`\\])\/\/.*$/gm, "$1").matchAll(/"((?:[^"\\\n]|\\.)*)"|`((?:[^`\\]|\\.)*)`/g)]
      .map((m) => m[1] ?? m[2])
      .filter((s) => /[一-鿿]/.test(s))
      .join("\n");
  const fork = read("../ui/apply-pending.js");
  const forkBody = fork.slice(fork.indexOf("export async function offerForkInstance"), fork.indexOf("if (!ok) return false;", fork.indexOf("export async function offerForkInstance")));
  const sources = [
    literals(read("./folder-state.js")),
    literals(read("./folder-checks.js")),
    literals(forkBody),
    literals(read("../../src-tauri/src/engine/instance_validate.rs")),
    literals(read("../../src-tauri/src/engine/folder_check.rs")),
  ].join("\n");
  assert.deepEqual(findForbidden(sources), []);
});

// ─── B5d 審查修正 ───────────────────────────────────────
import { versionUnknown } from "./folder-state.js";

test("審查 5a：偵測不到版本時，所有會開始翻譯的狀態都停用（S15、S18 的次要、S19b、可開始）", () => {
  const changed = computePackState({ ...base, packChanged: true, versionUnknown: true });
  assert.equal(changed.id, "S15");
  assert.ok(changed.primary.disabled && changed.showVersionRow);
  assert.equal(changed.disabledReason, "偵測不到 Minecraft 版本，請選一個");
  const s18 = computePackState({ ...base, hasTranslationRecord: true, versionUnknown: true });
  assert.equal(s18.id, "S18");
  assert.ok(s18.secondary.every((b) => b.action !== ACTION.run || b.disabled), "S18 的重新翻譯也要停用");
  assert.ok(s18.disabledReason && s18.showVersionRow);
  assert.equal(planStatusCard(s18).reason, "偵測不到 Minecraft 版本，請選一個", "次要停用也要寫原因");
  const s19b = computePackState({ ...base, versionUnknown: true, removal: { instancePath: base.instancePath, hasResult: false, result: {} } });
  assert.equal(s19b.id, "S19b");
  assert.ok(s19b.primary.disabled);
  const checking = stateFor(folder({ inspecting: true, inspection: null }), { versionUnknown: true });
  assert.equal(checking.disabledReason, "檢查完才能開始", "已停用的狀態保留自己的原因");
});

test("審查 5b：上一個資料夾自動偵測留下的版本，不算這個資料夾選好了", () => {
  assert.equal(versionUnknown({ detectFailed: true, selectValue: "1.20.1", autoDetected: "true" }), true);
  assert.equal(versionUnknown({ detectFailed: true, selectValue: "1.20.1", autoDetected: "false" }), false, "玩家自己選的算");
  assert.equal(versionUnknown({ detectFailed: true, selectValue: "", autoDetected: "false" }), true);
  assert.equal(versionUnknown({ detectFailed: false, selectValue: "", autoDetected: "true" }), false, "偵測成功或還沒偵測時不擋");
  const app = read("../app.js");
  assert.ok(app.includes("versionUnknown({"), "app.js 用同一個判斷");
  const adopt = app.slice(app.indexOf("async function adoptInstancePath("), app.indexOf("\n}\n", app.indexOf("async function adoptInstancePath(")));
  assert.ok(adopt.includes("clearAutoDetectedVersion("), "換資料夾時清掉上一個的自動值");
});
