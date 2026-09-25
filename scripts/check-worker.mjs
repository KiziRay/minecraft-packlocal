// 對 worker/src 下每個 .js／.mjs 做語法檢查。
// 取代原本在 package.json 手動列檔的寫法：新增或刪除 Worker 模組時不必再改腳本。
import { readdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { join } from "node:path";

const dir = join("worker", "src");
const files = readdirSync(dir)
  .filter((name) => name.endsWith(".js") || name.endsWith(".mjs"))
  .sort();

if (files.length === 0) {
  console.error(`check:worker 找不到任何 Worker 原始檔：${dir}`);
  process.exit(1);
}

let failed = 0;
for (const name of files) {
  const file = join(dir, name);
  const result = spawnSync(process.execPath, ["--check", file], { encoding: "utf8" });
  if (result.status !== 0) {
    failed += 1;
    console.error(`語法錯誤：${file}\n${result.stderr || result.stdout}`);
  }
}

if (failed > 0) {
  console.error(`check:worker 失敗：${failed}／${files.length} 個檔案有語法錯誤`);
  process.exit(1);
}
console.log(`check:worker ok：${files.length} 個檔案語法正確`);
