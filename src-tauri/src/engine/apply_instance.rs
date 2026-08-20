//! 一鍵套用到遊戲實例：先備份再複製資源包／任務／文字覆寫（社群期望：可裝、可回滾）。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::jar_scan::resolve_minecraft_dir;
use super::out_layout::{ensure_result_layout, ResultLayout, RESULT_DIR_NAME};
use super::session::{find_session_file, load_session};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub backup_dir: String,
    pub backup_created: bool,
    pub backup_reused: bool,
    pub zip_copied: Option<String>,
    pub jars_copied: usize,
    pub quests_copied: bool,
    pub minemenu_copied: bool,
    pub player_summary: String,
    pub warnings: Vec<String>,
}

/// 套用清單：記錄每個寫入的檔（相對遊戲目錄），以及它是「新增」還是「覆蓋既有」。
/// 有了它，「還原上次套用」才能精準反轉：新增的刪掉、覆蓋的從備份還原。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ApplyManifest {
    stamp: String,
    mc_dir: String,
    backup_dir: String,
    /// 這次新增（原本不存在）→ 還原時刪除
    added: Vec<String>,
    /// 這次覆蓋（原本存在，已備份）→ 還原時從備份複製回來
    overwritten: Vec<String>,
}

const APPLY_MANIFEST: &str = "套用清單.json";

fn rel_to(mc: &Path, target: &Path) -> String {
    target
        .strip_prefix(mc)
        .unwrap_or(target)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 一鍵還原：反轉最近一次套用（新增的刪掉、覆蓋的還原）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub backup_dir: String,
    pub removed: usize,
    pub restored: usize,
    pub player_summary: String,
    pub warnings: Vec<String>,
}

/// 從指定的翻譯結果位置找最近一次套用備份；沒有指定時相容舊版實例旁備份。
pub fn restore_last_apply_in(
    instance_path: &Path,
    result_root: Option<&Path>,
) -> Result<RestoreResult, String> {
    let mc = resolve_minecraft_dir(instance_path)?;
    let mut backups = find_backup_dirs(&mc, result_root);
    if backups.is_empty() {
        return Err("找不到任何『翻譯套用備份_』資料夾，沒有可還原的套用紀錄。".into());
    }
    backups.sort(); // 時間戳在名字裡，字典序≈時間序
    let backup_root = backups.last().unwrap().clone();

    let manifest_path = backup_root.join(APPLY_MANIFEST);
    let mut removed = 0usize;
    let mut restored = 0usize;
    let mut warnings = Vec::new();
    let mut critical_failures = Vec::new();

    if let Ok(text) = fs::read_to_string(&manifest_path) {
        let manifest: ApplyManifest = serde_json::from_str(&text)
            .map_err(|e| format!("套用清單讀取失敗：{e}"))?;
        if !manifest.mc_dir.is_empty() {
            let manifest_key = path_key(Path::new(&manifest.mc_dir));
            let current_key = path_key(&mc);
            if manifest_key != current_key {
                return Err(format!(
                    "備份對應的遊戲目錄與目前選擇不符，已中止還原以免改到錯誤實例。\n\
備份紀錄：{}\n\
目前選擇：{}",
                    manifest.mc_dir,
                    mc.display()
                ));
            }
        }
        // 新增的 → 刪除
        for rel in &manifest.added {
            let p = mc.join(rel);
            if !p.is_file() {
                continue;
            }
            match fs::remove_file(&p) {
                Ok(()) => removed += 1,
                Err(error) => {
                    let message = format!("無法移除新增檔「{rel}」：{error}");
                    warnings.push(message.clone());
                    critical_failures.push(message);
                }
            }
        }
        // 覆蓋的 → 從備份複製回來（備份鏡像 mc 相對結構）
        for rel in &manifest.overwritten {
            let from = backup_root.join(rel);
            let to = mc.join(rel);
            if !from.is_file() {
                let message = format!("備份缺少應還原的檔案「{rel}」，已略過。");
                warnings.push(message);
                continue;
            }
            if let Some(parent) = to.parent() {
                if let Err(error) = fs::create_dir_all(parent) {
                    let message = format!("無法建立還原目錄「{}」：{error}", parent.display());
                    warnings.push(message.clone());
                    critical_failures.push(message);
                    continue;
                }
            }
            match fs::copy(&from, &to) {
                Ok(_) => restored += 1,
                Err(error) => {
                    let message = format!("還原覆蓋檔「{rel}」失敗：{error}");
                    warnings.push(message.clone());
                    critical_failures.push(message);
                }
            }
        }
    } else {
        // 舊備份沒有清單：退回「把備份內容整包蓋回去」（只能還原覆蓋，無法刪掉新增的）
        for sub in [
            "mods",
            "resourcepacks",
            "config",
            "minemenu",
            "patchouli_books",
            "kubejs",
            "datapacks",
            "defaultconfigs",
            "global_packs",
            "paxi",
            "data",
        ] {
            let from = backup_root.join(sub);
            if from.is_dir() {
                let (count, failures) = restore_tree(&from, &mc.join(sub));
                restored += count;
                for failure in failures {
                    warnings.push(failure.clone());
                    critical_failures.push(failure);
                }
            }
        }
    }

    if !critical_failures.is_empty() {
        return Err(format!(
            "還原未完全成功（{} 項失敗），請關閉遊戲後重試或手動從備份還原。\n\
備份來源：{}\n\
已移除新增檔：{} 個；已還原覆蓋檔：{} 個\n\
失敗項目：\n{}",
            critical_failures.len(),
            backup_root.display(),
            removed,
            restored,
            critical_failures.join("\n")
        ));
    }

    let warning_block = if warnings.is_empty() {
        String::new()
    } else {
        format!("\n\n注意：\n• {}", warnings.join("\n• "))
    };
    let player_summary = format!(
        "已還原上次套用。\n\
• 備份來源：\n{}\n\
• 移除本次新增檔：{} 個\n\
• 還原被覆蓋檔：{} 個\n\n\
現在再開一次遊戲：\n\
• 若開得起來 → 先前是翻譯檔造成的，歡迎把當機報告給我們修\n\
• 若還是開不起來 → 不是翻譯，多半是整合包缺模組（可用『診斷開不了』看是缺什麼）\n\
（原始 mods/*.jar 不會直接修改；翻譯副本會先備份後套用）{}",
        backup_root.display(),
        removed,
        restored,
        warning_block
    );
    Ok(RestoreResult {
        backup_dir: backup_root.display().to_string(),
        removed,
        restored,
        player_summary,
        warnings,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteBackupResult {
    pub deleted: usize,
    pub failed: Vec<String>,
    pub player_summary: String,
}

pub fn delete_apply_backups_in(
    instance_path: &Path,
    result_root: Option<&Path>,
) -> Result<DeleteBackupResult, String> {
    let mc = resolve_minecraft_dir(instance_path)?;
    let backups = find_backup_dirs(&mc, result_root);

    let mut deleted = 0usize;
    let mut failed = Vec::new();
    for backup in backups {
        match fs::remove_dir_all(&backup) {
            Ok(()) => deleted += 1,
            Err(error) => failed.push(format!("{}：{}", backup.display(), error)),
        }
    }

    let player_summary = if failed.is_empty() {
        if deleted == 0 {
            "沒有找到工具建立的備份檔案。".to_string()
        } else {
            format!("已刪除 {} 個翻譯套用備份。", deleted)
        }
    } else {
        format!("已刪除 {} 個備份，但有 {} 個無法刪除。", deleted, failed.len())
    };

    Ok(DeleteBackupResult {
        deleted,
        failed,
        player_summary,
    })
}

/// 回報指定實例／結果位置是否存在本工具建立的套用備份。
/// 這只讀取目錄名稱與套用清單，不會讀寫遊戲內容，供 UI 決定是否顯示還原／刪除按鈕。
pub fn has_apply_backups_in(
    instance_path: &Path,
    result_root: Option<&Path>,
) -> Result<bool, String> {
    let mc = resolve_minecraft_dir(instance_path)?;
    Ok(!find_backup_dirs(&mc, result_root).is_empty())
}

fn find_backup_dirs(mc: &Path, result_root: Option<&Path>) -> Vec<PathBuf> {
    let mut containers = Vec::new();
    let mut add_container = |path: PathBuf| {
        if !containers.iter().any(|existing| existing == &path) {
            containers.push(path);
        }
    };

    if let Some(root) = result_root {
        let work_root = if root.file_name().and_then(|name| name.to_str()) == Some(RESULT_DIR_NAME) {
            root.to_path_buf()
        } else {
            root.join(RESULT_DIR_NAME)
        };
        add_container(work_root);
        add_container(root.to_path_buf());
    }
    if let Some(parent) = mc.parent() {
        // 舊版備份在 Minecraft 資料夾旁；保留讀取與刪除相容性。
        add_container(parent.to_path_buf());
    }

    let mut backups = Vec::new();
    for container in containers {
        if let Ok(entries) = fs::read_dir(container) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.starts_with("翻譯套用備份_"))
                        .unwrap_or(false)
                    && !backups.iter().any(|existing| existing == &path)
                {
                    backups.push(path);
                }
            }
        }
    }
    backups.sort();
    backups
}

fn collect_planned_tree(source: &Path, target_root: &Path, targets: &mut Vec<PathBuf>) {
    for entry in walkdir::WalkDir::new(source).into_iter().filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let relative = path.strip_prefix(source).unwrap_or(path);
        targets.push(target_root.join(relative));
    }
}

fn path_key(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn manifest_contains(list: &[String], relative: &str) -> bool {
    list.iter()
        .any(|item| item.replace('\\', "/") == relative)
}

/// 找到同一實例、同一結果資料夾下仍能完整還原的舊備份。
/// 只要這次會覆蓋一個舊備份沒有涵蓋的檔案，就不重用，改建新的備份保護玩家資料。
fn find_reusable_backup(
    work_root: &Path,
    mc: &Path,
    planned_targets: &[PathBuf],
) -> Option<(PathBuf, ApplyManifest)> {
    // 同時檢查目前結果資料夾與舊版曾放在 Minecraft 同層的備份，
    // 避免升級工具後把同一份原始檔再備份一次。
    let mut candidates = find_backup_dirs(mc, Some(work_root));
    candidates.sort();
    let mc_key = path_key(mc);

    for backup_root in candidates.into_iter().rev() {
        let manifest_path = backup_root.join(APPLY_MANIFEST);
        let Ok(text) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<ApplyManifest>(&text) else {
            continue;
        };
        if manifest.mc_dir.is_empty() || path_key(Path::new(&manifest.mc_dir)) != mc_key {
            continue;
        }

        let complete = planned_targets.iter().all(|target| {
            if !target.is_file() {
                return true;
            }
            let relative = rel_to(mc, target);
            if manifest_contains(&manifest.added, &relative) {
                return true;
            }
            manifest_contains(&manifest.overwritten, &relative)
                && backup_root.join(&relative).is_file()
        });
        if complete {
            return Some((backup_root, manifest));
        }
    }
    None
}

fn merge_manifests(previous: ApplyManifest, current: ApplyManifest) -> ApplyManifest {
    let mut merged = previous;
    for relative in current.added {
        if !manifest_contains(&merged.added, &relative) {
            merged.added.push(relative.clone());
        }
        merged
            .overwritten
            .retain(|item| item.replace('\\', "/") != relative);
    }
    for relative in current.overwritten {
        if !manifest_contains(&merged.added, &relative)
            && !manifest_contains(&merged.overwritten, &relative)
        {
            merged.overwritten.push(relative);
        }
    }
    merged.added.sort();
    merged.added.dedup();
    merged.overwritten.sort();
    merged.overwritten.dedup();
    merged
}

fn restore_tree(from: &Path, to: &Path) -> (usize, Vec<String>) {
    let mut n = 0usize;
    let mut failures = Vec::new();
    for entry in walkdir::WalkDir::new(from).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let rel = path.strip_prefix(from).unwrap_or(path);
        let target = to.join(rel);
        if let Some(parent) = target.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                failures.push(format!(
                    "無法建立還原目錄「{}」：{error}",
                    parent.display()
                ));
                continue;
            }
        }
        match fs::copy(path, &target) {
            Ok(_) => n += 1,
            Err(error) => failures.push(format!(
                "還原檔案失敗「{}」→「{}」：{error}",
                path.display(),
                target.display()
            )),
        }
    }
    (n, failures)
}

/// 將「翻譯結果」套用到遊戲：resourcepacks zip + config 文字覆寫 + minemenu
/// + patchouli_books / kubejs / 資料包根目錄（若 work 有）。
/// 不修改原始 mods/*.jar；翻譯副本套用前會備份被覆蓋的目標。
pub fn apply_to_instance(
    instance_path: &Path,
    output_or_work: &Path,
    pack_name_hint: Option<&str>,
    create_backup: bool,
) -> Result<ApplyResult, String> {
    if !instance_path.exists() {
        return Err("找不到遊戲資料夾，請重新選擇。".into());
    }
    let mc = resolve_minecraft_dir(instance_path)?;
    let layout = ensure_result_layout(output_or_work)?;
    let work = &layout.work_root;

    let mut warnings = Vec::new();
    warnings.push(
        "請先完全關閉 Minecraft／啟動器載入中的實例，再套用。若遊戲仍在跑，可能複製失敗或檔案被鎖。"
            .into(),
    );

    let pack_name = resolve_pack_name(work, pack_name_hint);
    let zip_src = find_zip_in_layout(&layout, &pack_name);
    let resourcepacks_extra_src = work.join("resourcepacks-extra");
    let quests_src = work.join("config").join("ftbquests");
    let menu_src = work.join("minemenu").join("menu.json");
    let patchouli_src = work.join("patchouli_books");
    let config_src = work.join("config");
    let openloader_src = work.join("config").join("openloader");
    let kubejs_src = work.join("kubejs");
    let fancymenu_src = work.join("config").join("fancymenu");
    let datapacks_src = work.join("datapacks");
    let defaultconfigs_src = work.join("defaultconfigs");
    let global_packs_src = work.join("global_packs");
    let paxi_src = work.join("paxi");
    let jar_src = work.join("jar-translated");

    let has_patchouli = dir_has_files(&patchouli_src);
    let has_openloader = dir_has_files(&openloader_src);
    let has_kubejs = dir_has_files(&kubejs_src);
    let has_fancymenu = dir_has_files(&fancymenu_src);
    let has_config = dir_has_files(&config_src);
    let has_datapacks = dir_has_files(&datapacks_src);
    let has_defaultconfigs = dir_has_files(&defaultconfigs_src);
    let has_global_packs = dir_has_files(&global_packs_src);
    let has_paxi = dir_has_files(&paxi_src);
    let has_jars = dir_has_files(&jar_src);
    let has_resourcepacks_extra = dir_has_files(&resourcepacks_extra_src);

    if zip_src.is_none()
        && !quests_src.is_dir()
        && !menu_src.is_file()
        && !has_patchouli
        && !has_openloader
        && !has_kubejs
        && !has_fancymenu
        && !has_config
        && !has_datapacks
        && !has_defaultconfigs
        && !has_global_packs
        && !has_paxi
        && !has_jars
        && !has_resourcepacks_extra
    {
        return Err(format!(
            "在「{}」找不到可套用的 zip／任務／快捷選單／文字覆寫。請先完成一鍵翻譯。",
            work.display()
        ));
    }

    let menu_dest = mc.join("minemenu").join("menu.json");
    let patchouli_dest = mc.join("patchouli_books");
    let kubejs_dest = mc.join("kubejs");
    let datapacks_dest = mc.join("datapacks");

    let mut planned_targets = Vec::new();
    if let Some(zip) = zip_src.as_deref() {
        if let Some(name) = zip.file_name() {
            planned_targets.push(mc.join("resourcepacks").join(name));
        }
        planned_targets.push(mc.join("options.txt"));
    }
    if has_resourcepacks_extra {
        collect_planned_tree(
            &resourcepacks_extra_src,
            &mc.join("resourcepacks"),
            &mut planned_targets,
        );
    }
    if has_config {
        collect_planned_tree(&config_src, &mc.join("config"), &mut planned_targets);
    }
    if menu_src.is_file() {
        planned_targets.push(menu_dest.clone());
    }
    for (source, target, enabled) in [
        (&patchouli_src, &patchouli_dest, has_patchouli),
        (&kubejs_src, &kubejs_dest, has_kubejs),
        (&datapacks_src, &datapacks_dest, has_datapacks),
    ] {
        if enabled {
            collect_planned_tree(source, target, &mut planned_targets);
        }
    }
    for (source, name, enabled) in [
        (&defaultconfigs_src, "defaultconfigs", has_defaultconfigs),
        (&global_packs_src, "global_packs", has_global_packs),
        (&paxi_src, "paxi", has_paxi),
    ] {
        if enabled {
            collect_planned_tree(source, &mc.join(name), &mut planned_targets);
        }
    }
    if has_jars {
        collect_planned_tree(&jar_src, &mc.join("mods"), &mut planned_targets);
    }

    let mut stamp = backup_stamp();
    // 備份跟著翻譯結果走，刪除結果資料夾時可以一次清理；相同實例與目標已經有完整備份時直接沿用。
    let mut backup_root = layout
        .work_root
        .join(format!("翻譯套用備份_{stamp}"));
    let mut backup_reused = false;
    let mut previous_manifest = None;
    if create_backup {
        if let Some((existing_root, manifest)) =
            find_reusable_backup(&layout.work_root, &mc, &planned_targets)
        {
            backup_root = existing_root;
            backup_reused = true;
            stamp = manifest.stamp.clone();
            previous_manifest = Some(manifest);
        } else {
            fs::create_dir_all(&backup_root).map_err(|e| format!("無法建立備份目錄：{e}"))?;
        }
    }

    // ── 備份現有資源包 ──
    if create_backup && !backup_reused {
        if let Some(ref zip) = zip_src {
        let name = zip
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("pack.zip");
        let dest_rp = mc.join("resourcepacks").join(name);
        if dest_rp.is_file() {
            let bak = backup_root.join("resourcepacks");
            fs::create_dir_all(&bak).ok();
            let _ = fs::copy(&dest_rp, bak.join(name));
        }
        // 若有同名資料夾資源包也備份
        let folder = mc
            .join("resourcepacks")
            .join(name.trim_end_matches(".zip"));
        if folder.is_dir() {
            let bak = backup_root.join("resourcepacks").join(
                folder
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("pack_dir"),
            );
            copy_dir_recursive(&folder, &bak)?;
        }
        }

        if has_resourcepacks_extra && mc.join("resourcepacks").is_dir() {
            backup_matching_tree(
                &resourcepacks_extra_src,
                &mc.join("resourcepacks"),
                &backup_root.join("resourcepacks"),
            )?;
        }

        // ── 備份 minemenu ──
        if menu_src.is_file() && menu_dest.is_file() {
            let bak = backup_root.join("minemenu");
            fs::create_dir_all(&bak).ok();
            let _ = fs::copy(&menu_dest, bak.join("menu.json"));
        }

        // ── 備份 patchouli_books ──
        if has_patchouli && patchouli_dest.is_dir() {
            let bak = backup_root.join("patchouli_books");
            copy_dir_recursive(&patchouli_dest, &bak)?;
        }

        // ── 備份所有即將覆寫的 config 文字（含任務、openloader 與顯示型設定）──
        if has_config && mc.join("config").is_dir() {
            backup_matching_tree(&config_src, &mc.join("config"), &backup_root.join("config"))?;
        }

        // ── 備份 kubejs（僅 work 會覆寫的相對路徑）──
        if has_kubejs && kubejs_dest.is_dir() {
            backup_matching_tree(&kubejs_src, &kubejs_dest, &backup_root.join("kubejs"))?;
        }

        if has_datapacks && datapacks_dest.is_dir() {
            let bak = backup_root.join("datapacks");
            copy_dir_recursive(&datapacks_dest, &bak)?;
        }

        for (source, name) in [
            (&defaultconfigs_src, "defaultconfigs"),
            (&global_packs_src, "global_packs"),
            (&paxi_src, "paxi"),
        ] {
            if dir_has_files(source) && mc.join(name).is_dir() {
                copy_dir_recursive(&mc.join(name), &backup_root.join(name))?;
            }
        }

        // ── 備份即將被翻譯 JAR 覆蓋的 mods 檔案 ──
        if has_jars {
            let mods_dest = mc.join("mods");
            if mods_dest.is_dir() {
                backup_matching_tree(&jar_src, &mods_dest, &backup_root.join("mods"))?;
            }
        }
    }

    // 寫備份說明
    if create_backup && !backup_reused {
        let bak_note = format!(
        "【翻譯套用備份】\n\
時間戳：{stamp}\n\
遊戲目錄：{}\n\
翻譯結果：{}\n\
\n\
還原方式：\n\
1. 關閉遊戲\n\
2. 把本備份內 mods / resourcepacks / config / minemenu / patchouli_books / kubejs / datapacks 對應複製回遊戲\n\
3. 勿刪未備份的其他自訂檔\n\
4. 原始 mods/*.jar 不會直接修改；翻譯副本會在備份後套用\n",
        mc.display(),
        work.display()
    );
        let _ = fs::write(backup_root.join("還原說明.txt"), bak_note);
    }

    // 套用清單（供一鍵還原）
    let mut manifest = ApplyManifest {
        stamp: stamp.clone(),
        mc_dir: mc.display().to_string(),
        backup_dir: if create_backup {
            backup_root.display().to_string()
        } else {
            String::new()
        },
        ..Default::default()
    };

    // ── 複製 zip ──
    let mut zip_copied = None;
    let mut jars_copied = 0usize;
    if let Some(zip) = zip_src {
        let rp = mc.join("resourcepacks");
        fs::create_dir_all(&rp).map_err(|e| e.to_string())?;
        let name = zip
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("繁體中文翻譯.zip");
        let dest = rp.join(name);
        let existed = dest.is_file();
        fs::copy(&zip, &dest).map_err(|e| {
            format!(
                "複製資源包失敗（請確認遊戲已關閉且路徑可寫）：{e}\n來源：{}\n目標：{}",
                zip.display(),
                dest.display()
            )
        })?;
        record_written(&mut manifest, &mc, &dest, existed);
        zip_copied = Some(dest.display().to_string());
    }

    // ── 複製 ZIP 內翻譯覆寫（不與主翻譯包混在一起）──
    let mut resourcepack_overlays_copied = false;
    if has_resourcepacks_extra {
        merge_copy_dir(
            &resourcepacks_extra_src,
            &mc.join("resourcepacks"),
            &mc,
            &mut manifest,
        )?;
        resourcepack_overlays_copied = true;
    }

    // ── 複製所有 config 翻譯覆寫（任務、openloader、FancyMenu 與其他顯示型設定）──
    let quests_copied = quests_src.is_dir();
    if has_config {
        fs::create_dir_all(mc.join("config")).map_err(|e| e.to_string())?;
        merge_copy_dir(&config_src, &mc.join("config"), &mc, &mut manifest)?;
    }

    // ── 複製 minemenu ──
    let mut minemenu_copied = false;
    if menu_src.is_file() {
        let menu_dir = mc.join("minemenu");
        fs::create_dir_all(&menu_dir).map_err(|e| e.to_string())?;
        let existed = menu_dest.is_file();
        fs::copy(&menu_src, &menu_dest).map_err(|e| format!("複製快捷選單失敗：{e}"))?;
        record_written(&mut manifest, &mc, &menu_dest, existed);
        minemenu_copied = true;
    }

    // ── 複製 patchouli_books ──
    let mut patchouli_copied = false;
    if has_patchouli {
        merge_copy_dir(&patchouli_src, &patchouli_dest, &mc, &mut manifest)?;
        patchouli_copied = true;
    }

    let openloader_copied = has_openloader;

    // ── 複製 kubejs（work 僅含翻譯產出；merge，不碰 mods）──
    let mut kubejs_copied = false;
    if has_kubejs {
        merge_copy_dir(&kubejs_src, &kubejs_dest, &mc, &mut manifest)?;
        kubejs_copied = true;
    }

    let fancymenu_copied = has_fancymenu;

    let config_overlays_copied = has_config;

    let mut datapacks_copied = false;
    if has_datapacks {
        merge_copy_dir(&datapacks_src, &datapacks_dest, &mc, &mut manifest)?;
        datapacks_copied = true;
    }

    for (source, name) in [
        (&defaultconfigs_src, "defaultconfigs"),
        (&global_packs_src, "global_packs"),
        (&paxi_src, "paxi"),
    ] {
        if dir_has_files(source) {
            merge_copy_dir(source, &mc.join(name), &mc, &mut manifest)?;
        }
    }

    if has_jars {
        merge_copy_dir(&jar_src, &mc.join("mods"), &mc, &mut manifest)?;
        jars_copied = count_files(&jar_src);
    }

    if let Some(ref copied) = zip_copied {
        if let Some(name) = Path::new(copied).file_name().and_then(|s| s.to_str()) {
            enable_resource_pack(
                &mc,
                name,
                (create_backup && !backup_reused).then_some(&backup_root),
                &mut manifest,
            )?;
            for warn in warn_enabled_packs_covering_font(&mc, name) {
                warnings.push(warn);
            }
        }
    }

    // 寫套用清單（供「一鍵還原」精準反轉）
    if create_backup {
        let manifest = if let Some(previous) = previous_manifest {
            merge_manifests(previous, manifest)
        } else {
            manifest
        };
        let js = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("套用清單序列化失敗：{e}"))?;
        fs::write(backup_root.join(APPLY_MANIFEST), js + "\n")
            .map_err(|e| format!("寫入套用清單失敗：{e}"))?;
    }

    let overlay_line = {
        let mut parts = Vec::new();
        if patchouli_copied {
            parts.push("patchouli_books");
        }
        if openloader_copied {
            parts.push("config/openloader");
        }
        if kubejs_copied {
            parts.push("kubejs");
        }
        if fancymenu_copied {
            parts.push("config/fancymenu");
        }
        if config_overlays_copied {
            parts.push("config 文字覆寫");
        }
        if datapacks_copied {
            parts.push("datapacks");
        }
        if resourcepack_overlays_copied {
            parts.push("resourcepacks 內 ZIP 覆寫");
        }
        if parts.is_empty() {
            "無／未複製".into()
        } else {
            format!("已合併 {}", parts.join("、"))
        }
    };

    let player_summary = format!(
        "已套用到遊戲（依備份選項複製；目標＝整合包可遊玩文字→台灣繁中（除圖片））\n\
• 備份目錄：\n{}\n\
• 資源包：{}\n\
• 翻譯 JAR：{} 個（是否備份原檔依選項，再覆蓋到 mods）\n\
• 任務 ftbquests：{}\n\
• 快捷選單：{}\n\
• 文字覆寫：{}\n\n\
【請你】\n\
1. 開遊戲 → 語言選「繁體中文（台灣）」\n\
2. 資源包啟用剛複製的 zip\n\
3. 本工具不保證 100% 中文，任務／寫死字串／圖片文字可能仍英文\n\
\n\
【萬一遊戲／世界開不起來】\n\
• 多半是整合包本身缺模組（結構／前置），跟翻譯無關——按「診斷開不了」會讀當機報告告訴你缺什麼。\n\
• 想排除是不是翻譯造成的：按「還原上次套用」一鍵復原（新增的刪掉、覆蓋的還原），再開一次。\n\
• 資源包（語言檔）很安全；會影響世界載入的是資料包／任務類，還原後即可排除。",
        if create_backup && backup_reused {
            format!("沿用既有備份：{}", backup_root.display())
        } else if create_backup {
            format!("新建備份：{}", backup_root.display())
        } else {
            "未建立備份（依你的選擇）".to_string()
        },
        zip_copied
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or("（本次無 zip）"),
        jars_copied,
        if quests_copied {
            "已覆蓋 config/ftbquests"
        } else {
            "無／未複製"
        },
        if minemenu_copied {
            "已複製"
        } else {
            "無／未複製"
        },
        overlay_line,
    );

    Ok(ApplyResult {
        backup_dir: if create_backup {
            backup_root.display().to_string()
        } else {
            String::new()
        },
        backup_created: create_backup && !backup_reused,
        backup_reused,
        zip_copied,
        jars_copied,
        quests_copied,
        minemenu_copied,
        player_summary,
        warnings,
    })
}

fn resolve_pack_name(work: &Path, hint: Option<&str>) -> String {
    if let Some(h) = hint {
        let t = h.trim();
        if !t.is_empty() {
            return t.trim_end_matches(".zip").to_string();
        }
    }
    if let Ok((sess, _)) = load_session(work) {
        if !sess.pack_name.trim().is_empty() {
            return sess.pack_name.trim().to_string();
        }
    }
    if let Some(sf) = find_session_file(work) {
        if let Ok((sess, _)) = load_session(sf.parent().unwrap_or(work)) {
            if !sess.pack_name.trim().is_empty() {
                return sess.pack_name.trim().to_string();
            }
        }
    }
    "繁體中文翻譯".into()
}

fn find_zip_in_layout(layout: &ResultLayout, pack_name: &str) -> Option<PathBuf> {
    let name = pack_name.trim_end_matches(".zip");
    let candidates = [
        layout.resourcepacks.join(format!("{name}.zip")),
        layout.work_root.join("resourcepacks").join(format!("{name}.zip")),
        layout.work_root.join(format!("{name}.zip")),
    ];
    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    // 掃 resourcepacks 下第一個 .zip
    if let Ok(rd) = fs::read_dir(&layout.resourcepacks) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("zip") {
                return Some(p);
            }
        }
    }
    None
}

fn backup_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

fn dir_has_files(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|e| e.path().is_file())
}

fn count_files(dir: &Path) -> usize {
    if !dir.is_dir() {
        return 0;
    }
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .count()
}

/// 只備份 work 裡會覆寫到 dest 的相對路徑（避免整包 kubejs 過大）
fn backup_matching_tree(src: &Path, dest: &Path, bak_root: &Path) -> Result<(), String> {
    if !src.is_dir() || !dest.is_dir() {
        return Ok(());
    }
    for entry in walkdir::WalkDir::new(src)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let rel = path.strip_prefix(src).unwrap_or(path);
        let existing = dest.join(rel);
        if existing.is_file() {
            let bak = bak_root.join(rel);
            if let Some(parent) = bak.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let _ = fs::copy(&existing, &bak);
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in walkdir::WalkDir::new(src)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let rel = path.strip_prefix(src).unwrap_or(path);
        let target = dst.join(rel);
        if path.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else if path.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(path, &target).map_err(|e| format!("備份複製失敗 {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// 把 src 樹合併進 dst（覆蓋同名檔）；記錄每個寫入檔是新增或覆蓋（供還原）。
fn merge_copy_dir(
    src: &Path,
    dst: &Path,
    mc: &Path,
    manifest: &mut ApplyManifest,
) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in walkdir::WalkDir::new(src)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let rel = path.strip_prefix(src).unwrap_or(path);
        let target = dst.join(rel);
        if path.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else if path.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let existed = target.is_file();
            fs::copy(path, &target).map_err(|e| {
                format!(
                    "套用複製失敗（請關遊戲後重試）{} → {}：{e}",
                    path.display(),
                    target.display()
                )
            })?;
            record_written(manifest, mc, &target, existed);
        }
    }
    Ok(())
}

/// 記錄一個寫入的檔到套用清單。
fn record_written(manifest: &mut ApplyManifest, mc: &Path, target: &Path, existed_before: bool) {
    let rel = rel_to(mc, target);
    if existed_before {
        manifest.overwritten.push(rel);
    } else {
        manifest.added.push(rel);
    }
}

fn enable_resource_pack(
    mc: &Path,
    zip_name: &str,
    backup_root: Option<&Path>,
    manifest: &mut ApplyManifest,
) -> Result<(), String> {
    let options = mc.join("options.txt");
    let existed = options.is_file();
    let original = if existed {
        fs::read_to_string(&options).map_err(|e| format!("讀取 options.txt 失敗：{e}"))?
    } else {
        String::new()
    };
    if let Some(backup_root) = backup_root {
        if existed {
            fs::copy(&options, backup_root.join("options.txt"))
                .map_err(|e| format!("備份 options.txt 失敗：{e}"))?;
        }
    }

    let entry = format!("file/{zip_name}");
    let mut found = false;
    let mut lines = Vec::new();
    for line in original.lines() {
        if let Some(value) = line.strip_prefix("resourcePacks:") {
            found = true;
            let mut list = value.trim().to_string();
            if !list.contains(&format!("\"{entry}\"")) {
                if list == "[]" {
                    list = format!("[\"{entry}\"]");
                } else if list.ends_with(']') {
                    list.pop();
                    if !list.ends_with('[') {
                        list.push(',');
                    }
                    list.push_str(&format!("\"{entry}\"]"));
                }
            }
            lines.push(format!("resourcePacks:{list}"));
        } else {
            lines.push(line.to_string());
        }
    }
    if !found {
        lines.push(format!("resourcePacks:[\"{entry}\"]"));
    }
    let mut updated = lines.join("\n");
    updated.push('\n');
    fs::write(&options, updated).map_err(|e| format!("寫入 options.txt 失敗：{e}"))?;
    record_written(manifest, mc, &options, existed);
    Ok(())
}

/// 已啟用且含 `assets/*/font/` 的資源包可能蓋掉翻譯／自訂字體 → 警告（不做 codec 重寫）。
fn warn_enabled_packs_covering_font(mc: &Path, our_zip_name: &str) -> Vec<String> {
    let options = mc.join("options.txt");
    let Ok(text) = fs::read_to_string(&options) else {
        return Vec::new();
    };
    let Some(list_line) = text.lines().find(|l| l.starts_with("resourcePacks:")) else {
        return Vec::new();
    };
    let value = list_line.strip_prefix("resourcePacks:").unwrap_or("").trim();
    let our_entry = format!("file/{our_zip_name}");
    let mut suspects = Vec::new();
    for raw in value.split('"') {
        let entry = raw.trim();
        if entry.is_empty()
            || entry == "vanilla"
            || entry == ","
            || entry == "["
            || entry == "]"
            || entry == our_entry
        {
            continue;
        }
        let Some(name) = entry.strip_prefix("file/") else {
            continue;
        };
        if pack_contains_font_override(mc, name) {
            suspects.push(name.to_string());
        }
    }
    if suspects.is_empty() {
        return Vec::new();
    }
    vec![format!(
        "以下已啟用資源包含 font/，可能蓋過翻譯或自訂字體顯示：{}。請在資源包選單把「繁中翻譯／字體包」置頂，或暫時停用上述包後重開遊戲。",
        suspects.join("、")
    )]
}

fn pack_contains_font_override(mc: &Path, pack_name: &str) -> bool {
    let rp = mc.join("resourcepacks").join(pack_name);
    if rp.is_dir() {
        return dir_has_font_assets(&rp);
    }
    if rp.is_file() {
        return zip_has_font_assets(&rp);
    }
    // 名稱可能沒副檔名
    let zip = mc.join("resourcepacks").join(format!("{pack_name}.zip"));
    if zip.is_file() {
        return zip_has_font_assets(&zip);
    }
    false
}

fn dir_has_font_assets(root: &Path) -> bool {
    let walker = walkdir::WalkDir::new(root).max_depth(8);
    for entry in walker.into_iter().flatten() {
        let path = entry.path();
        let lower = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        if lower.contains("/font/") && path.is_file() {
            return true;
        }
    }
    false
}

fn zip_has_font_assets(zip_path: &Path) -> bool {
    let Ok(file) = fs::File::open(zip_path) else {
        return false;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return false;
    };
    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index(i) else {
            continue;
        };
        let name = entry.name().replace('\\', "/").to_ascii_lowercase();
        if name.contains("/font/") && !entry.is_dir() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod apply_font_warn_tests {
    use super::*;

    #[test]
    fn detects_font_dir_in_loose_pack() {
        let root = std::env::temp_dir().join(format!("font_warn_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let pack = mc.join("resourcepacks").join("GuiFontPack");
        fs::create_dir_all(pack.join("assets/minecraft/font")).unwrap();
        fs::write(pack.join("assets/minecraft/font/default.json"), "{}").unwrap();
        fs::write(
            mc.join("options.txt"),
            "resourcePacks:[\"file/GuiFontPack\",\"file/繁體中文翻譯.zip\"]\n",
        )
        .unwrap();
        let warns = warn_enabled_packs_covering_font(&mc, "繁體中文翻譯.zip");
        assert_eq!(warns.len(), 1);
        assert!(warns[0].contains("GuiFontPack"));
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enables_pack_and_keeps_existing_resource_packs() {
        let root = std::env::temp_dir().join(format!("apply_options_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let backup = root.join("backup");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(&backup).unwrap();
        fs::write(mc.join("options.txt"), "guiScale:3\nresourcePacks:[\"vanilla\"]\n").unwrap();
        let mut manifest = ApplyManifest::default();
        enable_resource_pack(&mc, "pack.zip", Some(&backup), &mut manifest).unwrap();
        let options = fs::read_to_string(mc.join("options.txt")).unwrap();
        assert!(options.contains("\"vanilla\""));
        assert!(options.contains("\"file/pack.zip\""));
        assert!(backup.join("options.txt").is_file());
        assert!(manifest.overwritten.contains(&"options.txt".to_string()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn applies_translated_jars_after_backing_up_originals() {
        let root = std::env::temp_dir().join(format!("apply_jars_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let work = root.join("翻譯結果");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(work.join("jar-translated")).unwrap();
        fs::write(mc.join("mods/example.jar"), b"original").unwrap();
        fs::write(work.join("jar-translated/example.jar"), b"translated").unwrap();

        let result = apply_to_instance(&mc, &work, None, true).unwrap();
        assert_eq!(result.jars_copied, 1);
        assert!(result.backup_created);
        assert!(!result.backup_reused);
        assert_eq!(fs::read(mc.join("mods/example.jar")).unwrap(), b"translated");
        assert!(PathBuf::from(&result.backup_dir).starts_with(&work));
        assert_eq!(
            fs::read(PathBuf::from(&result.backup_dir).join("mods/example.jar")).unwrap(),
            b"original"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reuses_matching_backup_on_repeated_apply() {
        let root = std::env::temp_dir().join(format!("apply_reuse_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let work = root.join("翻譯結果");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(work.join("jar-translated")).unwrap();
        fs::write(mc.join("mods/example.jar"), b"original").unwrap();
        fs::write(work.join("jar-translated/example.jar"), b"translated-v1").unwrap();

        let first = apply_to_instance(&mc, &work, None, true).unwrap();
        let first_backup = first.backup_dir.clone();
        fs::write(work.join("jar-translated/example.jar"), b"translated-v2").unwrap();
        let second = apply_to_instance(&mc, &work, None, true).unwrap();

        assert!(!second.backup_created);
        assert!(second.backup_reused);
        assert_eq!(second.backup_dir, first_backup);
        assert_eq!(fs::read(mc.join("mods/example.jar")).unwrap(), b"translated-v2");
        let backup_count = fs::read_dir(&work)
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("翻譯套用備份_")
            })
            .count();
        assert_eq!(backup_count, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn skips_backup_when_player_disables_it() {
        let root = std::env::temp_dir().join(format!("apply_no_backup_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let work = root.join("翻譯結果");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(work.join("jar-translated")).unwrap();
        fs::write(mc.join("mods/example.jar"), b"original").unwrap();
        fs::write(work.join("jar-translated/example.jar"), b"translated").unwrap();

        let result = apply_to_instance(&mc, &work, None, false).unwrap();
        assert!(!result.backup_created);
        assert!(result.backup_dir.is_empty());
        assert_eq!(fs::read(mc.join("mods/example.jar")).unwrap(), b"translated");
        let backup_count = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("翻譯套用備份_")
            })
            .count();
        assert_eq!(backup_count, 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn deletes_only_tool_backup_directories() {
        let root = std::env::temp_dir().join(format!("delete_backups_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(root.join("翻譯套用備份_20260811_1")).unwrap();
        fs::create_dir_all(root.join("翻譯套用備份_20260811_2")).unwrap();
        fs::create_dir_all(root.join("player-backup")).unwrap();

        let result = delete_apply_backups_in(&mc, None).unwrap();
        assert_eq!(result.deleted, 2);
        assert!(root.join("player-backup").is_dir());
        assert!(!root.join("翻譯套用備份_20260811_1").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn applies_and_detects_config_text_overlays() {
        let root = std::env::temp_dir().join(format!("apply_config_overlay_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let work = root.join("翻譯結果");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(mc.join("config/unknown_display_mod")).unwrap();
        fs::create_dir_all(work.join("config/unknown_display_mod")).unwrap();
        fs::write(mc.join("config/unknown_display_mod/start.txt"), "原文").unwrap();
        fs::write(work.join("config/unknown_display_mod/start.txt"), "繁中").unwrap();

        let result = apply_to_instance(&mc, &work, None, true).unwrap();
        assert!(result.backup_created);
        assert_eq!(
            fs::read_to_string(mc.join("config/unknown_display_mod/start.txt")).unwrap(),
            "繁中"
        );
        assert!(has_apply_backups_in(&mc, Some(&work)).unwrap());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_rejects_manifest_for_different_mc_dir() {
        let root = std::env::temp_dir().join(format!("restore_mismatch_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let other = root.join("other_minecraft");
        let work = root.join("翻譯結果");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(other.join("mods")).unwrap();
        fs::create_dir_all(work.join("jar-translated")).unwrap();
        fs::write(mc.join("mods/example.jar"), b"original").unwrap();
        fs::write(work.join("jar-translated/example.jar"), b"translated").unwrap();

        let applied = apply_to_instance(&mc, &work, None, true).unwrap();
        let err = restore_last_apply_in(&other, Some(&work)).unwrap_err();
        assert!(err.contains("不符"));
        assert!(PathBuf::from(&applied.backup_dir).is_dir());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn does_not_copy_work_data_into_minecraft_data() {
        let root = std::env::temp_dir().join(format!("apply_no_mc_data_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let work = root.join("翻譯結果");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(work.join("jar-translated")).unwrap();
        fs::create_dir_all(work.join("data/example")).unwrap();
        fs::write(mc.join("mods/example.jar"), b"original").unwrap();
        fs::write(work.join("jar-translated/example.jar"), b"translated").unwrap();
        fs::write(work.join("data/example/book.json"), b"{}").unwrap();

        apply_to_instance(&mc, &work, None, false).unwrap();
        assert!(!mc.join("data/example/book.json").exists());
        assert_eq!(fs::read(mc.join("mods/example.jar")).unwrap(), b"translated");
        let _ = fs::remove_dir_all(root);
    }
}
