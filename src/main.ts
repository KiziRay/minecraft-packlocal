import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

const $ = <T extends HTMLElement>(id: string) =>
  document.getElementById(id) as T | null;

function log(msg: string) {
  const el = $("log");
  if (el) el.textContent = msg;
}

async function pickDir(target: "instance" | "output") {
  const selected = await open({
    directory: true,
    multiple: false,
    title: target === "instance" ? "選擇遊戲實例或 minecraft 資料夾" : "選擇輸出目錄",
  });
  if (typeof selected === "string") {
    const id = target === "instance" ? "instance" : "output";
    const input = $(id) as HTMLInputElement | null;
    if (input) input.value = selected;
  }
}

async function scanOnly() {
  const instance = ($("instance") as HTMLInputElement)?.value?.trim();
  if (!instance) {
    log("請先選擇實例路徑");
    return;
  }
  log("掃描中…");
  try {
    const report = await invoke<Record<string, unknown>>("scan_only", {
      instancePath: instance,
      useOpencc: ($("opencc") as HTMLInputElement)?.checked ?? true,
    });
    log(JSON.stringify(report, null, 2));
  } catch (e) {
    log("掃描失敗：\n" + String(e));
  }
}

async function runOneClick() {
  const instance = ($("instance") as HTMLInputElement)?.value?.trim();
  const output = ($("output") as HTMLInputElement)?.value?.trim();
  if (!instance || !output) {
    log("請填寫實例路徑與輸出目錄");
    return;
  }
  log("一鍵翻譯執行中（掃 jar + OpenCC，可能要幾分鐘）…");
  try {
    const result = await invoke<Record<string, unknown>>("one_click_translate", {
      instancePath: instance,
      outputDir: output,
      packName: ($("pack-name") as HTMLInputElement)?.value ?? "",
      packDescription: ($("pack-desc") as HTMLInputElement)?.value ?? "",
      useOpencc: ($("opencc") as HTMLInputElement)?.checked ?? true,
      stripOfZhi: ($("strip-zhi") as HTMLInputElement)?.checked ?? true,
      fixMinemenu: ($("fix-menu") as HTMLInputElement)?.checked ?? true,
      dictPath: null,
    });
    log(
      "完成！\n\n" +
        JSON.stringify(result, null, 2) +
        "\n\n請把產出的 resourcepacks 資料夾放進遊戲並啟用資源包，語言選繁中（台灣）。"
    );
  } catch (e) {
    log("失敗：\n" + String(e));
  }
}

window.addEventListener("DOMContentLoaded", () => {
  $("pick-instance")?.addEventListener("click", () => pickDir("instance"));
  $("pick-output")?.addEventListener("click", () => pickDir("output"));
  $("scan")?.addEventListener("click", () => scanOnly());
  $("run")?.addEventListener("click", () => runOneClick());
});
