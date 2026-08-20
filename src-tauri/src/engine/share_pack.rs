//! 把「翻譯結果」工作目錄打包成帶密碼自解 exe（NanaZip），供 R2 分享。
//!
//! Allowlist 同可安裝內容；不含說明／session／日誌。
//! 密碼固定 `cloud.zeitfrei.uk`；找不到 NanaZip 明確報錯，不静默改無密碼 zip。

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::security::sanitize_folder_name;

/// 分享自解檔固定密碼（落地頁會顯示）。
pub const SHARE_SFX_PASSWORD: &str = "cloud.zeitfrei.uk";
/// 分享檔軟頂（4GiB）；超過仍拒絕以免打爆 R2。大檔走 R2 multipart，不受 Worker 100MB body 限制。
pub const SHARE_MAX_UPLOAD_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Multipart 每 part 大小（須 < Worker body 上限；非最後一塊 R2 要求 ≥5MiB）。
pub const SHARE_MPU_PART_BYTES: usize = 20 * 1024 * 1024;
/// 包內捷徑檔名（套用後複製到實例 minecraft 根目錄）。
pub const CLOUD_URL_SHORTCUT_NAME: &str = "ZeitFrei雲端.url";
/// 本機 SFX 與下載檔名（R2 物件鍵仍用長 token）。
pub const SHARE_SFX_FILENAME: &str = "模組包繁中翻譯自解檔.exe";
const CLOUD_URL: &str = "https://cloud.zeitfrei.uk/";
const APPLY_SCRIPT_NAME: &str = "套用翻譯.ps1";
const SFX_CONFIG_NAME: &str = "sfx_config.txt";

#[cfg(windows)]
fn hide_console(cmd: &mut Command) -> &mut Command {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(CREATE_NO_WINDOW).stdin(Stdio::null())
}

#[cfg(not(windows))]
fn hide_console(cmd: &mut Command) -> &mut Command {
    cmd.stdin(Stdio::null())
}

/// 工作目錄是否至少有一個可分享檔（資源包／覆寫等；不含說明／session）。
pub fn has_shareable_content(work_root: &Path) -> bool {
    if !work_root.is_dir() {
        return false;
    }
    WalkDir::new(work_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|e| e.path().is_file() && is_shareable_path(work_root, e.path()))
}

/// 打包 `work_root` 成 `dest_dir/<name>.zip`（測試／除錯用；正式分享走 SFX）。
pub fn package_translation(work_root: &Path, dest_dir: &Path, name: &str) -> Result<PathBuf, String> {
    if !work_root.is_dir() {
        return Err("找不到翻譯結果資料夾，請先完成翻譯再打包。".into());
    }
    if !has_shareable_content(work_root) {
        return Err("翻譯結果是空的，沒有可打包的檔案。".into());
    }
    if dest_dir.as_os_str().is_empty() {
        return Err("請先選擇打包檔要存到哪個資料夾。".into());
    }

    std::fs::create_dir_all(dest_dir).map_err(|e| format!("無法建立輸出資料夾：{e}"))?;
    let safe = match sanitize_folder_name(name) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => "模組包翻譯分享".to_string(),
    };
    let zip_path = dest_dir.join(format!("{safe}.zip"));
    zip_dir(work_root, &zip_path)?;
    Ok(zip_path)
}

/// 用 NanaZip 打帶密碼自解 exe；內含 allowlist＋雲端捷徑＋套用腳本。
pub fn package_translation_sfx(work_root: &Path, dest_dir: &Path, name: &str) -> Result<PathBuf, String> {
    if !work_root.is_dir() {
        return Err("找不到翻譯結果資料夾，請先完成翻譯再打包。".into());
    }
    if !has_shareable_content(work_root) {
        return Err("翻譯結果是空的，沒有可打包的檔案。".into());
    }
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("無法建立輸出資料夾：{e}"))?;

    let nanazip = find_nanazip_cli()?;
    let sfx_module = find_nanazip_sfx_module()?;
    let _ = name;

    let stage = std::env::temp_dir().join(format!(
        "modpack-i18n-sfx-stage-{}-{}",
        std::process::id(),
        epoch_millis()
    ));
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&stage).map_err(|e| format!("無法建立暫存：{e}"))?;

    let result = (|| {
        stage_shareable_files(work_root, &stage)?;
        write_cloud_url_shortcut(&stage.join(CLOUD_URL_SHORTCUT_NAME))?;
        write_apply_script(&stage.join(APPLY_SCRIPT_NAME))?;
        let config_path = stage.join(SFX_CONFIG_NAME);
        write_sfx_config(&config_path)?;

        let archive_7z = stage.join("_payload.7z");
        let mut cmd = Command::new(&nanazip);
        hide_console(&mut cmd);
        let output = cmd
            .current_dir(&stage)
            .args([
                "a",
                "-t7z",
                &format!("-p{SHARE_SFX_PASSWORD}"),
                "-mhe=on",
                "-mx5",
                "-y",
            ])
            .arg(&archive_7z)
            .arg(APPLY_SCRIPT_NAME)
            .arg(CLOUD_URL_SHORTCUT_NAME)
            .args(shareable_top_level_args(&stage)?)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("無法執行 NanaZip：{e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.trim();
            let code = output.status.code().unwrap_or(-1);
            return Err(if detail.is_empty() {
                format!("NanaZip 壓縮失敗（結束碼 {code}），無法建立自解檔。")
            } else {
                let clipped: String = detail.chars().take(240).collect();
                format!("NanaZip 壓縮失敗（結束碼 {code}）：{clipped}")
            });
        }
        if !archive_7z.is_file() {
            return Err("NanaZip 未產出壓縮檔。".into());
        }

        let exe_path = dest_dir.join(SHARE_SFX_FILENAME);
        combine_sfx(&sfx_module, &config_path, &archive_7z, &exe_path)?;
        Ok(exe_path)
    })();

    let _ = fs::remove_dir_all(&stage);
    result
}

fn shareable_top_level_args(stage: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for entry in fs::read_dir(stage).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == APPLY_SCRIPT_NAME
            || name == CLOUD_URL_SHORTCUT_NAME
            || name == SFX_CONFIG_NAME
            || name.ends_with(".7z")
        {
            continue;
        }
        out.push(entry.path());
    }
    if out.is_empty() {
        return Err("暫存區沒有可分享的安裝檔。".into());
    }
    Ok(out)
}

fn stage_shareable_files(work_root: &Path, stage: &Path) -> Result<(), String> {
    stage_resourcepacks_as_zips(work_root, stage)?;
    for entry in WalkDir::new(work_root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() || !is_shareable_path(work_root, path) {
            continue;
        }
        let rel = path
            .strip_prefix(work_root)
            .map_err(|e| e.to_string())?;
        if rel
            .components()
            .next()
            .and_then(|part| part.as_os_str().to_str())
            == Some("resourcepacks")
        {
            continue;
        }
        let dest = stage.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::copy(path, &dest).map_err(|e| format!("複製失敗 {}：{e}", path.display()))?;
    }
    Ok(())
}

/// `resourcepacks/` 頂層只留 zip：資料夾打成同名 zip，既有 zip 原樣複製。
fn stage_resourcepacks_as_zips(work_root: &Path, stage: &Path) -> Result<(), String> {
    let rp = work_root.join("resourcepacks");
    if !rp.is_dir() {
        return Ok(());
    }
    let dest_rp = stage.join("resourcepacks");
    fs::create_dir_all(&dest_rp).map_err(|e| e.to_string())?;

    let mut existing_zips = Vec::new();
    for entry in fs::read_dir(&rp).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.to_ascii_lowercase().ends_with(".zip") {
            continue;
        }
        fs::copy(&path, dest_rp.join(&name))
            .map_err(|e| format!("複製失敗 {}：{e}", path.display()))?;
        existing_zips.push(name_str.to_ascii_lowercase());
    }

    for entry in fs::read_dir(&rp).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let folder_name = entry.file_name();
        let zip_name = format!("{}.zip", folder_name.to_string_lossy());
        if existing_zips
            .iter()
            .any(|n| n == &zip_name.to_ascii_lowercase())
        {
            continue;
        }
        zip_directory_contents(&path, &dest_rp.join(&zip_name))?;
    }
    Ok(())
}

fn zip_directory_contents(src: &Path, zip_path: &Path) -> Result<(), String> {
    let file = File::create(zip_path).map_err(|e| format!("無法建立資源包 zip：{e}"))?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut wrote = false;
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path == src {
            continue;
        }
        let rel = path.strip_prefix(src).map_err(|e| e.to_string())?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if path.is_dir() {
            let _ = zip.add_directory(format!("{}/", rel_str.trim_end_matches('/')), opts);
        } else if path.is_file() {
            zip.start_file(rel_str, opts).map_err(|e| e.to_string())?;
            let bytes = fs::read(path).map_err(|e| format!("讀取失敗 {}：{e}", path.display()))?;
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
            wrote = true;
        }
    }
    zip.finish().map_err(|e| format!("打包資源包失敗：{e}"))?;
    if !wrote {
        let _ = fs::remove_file(zip_path);
    }
    Ok(())
}

fn write_cloud_url_shortcut(path: &Path) -> Result<(), String> {
    let body = format!(
        "[InternetShortcut]\r\nURL={CLOUD_URL}\r\nIconIndex=0\r\n"
    );
    fs::write(path, body).map_err(|e| format!("寫入雲端捷徑失敗：{e}"))
}

fn write_apply_script(path: &Path) -> Result<(), String> {
    // 接收端：提醒選 Minecraft 目錄 → 依 allowlist 複製（對齊 apply_to_instance）→ 捷徑放到根目錄
    let script = r#"$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
$root = if ($args.Count -ge 1 -and $args[0]) { $args[0] } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
$root = (Resolve-Path $root).Path
$dialog = New-Object System.Windows.Forms.FolderBrowserDialog
$dialog.Description = '請選擇 Minecraft 遊戲資料夾（實例根目錄或 .minecraft）。翻譯會自動套用。'
$dialog.ShowNewFolderButton = $false
if ($dialog.ShowDialog() -ne [System.Windows.Forms.DialogResult]::OK) {
  [System.Windows.Forms.MessageBox]::Show('已取消套用。', '模組包翻譯') | Out-Null
  exit 1
}
$mc = $dialog.SelectedPath
function Ensure-Dir([string]$p) { if (-not (Test-Path $p)) { New-Item -ItemType Directory -Path $p -Force | Out-Null } }
function Copy-Tree([string]$src, [string]$dst) {
  if (-not (Test-Path $src)) { return }
  Ensure-Dir $dst
  Copy-Item -Path (Join-Path $src '*') -Destination $dst -Recurse -Force -ErrorAction SilentlyContinue
}
$rp = Join-Path $root 'resourcepacks'
if (Test-Path $rp) {
  Ensure-Dir (Join-Path $mc 'resourcepacks')
  Copy-Item -Path (Join-Path $rp '*') -Destination (Join-Path $mc 'resourcepacks') -Recurse -Force -ErrorAction SilentlyContinue
}
foreach ($name in @('config','patchouli_books','kubejs','minemenu','datapacks','defaultconfigs','global_packs','paxi','data')) {
  $src = Join-Path $root $name
  if (Test-Path $src) { Copy-Tree $src (Join-Path $mc $name) }
}
$url = Join-Path $root 'ZeitFrei雲端.url'
if (Test-Path $url) {
  Copy-Item -Path $url -Destination (Join-Path $mc 'ZeitFrei雲端.url') -Force -ErrorAction SilentlyContinue
}
[System.Windows.Forms.MessageBox]::Show("翻譯已套用到：`n$mc`n`n請關閉遊戲後重開，語言選繁體中文（台灣），並啟用翻譯資源包。", '模組包翻譯') | Out-Null
"#;
    fs::write(path, script).map_err(|e| format!("寫入套用腳本失敗：{e}"))
}

fn write_sfx_config(path: &Path) -> Result<(), String> {
    let config = format!(
        ";!@Install@!UTF-8!\r\n\
Title=\"模組包翻譯套用\"\r\n\
BeginPrompt=\"請選擇 Minecraft 遊戲資料夾後自動套用翻譯。解壓密碼請見下載頁（{SHARE_SFX_PASSWORD}）。\"\r\n\
ExtractTitle=\"解壓翻譯檔\"\r\n\
ExtractDialogText=\"正在解壓…\"\r\n\
GUIFlags=\"8+32+64+256+4096\"\r\n\
OverwriteMode=\"2\"\r\n\
RunProgram=\"powershell.exe -NoProfile -ExecutionPolicy Bypass -File \\\"%%T\\\\{APPLY_SCRIPT_NAME}\\\" \\\"%%T\\\"\"\r\n\
;!@InstallEnd@!\r\n"
    );
    fs::write(path, config).map_err(|e| format!("寫入 SFX 設定失敗：{e}"))
}

fn combine_sfx(sfx_module: &Path, config: &Path, archive: &Path, exe_path: &Path) -> Result<(), String> {
    let mut out = File::create(exe_path).map_err(|e| format!("無法建立自解檔：{e}"))?;
    for part in [sfx_module, config, archive] {
        let mut f = File::open(part).map_err(|e| format!("讀取 {} 失敗：{e}", part.display()))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)
            .map_err(|e| format!("讀取 {} 失敗：{e}", part.display()))?;
        out.write_all(&buf)
            .map_err(|e| format!("寫入自解檔失敗：{e}"))?;
    }
    out.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn find_nanazip_cli() -> Result<PathBuf, String> {
    let candidates = [
        "NanaZipC",
        "NanaZipC.exe",
        "nanazipc",
        "7z",
        "7z.exe",
    ];
    for name in candidates {
        let mut cmd = Command::new("where");
        hide_console(&mut cmd);
        if let Ok(output) = cmd.arg(name).output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = text.lines().next() {
                    let p = PathBuf::from(line.trim());
                    if p.is_file() {
                        return Ok(p);
                    }
                }
            }
        }
    }
    let prog = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    for rel in ["NanaZip/NanaZipC.exe", "7-Zip/7z.exe"] {
        let p = PathBuf::from(&prog).join(rel);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(
        "找不到 NanaZip／NanaZipC，無法建立帶密碼自解 exe。請先安裝 NanaZip 後再分享（不會改成無密碼 zip）。"
            .into(),
    )
}

fn find_nanazip_sfx_module() -> Result<PathBuf, String> {
    let mut roots = Vec::new();
    if let Ok(pf) = std::env::var("ProgramFiles") {
        roots.push(PathBuf::from(&pf).join("NanaZip"));
        roots.push(PathBuf::from(&pf).join("7-Zip"));
    }
    roots.push(PathBuf::from(r"C:\Program Files\NanaZip"));
    roots.push(PathBuf::from(r"C:\Program Files\7-Zip"));
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join(r"Programs\NanaZip"));
    }
    // Store：只掃名稱含 NanaZip 的套件目錄
    let apps = PathBuf::from(r"C:\Program Files\WindowsApps");
    if apps.is_dir() {
        if let Ok(rd) = fs::read_dir(&apps) {
            for entry in rd.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.contains("NanaZip") {
                    roots.push(entry.path());
                }
            }
        }
    }

    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&root)
            .max_depth(4)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.eq_ignore_ascii_case("NanaZip.Core.Windows.sfx")
                || name.eq_ignore_ascii_case("7z.sfx")
                || name.eq_ignore_ascii_case("7zS.sfx")
            {
                return Ok(path.to_path_buf());
            }
        }
    }
    Err(
        "找不到 NanaZip Windows SFX 模組，無法建立自解 exe。請安裝完整 NanaZip 後再試。"
            .into(),
    )
}

fn zip_dir(src: &Path, zip_path: &Path) -> Result<(), String> {
    let file = File::create(zip_path).map_err(|e| format!("無法建立打包檔：{e}"))?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = match path.strip_prefix(src) {
            Ok(r) if !r.as_os_str().is_empty() => r,
            _ => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if !is_shareable_path(src, path) {
            continue;
        }
        if path.is_file() {
            zip.start_file(rel_str, opts).map_err(|e| e.to_string())?;
            let bytes = std::fs::read(path)
                .map_err(|e| format!("讀取失敗 {}：{e}", path.display()))?;
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
        } else if path.is_dir() {
            let _ = zip.add_directory(format!("{}/", rel_str.trim_end_matches('/')), opts);
        }
    }
    zip.finish().map_err(|e| format!("打包完成失敗：{e}"))?;
    Ok(())
}

fn is_shareable_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let mut components = relative.components();
    let Some(first) = components.next().and_then(|part| part.as_os_str().to_str()) else {
        return false;
    };
    // 嚴格白名單：只裝對方可安裝的翻譯產物（不含 JAR 副本／extra／備份／日誌／說明）
    match first {
        "resourcepacks" | "patchouli_books" | "kubejs" | "minemenu" | "datapacks"
        | "defaultconfigs" | "global_packs" | "paxi" | "data" => true,
        "config" => components.next().is_some(),
        _ => false,
    }
}

fn epoch_millis() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mi18n_pkg_{tag}_{}", std::process::id()))
    }

    #[test]
    fn packages_work_dir_into_named_zip() {
        let root = scratch("ok");
        let _ = fs::remove_dir_all(&root);
        let work = root.join("翻譯結果");
        let rp = work.join("resourcepacks");
        let jars = work.join("jar-translated");
        fs::create_dir_all(&rp).unwrap();
        fs::create_dir_all(&jars).unwrap();
        fs::write(rp.join("pack.zip"), b"dummy-pack").unwrap();
        fs::write(jars.join("example.jar"), b"translated-jar").unwrap();
        fs::create_dir_all(work.join("resourcepacks-extra")).unwrap();
        fs::write(work.join("resourcepacks-extra/translated.zip"), b"overlay-pack").unwrap();
        fs::create_dir_all(work.join("config/starterkit")).unwrap();
        fs::write(work.join("config/starterkit/description.txt"), "翻譯內容").unwrap();
        fs::write(work.join("覆蓋範圍說明.txt"), "coverage").unwrap();

        let dest = root.join("out");
        let zip = package_translation(&work, &dest, "我的分享包").unwrap();
        assert!(zip.is_file());
        assert!(zip.file_name().unwrap().to_string_lossy().ends_with(".zip"));

        let f = fs::File::open(&zip).unwrap();
        let mut ar = zip::ZipArchive::new(f).unwrap();
        let names: Vec<String> = (0..ar.len())
            .map(|i| ar.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(names.iter().any(|n| n.contains("resourcepacks/pack.zip")), "{names:?}");
        assert!(
            !names
                .iter()
                .any(|n| n.contains("resourcepacks-extra") || n.contains("jar-translated")),
            "{names:?}"
        );
        assert!(names.iter().any(|n| n.contains("config/starterkit/description.txt")), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("覆蓋範圍說明.txt")), "{names:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_or_missing_work_dir_errors() {
        let root = scratch("empty");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        assert!(package_translation(&root, &root.join("out"), "x").is_err());
        assert!(package_translation(&root.join("nope"), &root.join("out"), "x").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn bad_name_falls_back_to_default() {
        let root = scratch("name");
        let _ = fs::remove_dir_all(&root);
        let work = root.join("w");
        fs::create_dir_all(work.join("resourcepacks")).unwrap();
        fs::write(work.join("resourcepacks/pack.zip"), b"x").unwrap();
        fs::write(work.join("a.txt"), "x").unwrap();
        let zip = package_translation(&work, &root.join("out"), "///").unwrap();
        assert_eq!(zip.file_name().unwrap().to_string_lossy(), "模組包翻譯分享.zip");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cloud_shortcut_body_points_to_zeitfrei() {
        let root = scratch("url");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join(CLOUD_URL_SHORTCUT_NAME);
        write_cloud_url_shortcut(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("https://cloud.zeitfrei.uk/"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sfx_filename_is_fixed_chinese() {
        assert_eq!(SHARE_SFX_FILENAME, "模組包繁中翻譯自解檔.exe");
        let dest = PathBuf::from("C:/tmp").join(SHARE_SFX_FILENAME);
        assert_eq!(
            dest.file_name().unwrap().to_string_lossy(),
            "模組包繁中翻譯自解檔.exe"
        );
    }

    #[test]
    fn stages_resourcepack_folders_as_zip_without_subdirs() {
        let root = scratch("rpzip");
        let _ = fs::remove_dir_all(&root);
        let work = root.join("翻譯結果");
        let stage = root.join("stage");
        let folder = work.join("resourcepacks").join("字體包");
        fs::create_dir_all(folder.join("assets/minecraft/font")).unwrap();
        fs::write(folder.join("pack.mcmeta"), b"{\"pack\":{}}").unwrap();
        fs::write(
            folder.join("assets/minecraft/font/default.json"),
            b"{}",
        )
        .unwrap();
        fs::write(work.join("resourcepacks").join("already.zip"), b"pkzip").unwrap();
        fs::create_dir_all(work.join("config/ftbquests")).unwrap();
        fs::write(work.join("config/ftbquests/chapter.snbt"), "title: \"Hi\"").unwrap();

        fs::create_dir_all(&stage).unwrap();
        stage_shareable_files(&work, &stage).unwrap();

        let staged_rp = stage.join("resourcepacks");
        assert!(staged_rp.join("already.zip").is_file());
        assert!(staged_rp.join("字體包.zip").is_file());
        assert!(!staged_rp.join("字體包").exists());
        for entry in fs::read_dir(&staged_rp).unwrap() {
            let path = entry.unwrap().path();
            assert!(path.is_file(), "resourcepacks 不可有子資料夾：{}", path.display());
            assert!(
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .ends_with(".zip"),
                "{}",
                path.display()
            );
        }

        let zip_file = fs::File::open(staged_rp.join("字體包.zip")).unwrap();
        let mut archive = zip::ZipArchive::new(zip_file).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(
            names.iter().any(|n| n == "pack.mcmeta" || n.ends_with("/pack.mcmeta")),
            "{names:?}"
        );
        assert!(
            names
                .iter()
                .any(|n| n.contains("assets/minecraft/font/default.json")),
            "{names:?}"
        );
        assert!(stage.join("config/ftbquests/chapter.snbt").is_file());
        let _ = fs::remove_dir_all(&root);
    }
}
