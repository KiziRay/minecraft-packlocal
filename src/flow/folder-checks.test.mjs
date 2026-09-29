/**
 * B5d 選資料夾就判定的接線（folder-checks.js）：只呼叫唯讀查詢、背景查身分與遊戲、動作對到正確的後端指令。
 */
import test from "node:test";
import assert from "node:assert/strict";

import { createFolderChecks } from "./folder-checks.js";
import { FOLDER_ACTION } from "./folder-state.js";

const PATH = "C:/Games/ATM10";
const okInspection = {
  reachable: true,
  validation: { ok: true, reason: "遊戲資料夾可以翻譯。", hints: [] },
  shape: { kind: "ok", hasOptions: false, candidates: [] },
  write: { writable: true, code: "ok" },
};

function harness({ responses = {}, last = "", confirm = true } = {}) {
  const calls = [];
  const logs = [];
  const adopted = [];
  const retired = [];
  const area = {
    active: new Map(),
    show(b) {
      this.active.set(b.id, b);
    },
    hide(id) {
      this.active.delete(id);
    },
    has(id) {
      return this.active.has(id);
    },
  };
  const elements = {};
  const el = (id) =>
    (elements[id] ||= {
      id,
      hidden: false,
      textContent: "",
      dataset: {},
      attrs: {},
      setAttribute(k, v) {
        this.attrs[k] = v;
      },
      removeAttribute(k) {
        delete this.attrs[k];
      },
    });
  const deps = {
    $: el,
    invoke: async (cmd, args) => {
      calls.push([cmd, args]);
      const r = responses[cmd];
      if (r instanceof Error) throw r;
      return typeof r === "function" ? r(args) : r;
    },
    appendLog: (text, level) => logs.push([text, level]),
    syncUiState: () => {},
    adoptInstancePath: async (p) => adopted.push(p),
    getCurrentPath: () => PATH,
    readLastInstancePath: () => last,
    confirmDialog: async () => confirm,
    disclosure: { retire: (k) => retired.push(k) },
    bannerArea: () => area,
  };
  return { fc: createFolderChecks(deps), calls, logs, adopted, retired, area, el };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

test("inspect：先查資料夾本身，通過後才在背景查身分與遊戲是否開著；全部是唯讀查詢", async () => {
  const h = harness({
    responses: {
      inspect_folder_cmd: okInspection,
      inspect_instance_identity_cmd: { state: "copied", path: "D:/ATM10", originName: "ATM10" },
      is_game_running_cmd: { running: true, known: true },
    },
  });
  const out = await h.fc.inspect(PATH);
  assert.equal(out.ok, true);
  assert.equal(h.fc.gateInput(PATH).identityPending, true, "身分還在查：開始翻譯先停用");
  await flush();
  await flush();
  assert.deepEqual(
    h.calls.map((c) => c[0]),
    ["inspect_folder_cmd", "inspect_instance_identity_cmd", "is_game_running_cmd"]
  );
  const gate = h.fc.gateInput(PATH);
  assert.equal(gate.identityPending, false);
  assert.equal(gate.identity.state, "copied");
  assert.equal(h.fc.hasOptions(PATH), false);
  assert.equal(h.fc.gateInput("D:/Other"), null, "路徑對不上不給（還在打字時交給一般驗證）");
  const readOnly = new Set(["inspect_folder_cmd", "inspect_instance_identity_cmd", "is_game_running_cmd"]);
  assert.ok(h.calls.every(([cmd]) => readOnly.has(cmd)), "選資料夾時只呼叫唯讀查詢");
});

test("inspect：資料夾不對或寫不進去時不再查身分；又選了別的資料夾時舊結果作廢", async () => {
  const bad = harness({
    responses: { inspect_folder_cmd: { ...okInspection, validation: { ok: false, reason: "x" }, write: null } },
  });
  assert.equal((await bad.fc.inspect(PATH)).ok, false);
  await flush();
  assert.deepEqual(bad.calls.map((c) => c[0]), ["inspect_folder_cmd"]);

  const noWrite = harness({
    responses: {
      inspect_folder_cmd: { ...okInspection, write: { writable: false, code: "network", message: "網路磁碟暫時寫不進去" } },
      is_game_running_cmd: { running: false },
    },
  });
  await noWrite.fc.inspect("Y:/packs/ATM10");
  await flush();
  assert.ok(!noWrite.calls.some((c) => c[0] === "inspect_instance_identity_cmd"));
  assert.ok(noWrite.logs.some(([t]) => t.includes("網路磁碟")), "完整原因進紀錄");

  let release;
  const slow = harness({
    responses: { inspect_folder_cmd: () => new Promise((r) => (release = r)) },
  });
  const first = slow.fc.inspect(PATH);
  slow.fc.reset("D:/Other");
  release(okInspection);
  assert.equal((await first).stale, true);
});

test("狀態卡動作：各自對到後端指令；重設紀錄不另跳確認；當成新的要先 D-10 確認", async () => {
  const h = harness({
    responses: {
      inspect_folder_cmd: okInspection,
      reset_apply_record_cmd: "已重設套用紀錄。",
      fork_apply_instance_cmd: "ok",
      relaunch_as_admin_cmd: { relaunching: false },
      open_path: null,
    },
  });
  await h.fc.onAction(FOLDER_ACTION.resetRecord);
  assert.deepEqual(h.calls.at(-1), ["reset_apply_record_cmd", { instancePath: PATH }]);
  assert.deepEqual(h.adopted, [PATH], "重設後重新檢查");
  assert.ok(h.retired.includes("brokenRecord"));

  await h.fc.onAction(FOLDER_ACTION.forkInstance);
  assert.deepEqual(h.calls.at(-1), ["fork_apply_instance_cmd", { instancePath: PATH }]);

  const declined = harness({ confirm: false, responses: { fork_apply_instance_cmd: "ok" } });
  await declined.fc.onAction(FOLDER_ACTION.forkInstance);
  assert.equal(declined.calls.length, 0, "D-10 沒按確認不能動");

  await h.fc.onAction(FOLDER_ACTION.relaunchAdmin);
  assert.deepEqual(h.calls.at(-1), ["relaunch_as_admin_cmd", { instancePath: PATH }]);
  assert.ok(h.logs.some(([t]) => t.includes("已取消")), "UAC 取消不是錯誤");

  await h.fc.onAction(FOLDER_ACTION.openMarker, { path: "C:/Games/ATM10/.mcpl/instance.json" });
  assert.deepEqual(h.calls.at(-1), ["open_path", { path: "C:/Games/ATM10/.mcpl" }]);

  await h.fc.onAction(FOLDER_ACTION.usePath, { path: "C:/Games" });
  assert.equal(h.adopted.at(-1), "C:/Games");

  await h.fc.inspect(PATH);
  await h.fc.onAction(FOLDER_ACTION.serverOverride);
  assert.equal(h.fc.gateInput(PATH).serverOverride, true);
  assert.ok(h.retired.includes("server"));
});

test("橫幅 N-03／N-04：狀態離開或關掉後，這次選資料夾不再出現；換資料夾重新計算", async () => {
  const h = harness({
    responses: { inspect_folder_cmd: okInspection, inspect_instance_identity_cmd: { state: "new" }, is_game_running_cmd: { running: true } },
  });
  await h.fc.inspect(PATH);
  await flush();
  await flush();
  h.fc.syncBanners("READY", PATH);
  assert.deepEqual([...h.area.active.keys()].sort(), ["N-03", "N-04"]);
  h.fc.syncBanners("BUSY", PATH);
  assert.equal(h.area.active.size, 0);
  h.fc.syncBanners("READY", PATH);
  assert.equal(h.area.active.size, 0, "每次選資料夾最多一次");
  await h.fc.inspect(PATH);
  await flush();
  await flush();
  h.fc.syncBanners("READY", PATH);
  assert.equal(h.area.active.size, 2, "重新選資料夾重新計算");
  h.area.hide("N-04");
  h.fc.noteBannerDismissed({ id: "N-04" });
  h.fc.syncBanners("READY", PATH);
  assert.deepEqual([...h.area.active.keys()], ["N-03"]);
});

test("「上次」與瀏覽起始位置：啟動時只讀路徑；沒有上次路徑才查常見啟動器資料夾（只查一次）", async () => {
  const h = harness({ last: "C:/cf/Instances/ATM10", responses: { common_launcher_dir_cmd: "C:/cf/Instances" } });
  h.fc.syncLastButton();
  const btn = h.el("btn-last-instance");
  assert.equal(btn.hidden, false);
  assert.equal(btn.textContent, "上次：ATM10");
  assert.equal(h.calls.length, 0, "畫「上次」不呼叫任何後端（百分比按下後才算）");
  await h.fc.useLast();
  assert.deepEqual(h.adopted, ["C:/cf/Instances/ATM10"]);
  assert.equal(await h.fc.pickStart(), "C:/cf/Instances/ATM10");
  assert.equal(h.calls.length, 0);

  const fresh = harness({ responses: { common_launcher_dir_cmd: "C:/cf/Instances" } });
  assert.equal(await fresh.fc.pickStart(), "C:/cf/Instances");
  assert.equal(await fresh.fc.pickStart(), "C:/cf/Instances");
  assert.equal(fresh.calls.filter((c) => c[0] === "common_launcher_dir_cmd").length, 1);
});

// ─── B5d 審查修正 ───────────────────────────────────────
import { runWhileCurrent } from "./folder-checks.js";
import { readFileSync as readSrc } from "node:fs";

test("審查 2：選資料夾後的步驟每個 await 後確認沒換資料夾；A 慢 B 快時最後狀態全屬 B", async () => {
  const ui = { instance: "", version: "", output: "", probe: "" };
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const pipeline = (path, delay) =>
    runWhileCurrent(path, () => ui.instance, [
      async () => {
        await wait(delay);
        return () => (ui.version = `ver-${path}`);
      },
      async () => {
        await wait(delay);
        return () => (ui.output = `out-${path}`);
      },
      async () => {
        await wait(delay);
        return () => (ui.probe = `probe-${path}`);
      },
    ]);
  ui.instance = "A";
  const a = pipeline("A", 30);
  await wait(5);
  ui.instance = "B";
  const b = pipeline("B", 1);
  assert.deepEqual(await Promise.all([a, b]), [false, true]);
  assert.deepEqual(ui, { instance: "B", version: "ver-B", output: "out-B", probe: "probe-B" });
});

test("審查 2／1b：adoptInstancePath 與手動輸入都用 runWhileCurrent；上次路徑在檢查通過後才記；探測與版本偵測自己也檢查", () => {
  const app = readSrc(new URL("../app.js", import.meta.url), "utf8").replace(/\r\n/g, "\n");
  const body = (sig) => {
    const at = app.indexOf(sig);
    return app.slice(at, app.indexOf("\n}\n", at));
  };
  for (const sig of ["async function adoptInstancePath(", "async function onInstanceTypedPath("]) {
    const b = body(sig);
    assert.ok(b.includes("runWhileCurrent("), `${sig} 要用 runWhileCurrent`);
    const check = b.indexOf("checkSelectedFolder(");
    const remember = b.indexOf("writeLastInstancePath(");
    assert.ok(check >= 0 && remember > check, `${sig}：檢查通過後才記上次路徑`);
  }
  assert.ok(body("async function probeLocalPackCache(").includes("stillSelected("), "探測結果回來時資料夾已換就丟掉");
  assert.ok(body("async function detectVersionForInstance(").includes("stillSelected("), "版本偵測回來時資料夾已換就不改下拉");
});
