//! 一鍵套用到遊戲實例：先備份再複製資源包／任務／文字覆寫（社群期望：可裝、可回滾）。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::cancel;
use super::game_process::{self, GameRunning};
use super::jar_scan::resolve_minecraft_dir;
use super::out_layout::{ensure_result_layout, ResultLayout, RESULT_DIR_NAME};
use super::session::{find_session_file, is_tool_resource_pack, load_session};

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
/// 一鍵套用。**這是唯一的寫入入口**，遊戲關閉檢查在這裡做（P0-02）。
///
/// 為什麼不能只靠前端：`app.js` 的 `ensureGameClosed` 只擋得到它自己接線的按鈕，
/// 而 one_click／補翻／修復／單獨套用最後全都走到這個函式。前端檢查與實際寫入
/// 之間還有選路徑、跳確認等空窗，遊戲可能在那段時間被開起來。
pub fn apply_to_instance(
    instance_path: &Path,
    output_or_work: &Path,
    pack_name_hint: Option<&str>,
    create_backup: bool,
) -> Result<ApplyResult, String> {
    let running = game_process::is_game_running(instance_path);
    apply_to_instance_with_game_state(instance_path, output_or_work, pack_name_hint, create_backup, running)
}

/// 套用本體。把「偵測遊戲行程」這個會碰全域狀態的動作留在外層，
/// 這裡只依傳入的判定結果行事——測試才能覆蓋 Yes／No／Unknown 三種情境
/// 而不必真的去啟動一個 Minecraft。
pub fn apply_to_instance_with_game_state(
    instance_path: &Path,
    output_or_work: &Path,
    pack_name_hint: Option<&str>,
    create_backup: bool,
    running: GameRunning,
) -> Result<ApplyResult, String> {
    cancel::check()?;
    if !instance_path.exists() {
        return Err("找不到遊戲資料夾，請重新選擇。".into());
    }
    let mc = resolve_minecraft_dir(instance_path)?;
    crate::dev_log!(
        "apply",
        "開始套用 instance={} 來源={} 資源包提示={:?} 建立備份={}",
        instance_path.display(),
        output_or_work.display(),
        pack_name_hint,
        create_backup
    );
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

    // ── 後端最後一道遊戲關閉檢查（P0-02）──
    //
    // 位置刻意壓到這裡：上面全是唯讀的規劃（算 planned_targets），下一行才開始寫檔。
    // 檢查與第一次寫入之間愈短，「檢查完使用者才把遊戲打開」的空窗愈小。
    match &running {
        GameRunning::Yes { detail } => {
            crate::dev_log!("apply", "後端閘門擋下套用：遊戲正在執行 instance={}", instance_path.display());
            return Err(format!(
                "Minecraft 正在使用這個整合包，已停止套用。\n\
                 現在寫入可能造成檔案被鎖、只套用一半，或讓遊戲讀到壞掉的資源包。\n\
                 請完全關閉遊戲（含啟動器裡載入中的實例）後再套用一次。\n\
                 （{detail}）"
            ));
        }
        GameRunning::No => {}
        GameRunning::Unknown => {
            // 失效方向＝放行。偵測不出來（非 Windows、沒有 PowerShell、權限不足、逾時）
            // 若一律擋下，會把「查不到」變成新的卡關。但**不得偽裝成已確認關閉**：
            // 這句話會進 warnings，使用者看得到這次沒能確認。
            warnings.push(
                "這次無法確認 Minecraft 是否已關閉（偵測不可用），已照常套用。若遊戲內出現殘缺翻譯，請關閉遊戲後再套用一次。"
                    .into(),
            );
        }
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
            fs::create_dir_all(&bak)
                .map_err(|e| format!("建立資源包備份資料夾失敗：{e}"))?;
            // 同 backup_matching_tree：備份失敗不能吞，否則會「沒備份卻照樣覆蓋」
            fs::copy(&dest_rp, bak.join(name)).map_err(|e| {
                format!(
                    "備份既有資源包 {name} 失敗：{e}
已停止套用，避免在沒有備份的情況下覆蓋。常見原因是遊戲還開著把檔案鎖住，或磁碟空間不足。"
                )
            })?;
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
            fs::create_dir_all(&bak)
                .map_err(|e| format!("建立快捷選單備份資料夾失敗：{e}"))?;
            fs::copy(&menu_dest, bak.join("menu.json")).map_err(|e| {
                format!("備份既有快捷選單設定失敗：{e}
已停止套用，避免在沒有備份的情況下覆蓋。")
            })?;
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
        // 寫一份指紋標記，供下次「開始翻譯」判斷這個 zip 到底是不是這個整合包產生的
        // ——同一個實例路徑換了完全不同的整合包時，resourcepacks 裡的舊 zip 不會自動
        // 消失，沒有這個標記就只能憑檔名判斷「有就合併」，可能把舊包的翻譯誤merge進
        // 新包（見 lib.rs 的 run_one_click 合併點）。寫失敗不影響套用本身，忽略即可。
        let fingerprint = super::session::mods_fingerprint(instance_path);
        let meta_name = format!("{}.meta.json", name.trim_end_matches(".zip"));
        let meta = serde_json::json!({ "modsFingerprint": fingerprint });
        let _ = fs::write(
            rp.join(meta_name),
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        );
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
            for warn in collect_post_apply_warnings(&mc, work, Some(name)) {
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

    let warning_block = if warnings.is_empty() {
        String::new()
    } else {
        format!("\n\n注意：\n• {}", warnings.join("\n• "))
    };
    let player_summary = format!(
        "{player_summary}{warning_block}"
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
            // 備份失敗必須讓整個套用停下來，不能吞掉。
            //
            // 這個函式備份的是使用者的 config、kubejs 與**原始 mod JAR**，
            // 呼叫端接下來就會覆寫它們。舊版用 `let _ =` 忽略複製失敗，於是
            // 檔案被鎖住（遊戲還開著）、磁碟滿、或沒有寫入權限時，工具會
            // 「沒有備份卻照樣覆蓋」並回報成功——使用者事後按「還原上一次套用」
            // 才發現原始檔案已經永久消失。使用者選了備份就是要這份保障。
            fs::copy(&existing, &bak).map_err(|e| {
                format!(
                    "備份 {} 失敗：{e}\n為了不讓原始檔案在沒有備份的情況下被覆蓋，已停止套用。\
常見原因是遊戲還開著把檔案鎖住，或磁碟空間不足。",
                    existing.display()
                )
            })?;
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
        cancel::check()?;
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

/// 套用後檢查：多個工具 zip、options 未啟用本次包等。
fn collect_post_apply_warnings(
    mc: &Path,
    _work: &Path,
    applied_zip_name: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    let rp = mc.join("resourcepacks");
    if rp.is_dir() {
        let mut tool_count = 0usize;
        for entry in fs::read_dir(&rp).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let stem = name
                .trim_end_matches(".zip")
                .trim_end_matches(".ZIP");
            if is_tool_resource_pack(stem) {
                tool_count += 1;
            }
        }
        if tool_count >= 2 {
            out.push(format!(
                "遊戲 resourcepacks 內有 {tool_count} 個「模組包翻譯工具+*」資源包；請在遊戲設定裡只啟用最新一個，並停用舊版以免混亂。"
            ));
        }
    }
    if let Some(zip_name) = applied_zip_name {
        if !options_lists_resource_pack(mc, zip_name) {
            out.push(format!(
                "options.txt 似乎未啟用本次套用的「{zip_name}」；若進遊戲後沒看到繁中，請在資源包列表手動啟用。"
            ));
        }
    }
    out
}

fn options_lists_resource_pack(mc: &Path, zip_name: &str) -> bool {
    let options = mc.join("options.txt");
    let Ok(text) = fs::read_to_string(&options) else {
        return false;
    };
    let entry = format!("file/{zip_name}");
    text.lines().any(|line| {
        line.starts_with("resourcePacks:")
            && (line.contains(&format!("\"{entry}\"")) || line.contains(zip_name))
    })
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
    // options.txt 一律另存一份，不受「要不要備份」選項影響。
    //
    // 它只有幾 KB，但壞掉的代價是整個遊戲開不起來：使用者實測遇過資源包清單
    // 被清空，導致字體找不到材質 → 資源重載失敗 → 模型沒烘焙 → 標題畫面閃退。
    // 這種東西不該跟「要不要備份翻譯結果」綁在一起。
    if existed {
        let _ = fs::write(options.with_extension("txt.mcpl-bak"), &original);
    }
    // 動之前先記下清單，動完之後要驗證一個都沒少
    let packs_before = super::resource_pack_guard::parse_pack_list(&original);

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
    fs::write(&options, &updated).map_err(|e| format!("寫入 options.txt 失敗：{e}"))?;

    // 寫完馬上重讀驗證：原本清單裡的每一個資源包都必須還在。
    //
    // 這道驗證是使用者那次閃退換來的——工具動了 options.txt 卻沒有確認結果，
    // 等到遊戲缺材質、字體載入失敗、模型烘焙不完、標題畫面空指標才發現。
    // 少掉任何一項就把原檔寫回去並回報失敗，寧可不啟用翻譯包，也不能讓
    // 使用者的遊戲開不起來。
    let after_text = fs::read_to_string(&options).unwrap_or_default();
    let packs_after = super::resource_pack_guard::parse_pack_list(&after_text);
    let diff = super::resource_pack_guard::diff_pack_lists(&packs_before, &packs_after);
    if !diff.is_safe() {
        let _ = fs::write(&options, &original);
        return Err(format!(
            "啟用翻譯資源包時偵測到原本的資源包清單少了 {} 項（{}），已還原 options.txt 不做修改。\
請手動在遊戲的資源包畫面啟用「{zip_name}」。",
            diff.missing.len(),
            diff.missing.join("、")
        ));
    }
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
    fn backup_failure_stops_the_apply_instead_of_overwriting_unprotected() {
        // 這條釘死一個會造成永久資料遺失的缺陷：
        // backup_matching_tree 備份的是使用者的原始 mod JAR／config，呼叫端接著
        // 就會覆寫它們。舊版用 `let _ =` 吞掉複製失敗，於是檔案被鎖住（遊戲還開著）
        // 或磁碟滿的時候，會「沒有備份卻照樣覆蓋」並回報成功——使用者事後想還原
        // 才發現原始檔案已經沒了。備份失敗必須讓整個套用停下來。
        let root = std::env::temp_dir().join(format!("apply_bakfail_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let src = root.join("src");
        let dest = root.join("dest");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();
        fs::write(src.join("a.jar"), b"new").unwrap();
        fs::write(dest.join("a.jar"), b"original").unwrap();

        // 正常情況：備份得起來就成功，而且備份內容是「原始」那一份
        let bak_ok = root.join("bak_ok");
        backup_matching_tree(&src, &dest, &bak_ok).unwrap();
        assert_eq!(fs::read(bak_ok.join("a.jar")).unwrap(), b"original");

        // 失敗情況：備份根目錄被一個同名檔案佔住，建立子目錄／複製一定失敗
        let blocked = root.join("blocked");
        fs::write(&blocked, b"I am a file, not a directory").unwrap();
        let result = backup_matching_tree(&src, &dest, &blocked);
        assert!(result.is_err(), "備份失敗時必須回傳錯誤，不能靜默繼續");
        // 原始檔案必須原封不動——呼叫端會因為這個錯誤而中止，不會覆寫它
        assert_eq!(fs::read(dest.join("a.jar")).unwrap(), b"original");

        let _ = fs::remove_dir_all(&root);
    }

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

    /// 建一個「有東西可套用」的最小場景，回傳 (root, mc, work)。
    fn stage_applyable(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("apply_gate_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let work = root.join("翻譯結果");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(work.join("jar-translated")).unwrap();
        fs::write(mc.join("mods/example.jar"), b"original").unwrap();
        fs::write(work.join("jar-translated/example.jar"), b"translated").unwrap();
        (root, mc, work)
    }

    #[test]
    fn game_running_blocks_apply_with_zero_writes() {
        // P0-02：後端是最後一道關卡。遊戲開著時必須拒絕，而且**一個檔案都不能動**。
        let (root, mc, work) = stage_applyable("yes");
        let err = apply_to_instance_with_game_state(
            &mc,
            &work,
            None,
            true,
            GameRunning::Yes { detail: "偵測到這個整合包的遊戲行程正在執行。".into() },
        )
        .unwrap_err();

        assert!(err.contains("Minecraft 正在使用這個整合包"), "訊息要說得出發生什麼：{err}");
        assert!(err.contains("關閉遊戲"), "要告訴使用者下一步怎麼做：{err}");

        // 零寫入：原始檔沒被換掉，也沒有留下任何備份資料夾
        assert_eq!(fs::read(mc.join("mods/example.jar")).unwrap(), b"original");
        let leftovers: Vec<_> = fs::read_dir(&work)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("翻譯套用備份_"))
            .collect();
        assert!(leftovers.is_empty(), "被擋下時不得留下半成品備份目錄");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_game_state_proceeds_but_says_so() {
        // 偵測不出來時失效方向朝放行（不能把「查不到」變成新的卡關），
        // 但不得偽裝成已確認關閉——使用者要看得到這次沒能確認。
        let (root, mc, work) = stage_applyable("unknown");
        let result =
            apply_to_instance_with_game_state(&mc, &work, None, true, GameRunning::Unknown).unwrap();

        assert_eq!(fs::read(mc.join("mods/example.jar")).unwrap(), b"translated", "Unknown 應放行");
        assert!(
            result.warnings.iter().any(|w| w.contains("無法確認")),
            "Unknown 必須留下未確認的紀錄，實際警告：{:?}",
            result.warnings
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn confirmed_closed_game_has_no_unconfirmed_warning() {
        // 確定沒開時不該冒出「無法確認」這種讓人以為有問題的字。
        let (root, mc, work) = stage_applyable("no");
        let result =
            apply_to_instance_with_game_state(&mc, &work, None, true, GameRunning::No).unwrap();
        assert!(!result.warnings.iter().any(|w| w.contains("無法確認")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn only_yes_blocks_apply() {
        assert!(GameRunning::Yes { detail: String::new() }.blocks_apply());
        assert!(!GameRunning::No.blocks_apply());
        // 這條是刻意的：查不出來不擋，否則沒有 PowerShell 的環境永遠套用不了
        assert!(!GameRunning::Unknown.blocks_apply());
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

        let result = apply_to_instance_with_game_state(&mc, &work, None, true, GameRunning::No).unwrap();
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

        let first = apply_to_instance_with_game_state(&mc, &work, None, true, GameRunning::No).unwrap();
        let first_backup = first.backup_dir.clone();
        fs::write(work.join("jar-translated/example.jar"), b"translated-v2").unwrap();
        let second = apply_to_instance_with_game_state(&mc, &work, None, true, GameRunning::No).unwrap();

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

        let result = apply_to_instance_with_game_state(&mc, &work, None, false, GameRunning::No).unwrap();
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

        let result = apply_to_instance_with_game_state(&mc, &work, None, true, GameRunning::No).unwrap();
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

        let applied = apply_to_instance_with_game_state(&mc, &work, None, true, GameRunning::No).unwrap();
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

        apply_to_instance_with_game_state(&mc, &work, None, false, GameRunning::No).unwrap();
        assert!(!mc.join("data/example/book.json").exists());
        assert_eq!(fs::read(mc.join("mods/example.jar")).unwrap(), b"translated");
        let _ = fs::remove_dir_all(root);
    }
}
