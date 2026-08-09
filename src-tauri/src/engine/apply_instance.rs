//! 一鍵套用到遊戲實例：先備份再複製資源包／任務／文字覆寫（社群期望：可裝、可回滾）。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::jar_scan::resolve_minecraft_dir;
use super::out_layout::{ensure_result_layout, ResultLayout};
use super::session::{find_session_file, load_session};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub backup_dir: String,
    pub zip_copied: Option<String>,
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
}

pub fn restore_last_apply(instance_path: &Path) -> Result<RestoreResult, String> {
    let mc = resolve_minecraft_dir(instance_path)?;
    let parent = mc.parent().unwrap_or(&mc);
    // 找最新的 翻譯套用備份_*
    let mut backups: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = fs::read_dir(parent) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir()
                && p.file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.starts_with("翻譯套用備份_"))
                    .unwrap_or(false)
            {
                backups.push(p);
            }
        }
    }
    if backups.is_empty() {
        return Err("找不到任何『翻譯套用備份_』資料夾，沒有可還原的套用紀錄。".into());
    }
    backups.sort(); // 時間戳在名字裡，字典序≈時間序
    let backup_root = backups.last().unwrap().clone();

    let manifest_path = backup_root.join(APPLY_MANIFEST);
    let mut removed = 0usize;
    let mut restored = 0usize;

    if let Ok(text) = fs::read_to_string(&manifest_path) {
        let manifest: ApplyManifest = serde_json::from_str(&text)
            .map_err(|e| format!("套用清單讀取失敗：{e}"))?;
        // 新增的 → 刪除
        for rel in &manifest.added {
            let p = mc.join(rel);
            if p.is_file() && fs::remove_file(&p).is_ok() {
                removed += 1;
            }
        }
        // 覆蓋的 → 從備份複製回來（備份鏡像 mc 相對結構）
        for rel in &manifest.overwritten {
            let from = backup_root.join(rel);
            let to = mc.join(rel);
            if from.is_file() {
                if let Some(parent) = to.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if fs::copy(&from, &to).is_ok() {
                    restored += 1;
                }
            }
        }
    } else {
        // 舊備份沒有清單：退回「把備份內容整包蓋回去」（只能還原覆蓋，無法刪掉新增的）
        for sub in ["resourcepacks", "config", "minemenu", "patchouli_books", "kubejs", "datapacks"] {
            let from = backup_root.join(sub);
            if from.is_dir() {
                restored += restore_tree(&from, &mc.join(sub));
            }
        }
    }

    let player_summary = format!(
        "已還原上次套用。\n\
• 備份來源：\n{}\n\
• 移除本次新增檔：{} 個\n\
• 還原被覆蓋檔：{} 個\n\n\
現在再開一次遊戲：\n\
• 若開得起來 → 先前是翻譯檔造成的，歡迎把當機報告給我們修\n\
• 若還是開不起來 → 不是翻譯，多半是整合包缺模組（可用『診斷開不了』看是缺什麼）\n\
（本工具不改 mods/*.jar）",
        backup_root.display(),
        removed,
        restored
    );
    Ok(RestoreResult {
        backup_dir: backup_root.display().to_string(),
        removed,
        restored,
        player_summary,
    })
}

fn restore_tree(from: &Path, to: &Path) -> usize {
    let mut n = 0usize;
    for entry in walkdir::WalkDir::new(from).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let rel = path.strip_prefix(from).unwrap_or(path);
        let target = to.join(rel);
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if fs::copy(path, &target).is_ok() {
            n += 1;
        }
    }
    n
}

/// 將「翻譯結果」套用到遊戲：resourcepacks zip + config/ftbquests + minemenu
/// + patchouli_books / config/openloader / kubejs（若 work 有）。
/// 絕不改 mods/*.jar。套用前備份會被覆蓋的目標。
pub fn apply_to_instance(
    instance_path: &Path,
    output_or_work: &Path,
    pack_name_hint: Option<&str>,
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
    let quests_src = work.join("config").join("ftbquests");
    let menu_src = work.join("minemenu").join("menu.json");
    let patchouli_src = work.join("patchouli_books");
    let openloader_src = work.join("config").join("openloader");
    let kubejs_src = work.join("kubejs");
    let fancymenu_src = work.join("config").join("fancymenu");
    let datapacks_src = work.join("datapacks");

    let has_patchouli = dir_has_files(&patchouli_src);
    let has_openloader = dir_has_files(&openloader_src);
    let has_kubejs = dir_has_files(&kubejs_src);
    let has_fancymenu = dir_has_files(&fancymenu_src);
    let has_datapacks = dir_has_files(&datapacks_src);

    if zip_src.is_none()
        && !quests_src.is_dir()
        && !menu_src.is_file()
        && !has_patchouli
        && !has_openloader
        && !has_kubejs
        && !has_fancymenu
        && !has_datapacks
    {
        return Err(format!(
            "在「{}」找不到可套用的 zip／任務／快捷選單／文字覆寫。請先完成一鍵翻譯。",
            work.display()
        ));
    }

    let stamp = backup_stamp();
    let backup_root = mc
        .parent()
        .unwrap_or(&mc)
        .join(format!("翻譯套用備份_{stamp}"));
    fs::create_dir_all(&backup_root).map_err(|e| format!("無法建立備份目錄：{e}"))?;

    // ── 備份現有資源包 ──
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

    // ── 備份 ftbquests ──
    let quests_dest = mc.join("config").join("ftbquests");
    if quests_src.is_dir() && quests_dest.is_dir() {
        let bak = backup_root.join("config").join("ftbquests");
        copy_dir_recursive(&quests_dest, &bak)?;
    }

    // ── 備份 minemenu ──
    let menu_dest = mc.join("minemenu").join("menu.json");
    if menu_src.is_file() && menu_dest.is_file() {
        let bak = backup_root.join("minemenu");
        fs::create_dir_all(&bak).ok();
        let _ = fs::copy(&menu_dest, bak.join("menu.json"));
    }

    // ── 備份 patchouli_books ──
    let patchouli_dest = mc.join("patchouli_books");
    if has_patchouli && patchouli_dest.is_dir() {
        let bak = backup_root.join("patchouli_books");
        copy_dir_recursive(&patchouli_dest, &bak)?;
    }

    // ── 備份 config/openloader ──
    let openloader_dest = mc.join("config").join("openloader");
    if has_openloader && openloader_dest.is_dir() {
        let bak = backup_root.join("config").join("openloader");
        copy_dir_recursive(&openloader_dest, &bak)?;
    }

    // ── 備份 kubejs（僅 work 會覆寫的相對路徑）──
    let kubejs_dest = mc.join("kubejs");
    if has_kubejs && kubejs_dest.is_dir() {
        backup_matching_tree(&kubejs_src, &kubejs_dest, &backup_root.join("kubejs"))?;
    }

    let fancymenu_dest = mc.join("config").join("fancymenu");
    if has_fancymenu && fancymenu_dest.is_dir() {
        let bak = backup_root.join("config").join("fancymenu");
        copy_dir_recursive(&fancymenu_dest, &bak)?;
    }

    let datapacks_dest = mc.join("datapacks");
    if has_datapacks && datapacks_dest.is_dir() {
        let bak = backup_root.join("datapacks");
        copy_dir_recursive(&datapacks_dest, &bak)?;
    }

    // 寫備份說明
    let bak_note = format!(
        "【翻譯套用備份】\n\
時間戳：{stamp}\n\
遊戲目錄：{}\n\
翻譯結果：{}\n\
\n\
還原方式：\n\
1. 關閉遊戲\n\
2. 把本備份內 resourcepacks / config / minemenu / patchouli_books / kubejs / datapacks 對應複製回遊戲\n\
3. 勿刪未備份的其他自訂檔\n\
4. 本工具從不改 mods/*.jar\n",
        mc.display(),
        work.display()
    );
    let _ = fs::write(backup_root.join("還原說明.txt"), bak_note);

    // 套用清單（供一鍵還原）
    let mut manifest = ApplyManifest {
        stamp: stamp.clone(),
        mc_dir: mc.display().to_string(),
        backup_dir: backup_root.display().to_string(),
        ..Default::default()
    };

    // ── 複製 zip ──
    let mut zip_copied = None;
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

    // ── 複製任務 ──
    let mut quests_copied = false;
    if quests_src.is_dir() {
        fs::create_dir_all(mc.join("config")).map_err(|e| e.to_string())?;
        merge_copy_dir(&quests_src, &quests_dest, &mc, &mut manifest)?;
        quests_copied = true;
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

    // ── 複製 config/openloader（merge）──
    let mut openloader_copied = false;
    if has_openloader {
        fs::create_dir_all(mc.join("config")).map_err(|e| e.to_string())?;
        merge_copy_dir(&openloader_src, &openloader_dest, &mc, &mut manifest)?;
        openloader_copied = true;
    }

    // ── 複製 kubejs（work 僅含翻譯產出；merge，不碰 mods）──
    let mut kubejs_copied = false;
    if has_kubejs {
        merge_copy_dir(&kubejs_src, &kubejs_dest, &mc, &mut manifest)?;
        kubejs_copied = true;
    }

    let mut fancymenu_copied = false;
    if has_fancymenu {
        fs::create_dir_all(mc.join("config")).map_err(|e| e.to_string())?;
        merge_copy_dir(&fancymenu_src, &fancymenu_dest, &mc, &mut manifest)?;
        fancymenu_copied = true;
    }

    let mut datapacks_copied = false;
    if has_datapacks {
        merge_copy_dir(&datapacks_src, &datapacks_dest, &mc, &mut manifest)?;
        datapacks_copied = true;
    }

    // 寫套用清單（供「一鍵還原」精準反轉）
    if let Ok(js) = serde_json::to_string_pretty(&manifest) {
        let _ = fs::write(backup_root.join(APPLY_MANIFEST), js + "\n");
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
        if datapacks_copied {
            parts.push("datapacks");
        }
        if parts.is_empty() {
            "無／未複製".into()
        } else {
            format!("已合併 {}", parts.join("、"))
        }
    };

    let player_summary = format!(
        "已套用到遊戲（先備份再複製；目標＝整合包可遊玩文字→台灣繁中（除圖片）；不改 jar）\n\
• 備份目錄：\n{}\n\
• 資源包：{}\n\
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
        backup_root.display(),
        zip_copied
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or("（本次無 zip）"),
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
        backup_dir: backup_root.display().to_string(),
        zip_copied,
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
