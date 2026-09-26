//! 移除翻譯（依套用紀錄、遊戲裡的標記與舊版清單反轉），不需要備份也能執行。
//!
//! 每個檔都先過前置條件（apply_guard.rs），條件不符就不動檔並列出：
//! - 遊戲檔標記要與紀錄、批次互相對得上（缺一端能重建就重建，這次不動檔）。
//! - 目前內容要等於工具寫入的版本，否則是玩家或整合包改過的 → 不動。
//! - 工具新增的 → 刪除；工具覆蓋的 → 備份標記屬本包、有效、互指、指紋相符才放回，放回後標成已用過。
//! - 來源不明的（原本的內容在隔離區）→ 不動，列出。
//! - 設定檔：清單項目標記要與資源包檔標記互指、語言標記要與紀錄一致，才逐行還原。
//! - 1.0.x 舊版清單上的檔：清單屬本包、內容比對得上工具產出，並先認領舊備份（補標記）才動。

use std::fs;

use super::apply_guard::{self, Check, Ctx};
use super::apply_knowledge::{self, Knowledge};
use super::apply_notice::language_display_name;
use super::apply_record::{self, FileKind, Origin};
use super::mcpl_marker as mk;
use super::options_txt;
use super::paths::long_path;

#[derive(Debug, Clone, Default)]
pub struct RecordRestore {
    pub restored_files: Vec<String>,
    pub removed_files: Vec<String>,
    /// 套用後被改過、所以沒有動的檔
    pub skipped_modified: Vec<String>,
    /// 無法確認（標記對不上、來源不明、舊版無法比對）、所以沒有動的檔
    pub uncertain: Vec<String>,
    /// 標記有缺、已從另一端補回；這次沒有動檔，再按一次移除翻譯就會處理
    pub repaired: Vec<String>,
    /// 確定是工具寫的，但當初沒有備份，無法放回原檔
    pub unrestorable: Vec<String>,
    pub options_restored: bool,
    pub lang_restored_to: Option<String>,
    /// 刪除或複製失敗（多半是遊戲開著鎖住檔案）
    pub failures: Vec<String>,
    /// 隔離區裡保存的玩家原本版本：「檔案 — 保存位置」
    pub quarantined: Vec<String>,
    /// 備份已被玩家刪掉、無法還原的被覆蓋檔
    pub backup_deleted: Vec<String>,
    /// 上次套用中途中斷、還沒被翻譯寫入的檔（保持原樣）
    pub not_written: Vec<String>,
}

fn push_unique(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}

/// `owner`：空＝移除翻譯；`apply_record::OWNER_FONT`＝移除字體包。只處理該功能放的東西。
pub fn restore_all(ctx: &Ctx, knowledge: &mut Knowledge, owner: &str) -> RecordRestore {
    let mut out = RecordRestore::default();
    // 設定檔先處理：它要檢查資源包檔的標記，檔案處理完標記就移除了
    restore_options(ctx, knowledge, &mut out, owner);
    restore_recorded_files(ctx, knowledge, &mut out, owner);
    if owner.is_empty() {
        restore_legacy_files(ctx, knowledge, &mut out);
    }
    out
}

fn forget(ctx: &Ctx, knowledge: &mut Knowledge, rel: &str) {
    knowledge.record.files.remove(rel);
    knowledge.record.unconfirmed.remove(rel);
    mk::remove_file_marker(&ctx.mc, rel);
}

fn restore_recorded_files(ctx: &Ctx, knowledge: &mut Knowledge, out: &mut RecordRestore, owner: &str) {
    let rels: Vec<String> =
        knowledge.record.files.iter().filter(|(_, e)| e.owner == owner).map(|(rel, _)| rel.clone()).collect();
    for rel in rels {
        let target = ctx.mc.join(&rel);
        if !long_path(&target).is_file() {
            forget(ctx, knowledge, &rel);
            continue;
        }
        // 前置條件一：遊戲檔標記完整
        let game = match apply_guard::require_game_marker(ctx, &knowledge.record, &rel) {
            Check::Ready(game) => game,
            Check::Repaired(why) => {
                crate::dev_log!("restore", "{rel}：{why}");
                push_unique(&mut out.repaired, rel);
                continue;
            }
            Check::Broken(why) => {
                crate::dev_log!("restore", "{rel}：{why}");
                push_unique(&mut out.uncertain, rel);
                continue;
            }
        };
        // 前置條件二：目前內容是工具寫入的版本
        let current = apply_record::file_sha256(&target);
        // 內容等於任一版工具寫入（含中途中斷時「待確認」的那版）就算工具檔
        let tool_version = knowledge.record.is_tool_version(&rel, current.as_deref())
            || game.is_tool_content(current.as_deref());
        if !tool_version {
            if super::apply_pending::is_untouched(&ctx.mc, &knowledge.record, &game, current.as_deref()) {
                // 上次中途中斷、還沒寫入：它就是套用前的樣子，不是被改過
                push_unique(&mut out.not_written, rel.clone());
                forget(ctx, knowledge, &rel);
            } else if knowledge.record.unconfirmed.contains(&rel) {
                // 識別碼認回時就對不上：無法確認，不動也不忘記
                push_unique(&mut out.uncertain, rel);
            } else {
                push_unique(&mut out.skipped_modified, rel.clone());
                forget(ctx, knowledge, &rel);
            }
            continue;
        }
        let entry = knowledge.record.files[&rel].clone();
        if entry.origin == Origin::Unknown || game.origin == "unknown" {
            // 來源不明：不動，但要告訴玩家他原本的版本放在隔離區哪裡
            let id = mk::linked(&game.links, mk::REL_QUARANTINE).unwrap_or_else(|| entry.quarantine_id.clone());
            if let Some(path) = quarantine_location(&ctx.mc, &id) {
                push_unique(
                    &mut out.quarantined,
                    format!("{} — 保存位置：{}", display_rel(&rel), path.display()),
                );
            }
            push_unique(&mut out.uncertain, rel);
            continue;
        }
        match entry.kind {
            FileKind::Added => {
                if !apply_guard::require_deletable(&game) {
                    push_unique(&mut out.uncertain, rel);
                    continue;
                }
                match fs::remove_file(long_path(&target)) {
                    Ok(()) => {
                        push_unique(&mut out.removed_files, rel.clone());
                        forget(ctx, knowledge, &rel);
                    }
                    Err(error) => out.failures.push(format!("無法刪除「{rel}」：{error}")),
                }
            }
            FileKind::Overwritten => {
                if mk::linked(&game.links, mk::REL_BACKUP).is_none() {
                    // 當初選了不備份：留在紀錄裡，檔案仍是翻譯版
                    push_unique(&mut out.unrestorable, rel);
                    continue;
                }
                match apply_guard::require_restorable(ctx, &game) {
                    Check::Ready((backup, marker)) => match apply_record::copy_atomic(&backup, &target) {
                        Ok(()) => {
                            if let Err(e) = apply_guard::mark_backup_used(ctx, &marker) {
                                out.failures.push(format!("已放回「{rel}」，但無法把備份標成已用過：{e}"));
                            }
                            push_unique(&mut out.restored_files, rel.clone());
                            forget(ctx, knowledge, &rel);
                        }
                        Err(error) => out.failures.push(format!("無法還原「{rel}」：{error}")),
                    },
                    Check::Repaired(why) => {
                        crate::dev_log!("restore", "{rel}：{why}");
                        push_unique(&mut out.repaired, rel)
                    }
                    Check::Broken(why) if knowledge.record.backups_deleted => {
                        crate::dev_log!("restore", "{rel}：{why}（玩家刪過備份）");
                        push_unique(&mut out.backup_deleted, rel)
                    }
                    Check::Broken(why) => {
                        crate::dev_log!("restore", "{rel}：{why}");
                        push_unique(&mut out.uncertain, rel)
                    }
                }
            }
        }
    }
}

/// 依隔離標記 id 找出隔離區裡保存的檔（標記旁的同名檔）。
fn quarantine_location(mc: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
    if id.is_empty() {
        return None;
    }
    let root = apply_record::quarantine_dir(mc);
    walkdir::WalkDir::new(long_path(&root)).into_iter().filter_map(|e| e.ok()).find_map(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        let stem = name.strip_suffix(apply_guard::QUARANTINE_MARKER_SUFFIX)?;
        let text = fs::read_to_string(e.path()).ok()?;
        let marker: apply_guard::QuarantineMarker = serde_json::from_str(&text).ok()?;
        let saved = e.path().with_file_name(stem);
        (marker.id == id && saved.is_file()).then(|| root.join(saved.strip_prefix(long_path(&root)).unwrap_or(&saved)))
    })
}

/// 舊版清單上、新紀錄沒有涵蓋的檔（1.0.x 套用過、新版還沒動過的）。只用屬本包的清單。
fn restore_legacy_files(ctx: &Ctx, knowledge: &mut Knowledge, out: &mut RecordRestore) {
    for (rel, was_added) in knowledge.legacy_listed() {
        if knowledge.record.files.contains_key(&rel)
            || knowledge.record.legacy_done.contains(&rel)
            || rel == "options.txt"
        {
            // 新紀錄已處理；options.txt 一律逐行還原，不整檔蓋回
            continue;
        }
        let target = ctx.mc.join(&rel);
        if !long_path(&target).is_file() {
            continue;
        }
        let current = apply_record::file_sha256(&target);
        let known_tool = apply_knowledge::matches_known_tool_output(&knowledge.work_roots, &rel, current.as_deref())
            || apply_knowledge::is_identifiable_tool_product(&target);
        if !known_tool {
            push_unique(&mut out.uncertain, rel);
            continue;
        }
        if was_added {
            match fs::remove_file(long_path(&target)) {
                Ok(()) => {
                    push_unique(&mut out.removed_files, rel.clone());
                    knowledge.record.legacy_done.insert(rel);
                }
                Err(error) => out.failures.push(format!("無法刪除「{rel}」：{error}")),
            }
            continue;
        }
        let Some((original, manifest_dir)) = knowledge.legacy_original(&rel) else {
            push_unique(&mut out.unrestorable, rel);
            continue;
        };
        // 前置條件：先認領舊備份（複製進本包備份區並補標記），再照一般規則放回
        let game_id = mk::new_marker_id();
        let claimed = apply_guard::require_legacy_claimed(ctx, &rel, &original, &manifest_dir, &game_id)
            .and_then(|_| {
                apply_guard::read_backup_marker(&ctx.mc, &rel).ok_or_else(|| "認領後找不到備份標記".to_string())
            });
        let marker = match claimed {
            Ok(marker) => marker,
            Err(error) => {
                out.failures.push(format!("無法認領舊版備份「{rel}」：{error}"));
                continue;
            }
        };
        match apply_record::copy_atomic(&apply_guard::backup_file_path(&ctx.mc, &rel), &target) {
            Ok(()) => {
                if let Err(e) = apply_guard::mark_backup_used(ctx, &marker) {
                    out.failures.push(format!("已放回「{rel}」，但無法把備份標成已用過：{e}"));
                }
                push_unique(&mut out.restored_files, rel.clone());
                knowledge.record.legacy_done.insert(rel);
            }
            Err(error) => out.failures.push(format!("無法還原「{rel}」：{error}")),
        }
    }
}

/// 資源包清單項目：清單項目標記 ↔ 資源包檔標記要互指，才拿掉這個項目。
/// 清單項目標記不見但資源包檔標記還指著它 → 補回標記（這次不動設定檔的這一項）。
fn entry_link_check(ctx: &Ctx, options_marker: &mut mk::OptionsMarker, entry: &str) -> Check<()> {
    let all_markers = mk::all_file_markers(&ctx.mc);
    if let Some(em) = options_marker.entries.iter().find(|e| e.entry == entry) {
        let Some(game_id) = mk::linked(&em.links, mk::REL_GAME_FILE) else {
            return Check::Broken("清單項目標記沒有指向資源包檔".into());
        };
        let back = all_markers
            .iter()
            .any(|m| m.id == game_id && mk::links_to(&m.links, mk::REL_PACK_ENTRY, &em.id));
        return if back { Check::Ready(()) } else { Check::Broken("清單項目與資源包檔的標記互指不一致".into()) };
    }
    // 另一端還在：資源包檔標記指向一個清單項目 id，而且那個檔就是這一項
    let file_name = entry.strip_prefix("file/").unwrap_or(entry);
    let owner = all_markers.iter().find(|m| {
        mk::linked(&m.links, mk::REL_PACK_ENTRY).is_some()
            && m.rel.strip_prefix("resourcepacks/").is_some_and(|r| r == file_name || r.starts_with(&format!("{file_name}/")))
    });
    match owner {
        Some(m) => {
            options_marker.entries.push(mk::EntryMarker {
                id: mk::linked(&m.links, mk::REL_PACK_ENTRY).unwrap_or_default(),
                entry: entry.to_string(),
                links: vec![mk::link(&m.id, mk::REL_GAME_FILE)],
            });
            Check::Repaired("清單項目標記不見了，已從資源包檔標記補回".into())
        }
        None => Check::Broken("找不到這個清單項目的標記".into()),
    }
}

fn restore_options(ctx: &Ctx, knowledge: &mut Knowledge, out: &mut RecordRestore, owner: &str) {
    let font = !owner.is_empty();
    let added = if font {
        knowledge.record.options.font_packs_added.clone()
    } else {
        knowledge.record.options.packs_added.clone()
    };
    let lang_changed = !font && knowledge.record.options.lang_changed;
    if added.is_empty() && !lang_changed {
        return;
    }
    let options = ctx.mc.join("options.txt");
    let Ok(text) = fs::read_to_string(long_path(&options)) else {
        // 遊戲資料夾已沒有 options.txt：沒有東西要還原
        knowledge.record.options = Default::default();
        return;
    };
    let mut marker = mk::read_options_marker(&ctx.mc);
    let mut removable = Vec::new();
    let mut kept_entries = Vec::new();
    for entry in added {
        match entry_link_check(ctx, &mut marker, &entry) {
            Check::Ready(()) => removable.push(entry),
            Check::Repaired(why) => {
                crate::dev_log!("restore", "{entry}：{why}");
                push_unique(&mut out.repaired, format!("資源包清單的「{entry}」"));
                kept_entries.push(entry);
            }
            Check::Broken(why) => {
                crate::dev_log!("restore", "{entry}：{why}");
                push_unique(&mut out.uncertain, format!("資源包清單的「{entry}」"));
                kept_entries.push(entry);
            }
        }
    }
    // 語言：語言標記要在、記的原語言要與紀錄一致、指回的那次套用要在紀錄裡
    let mut lang_ok = false;
    if lang_changed {
        match &marker.lang {
            Some(lang) => {
                let batch_ok = mk::linked(&lang.links, mk::REL_BATCH)
                    .is_some_and(|b| knowledge.record.batches.iter().any(|x| x.id == b && x.markers.contains(&lang.id)));
                if lang.original_lang == knowledge.record.options.original_lang && batch_ok {
                    lang_ok = true;
                } else {
                    push_unique(&mut out.uncertain, "遊戲語言設定".into());
                }
            }
            None => {
                // 照紀錄重建：沿用原本的標記 id，並連回它那次套用的批次
                let recorded = &knowledge.record.options;
                let id = Some(recorded.lang_marker_id.clone()).filter(|id| !id.is_empty()).unwrap_or_else(mk::new_marker_id);
                let links = Some(recorded.lang_batch.clone())
                    .filter(|b| !b.is_empty())
                    .map(|b| vec![mk::link(&b, mk::REL_BATCH)])
                    .unwrap_or_default();
                marker.lang = Some(mk::LangMarker { id, original_lang: recorded.original_lang.clone(), links });
                push_unique(&mut out.repaired, "遊戲語言設定".into());
            }
        }
    }
    let (updated, changed) = options_txt::restore_lines(
        &text,
        &removable,
        lang_ok,
        knowledge.record.options.original_lang.as_deref(),
    );
    if changed {
        if let Err(error) = apply_record::write_atomic(&options, updated.as_bytes()) {
            out.failures.push(format!("無法還原遊戲設定檔：{error}"));
            return;
        }
        out.options_restored = !removable.is_empty();
        if lang_ok && options_txt::read_lang(&updated) != options_txt::read_lang(&text) {
            out.lang_restored_to = Some(language_display_name(knowledge.record.options.original_lang.as_deref()));
        }
    }
    marker.entries.retain(|e| !removable.contains(&e.entry));
    if font {
        knowledge.record.options.font_packs_added = kept_entries;
    } else {
        knowledge.record.options.packs_added = kept_entries;
    }
    if lang_ok {
        knowledge.record.options.lang_changed = false;
        knowledge.record.options.original_lang = None;
        marker.lang = None;
    }
    let _ = mk::write_options_marker(&ctx.mc, &marker);
}

/// 給玩家看的結果說明。
pub fn describe(result: &RecordRestore) -> String {
    let mut lines = vec![
        "已移除翻譯。".to_string(),
        format!("• 已刪除工具加入的檔案：{} 個", result.removed_files.len()),
        format!("• 已還原被覆蓋的原檔：{} 個", result.restored_files.len()),
    ];
    if result.options_restored {
        lines.push("• 資源包清單：已拿掉工具加的資源包".into());
    }
    if let Some(lang) = &result.lang_restored_to {
        lines.push(format!("• 遊戲語言：已改回 {lang}"));
    }
    if !result.restored_files.is_empty() {
        lines.push(format!("\n已還原的檔案：\n{}", list_preview(&result.restored_files)));
    }
    if !result.removed_files.is_empty() {
        lines.push(format!("\n已刪除的檔案：\n{}", list_preview(&result.removed_files)));
    }
    if !result.skipped_modified.is_empty() {
        lines.push(format!(
            "\n以下 {} 個檔案在套用後被你或整合包更新改過，為了不蓋掉你的修改，沒有動它們：\n{}",
            result.skipped_modified.len(),
            list_preview(&result.skipped_modified)
        ));
    }
    if !result.not_written.is_empty() {
        lines.push(format!(
            "\n以下 {} 個檔案上次套用到一半就停了，這個檔這次還沒被翻譯寫入，保持原樣：\n{}",
            result.not_written.len(),
            list_preview(&result.not_written)
        ));
    }
    if !result.repaired.is_empty() {
        lines.push(format!(
            "\n以下 {} 項的標記有缺，已經補好，但這次沒有動它們；再按一次「移除翻譯」就會處理：\n{}",
            result.repaired.len(),
            list_preview(&result.repaired)
        ));
    }
    if !result.uncertain.is_empty() {
        lines.push(format!(
            "\n以下 {} 項無法確認是不是工具改的（或原本的檔在隔離區），沒有動它。\
若要完全回到原版，可以在啟動器裡重新安裝這個模組整合包：\n{}",
            result.uncertain.len(),
            list_preview(&result.uncertain)
        ));
    }
    if !result.quarantined.is_empty() {
        let shown: Vec<String> = result.quarantined.iter().take(10).map(|q| format!("  - {q}")).collect();
        lines.push(format!(
            "\n以下 {} 個檔案，套用前你原本的版本已移到隔離區保存（沒有刪除）。\
要用回你的版本，把它從保存位置複製回遊戲資料夾同一個位置即可：\n{}",
            result.quarantined.len(),
            shown.join("\n")
        ));
    }
    if !result.backup_deleted.is_empty() {
        lines.push(format!(
            "\n備份已刪除，無法還原被覆蓋的檔案：以下 {} 個檔案仍是翻譯版。\
若要完全回到原版，可以在啟動器裡重新安裝這個模組整合包：\n{}",
            result.backup_deleted.len(),
            list_preview(&result.backup_deleted)
        ));
    }
    if !result.unrestorable.is_empty() {
        lines.push(format!(
            "\n以下 {} 個檔案當初沒有備份，無法還原成原本的內容（仍是翻譯版）。\
若要完全回到原版，可以在啟動器裡重新安裝這個模組整合包：\n{}",
            result.unrestorable.len(),
            list_preview(&result.unrestorable)
        ));
    }
    lines.join("\n")
}

/// 給玩家看的檔名：工具自己的檔、資源包、模組檔用白話說；其餘照相對路徑。
pub fn display_rel(rel: &str) -> String {
    if rel == "options.txt.mcpl-bak" {
        return "遊戲設定檔旁的另存檔".into();
    }
    if let Some(rest) = rel.strip_prefix("resourcepacks/") {
        if let Some(stem) = rest.strip_suffix(".meta.json") {
            return format!("資源包「{stem}」的指紋標記");
        }
        let (pack, inner) = rest.split_once('/').unwrap_or((rest, ""));
        return if inner.is_empty() {
            format!("資源包「{pack}」")
        } else {
            format!("資源包「{pack}」裡的 {inner}")
        };
    }
    if let Some(rest) = rel.strip_prefix("mods/") {
        return format!("模組檔「{rest}」");
    }
    rel.to_string()
}

fn list_preview(items: &[String]) -> String {
    let mut shown: Vec<String> = items.iter().take(10).map(|s| format!("  - {}", display_rel(s))).collect();
    if items.len() > 10 {
        shown.push(format!("  …另有 {} 個", items.len() - 10));
    }
    shown.join("\n")
}
