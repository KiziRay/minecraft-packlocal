//! 一鍵套用到遊戲：依套用清單把翻譯放進遊戲資料夾、啟用資源包、把遊戲語言設成繁中。
//!
//! 每個會備份／覆蓋／還原／刪除的動作都先過 apply_guard.rs 的前置條件檢查；
//! 紀錄與標記（apply_record.rs、mcpl_marker.rs）一律先寫，才動遊戲檔。

use std::fs;
use std::path::{Path, PathBuf};

use super::apply_guard::{self, Ctx};
use super::apply_knowledge::{self, Class, Knowledge};
#[cfg(test)]
use super::apply_knowledge::APPLY_MANIFEST;
use super::apply_plan::{self, Group, ItemSource};
use super::apply_notice::language_display_name;
use super::apply_record::{self, ApplyRecord, BackupPolicy, FileKind, Origin};
use super::apply_restore;
use super::cancel;
use super::game_process::{self, GameRunning};
use super::hashutil::sha256_hex;
use super::jar_scan::resolve_minecraft_dir;
use super::options_txt;
use super::out_layout::{ensure_result_layout, ResultLayout};
use super::paths::long_path;
use super::session::{find_session_file, is_tool_resource_pack, load_session};

/// 套用的結果狀態。只有 `Applied` 代表檔案已經放進遊戲；其餘都是「翻譯已完成、還沒套用」，
/// 而且一個檔都沒動——玩家處理完（關遊戲、先開一次遊戲、選備份、確認覆蓋）再按「套用到遊戲」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplyStatus {
    Applied,
    /// 遊戲正在執行
    GameRunning,
    /// 全新的遊戲資料夾（沒有 options.txt）：請先啟動一次遊戲
    NoOptionsTxt,
    /// 第一次套用，還沒選要不要備份
    NeedsBackupChoice,
    /// 選了不備份，這次會蓋掉原檔：要玩家確認
    NeedsOverwriteConfirm,
    /// B5c：整份複製來的遊戲資料夾（或原位置連不到）：要先「當成新的模組整合包」才能套用
    ForkNeeded,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub status: ApplyStatus,
    pub backup_dir: String,
    pub backup_created: bool,
    pub backup_reused: bool,
    pub zip_copied: Option<String>,
    pub jars_copied: usize,
    pub quests_copied: bool,
    pub minemenu_copied: bool,
    /// 這次有沒有把遊戲語言改成繁中
    pub lang_set: bool,
    /// 改之前的語言（移除翻譯時會改回來）
    pub original_lang: Option<String>,
    /// 不備份模式下，這次會被覆蓋、且無法還原的原檔（相對遊戲資料夾）
    pub pending_overwrites: Vec<String>,
    /// 來源不明（無法確定原本是不是工具改的）而沒有備份就覆蓋的檔
    pub unknown_files: Vec<String>,
    /// 上次套用後被改過、這次沒有動的檔
    pub skipped_changed: Vec<String>,
    /// 覆蓋前先移入隔離區的檔（來源不明，不會自動還原）
    pub quarantined_files: Vec<String>,
    /// 標記對不上又重建不了、所以這次沒有覆蓋的檔
    pub unconfirmed_files: Vec<String>,
    /// 翻譯之後模組被更新、改名或刪除：舊翻譯沒有放進遊戲（模組已更新，需重新翻譯）
    pub outdated_mods: Vec<String>,
    /// B3：翻譯之後來源被改過的文字檔（任務、語言檔、腳本…）：舊譯文沒有放進遊戲，需重新翻譯
    pub outdated_texts: Vec<String>,
    /// B3 審查 F1：翻譯結果裡不是這一輪產出的檔（舊版產物、產出後被改過）：沒有放進遊戲
    pub stale_outputs: Vec<String>,
    /// B3 審查 F1：上次由工具放進遊戲、這一輪沒有產出的檔：已還原原檔或刪除（前置條件同移除翻譯）
    pub retired_files: Vec<String>,
    /// 上一項中條件不符、沒有動的檔（被改過、無法確認、沒有備份）
    pub retire_skipped: Vec<String>,
    /// 第四輪 A：來源已被整合包移除的譯文（沒有放進遊戲）
    pub source_removed_texts: Vec<String>,
    /// 第四輪 A：讀不到來源、無法確認，未處理（維持遊戲現狀）
    pub unverifiable_texts: Vec<String>,
    pub player_summary: String,
    pub warnings: Vec<String>,
}

impl ApplyResult {
    pub fn is_applied(&self) -> bool {
        self.status == ApplyStatus::Applied
    }

    fn pending(status: ApplyStatus, message: String, pending_overwrites: Vec<String>) -> Self {
        Self {
            status,
            backup_dir: String::new(),
            backup_created: false,
            backup_reused: false,
            zip_copied: None,
            jars_copied: 0,
            quests_copied: false,
            minemenu_copied: false,
            lang_set: false,
            original_lang: None,
            pending_overwrites,
            unknown_files: Vec::new(),
            skipped_changed: Vec::new(),
            quarantined_files: Vec::new(),
            unconfirmed_files: Vec::new(),
            outdated_mods: Vec::new(),
            outdated_texts: Vec::new(),
            stale_outputs: Vec::new(),
            retired_files: Vec::new(),
            retire_skipped: Vec::new(),
            source_removed_texts: Vec::new(),
            unverifiable_texts: Vec::new(),
            player_summary: message,
            warnings: Vec::new(),
        }
    }
}

/// B5c：翻完才套用時，複製資料夾（識別碼相同、原位置還在）或原位置連不到被擋，
/// 回「已翻完、還沒套用」狀態而不是錯誤——翻譯本身沒有失敗（G1.24 的拒絕與零寫入不變：
/// 擋下發生在第一次寫入前的身分檢查）。其他錯誤照舊是錯誤。
pub fn pending_when_copied(result: Result<ApplyResult, String>) -> Result<ApplyResult, String> {
    match result {
        Err(message) if message.contains(super::apply_identity::FORK_HINT) => {
            Ok(ApplyResult::pending(ApplyStatus::ForkNeeded, message, Vec::new()))
        }
        other => other,
    }
}

/// 移除翻譯的結果。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub backup_dir: String,
    pub removed: usize,
    pub restored: usize,
    /// 套用後被玩家或整合包更新改過、所以沒有動的檔
    pub skipped_modified: Vec<String>,
    /// 無法確認是不是工具改的（或標記對不上）、所以沒有動的檔
    pub uncertain: Vec<String>,
    /// 沒有備份、無法還原成原本內容的檔
    pub unrestorable: Vec<String>,
    /// 已從備份放回原檔的檔
    pub restored_files: Vec<String>,
    /// 已刪除的工具新增檔
    pub removed_files: Vec<String>,
    /// 標記有缺但能從另一端補回：已補好標記，這次沒有動檔
    pub repaired: Vec<String>,
    /// 隔離區裡保存的玩家原本版本：「檔案 — 保存位置」
    pub quarantined: Vec<String>,
    pub player_summary: String,
    pub warnings: Vec<String>,
}

/// 移除翻譯：每個檔都先過前置條件檢查（原則 B＋標記互相關聯），不需要備份也能執行。
pub fn restore_last_apply_in(
    instance_path: &Path,
    result_root: Option<&Path>,
) -> Result<RestoreResult, String> {
    // 測試不偵測真的遊戲行程（玩家同時開著別的整合包時會誤判）；測試直接呼叫下面那個
    let running = if cfg!(test) { GameRunning::No } else { game_process::is_game_running(instance_path) };
    restore_last_apply_with_game_state(instance_path, result_root, running)
}

/// 移除翻譯本體。前置條件：遊戲沒開著（會刪、會蓋遊戲資料夾裡的檔）——開著就一個檔都不動。
pub fn restore_last_apply_with_game_state(
    instance_path: &Path,
    result_root: Option<&Path>,
    running: GameRunning,
) -> Result<RestoreResult, String> {
    if let GameRunning::Yes { detail } = &running {
        return Err(format!(
            "Minecraft 正在使用這個模組整合包，現在移除翻譯可能讓檔案被鎖住或只移除一半，所以一個檔都沒有動。\n\
請完全關閉遊戲後再按「移除翻譯」。（{detail}）"
        ));
    }
    let mc = resolve_minecraft_dir(instance_path)?;
    let mut knowledge = Knowledge::load(&mc, result_root)?;
    if !knowledge.record.has_translation() && knowledge.legacy_listed().is_empty() {
        // 只裝過字體包、或只修復過資源包清單：那些不是翻譯，由各自的功能處理
        return Err("這個遊戲資料夾沒有用工具裝過翻譯，沒有翻譯可以移除。".into());
    }
    let ctx = match Ctx::existing(&mc)? {
        Some(ctx) => ctx,
        // 只有舊版清單、還沒有識別碼：認領舊備份前要先有身分
        None => Ctx::begin(&mc)?,
    };
    let outcome = apply_restore::restore_all(&ctx, &mut knowledge, "");
    remember_knowledge(&mut knowledge);
    // 紀錄存不了要回報，不吞掉
    apply_record::save(&mc, &mut knowledge.record)?;
    if !outcome.failures.is_empty() {
        return Err(format!(
            "移除翻譯沒有全部完成（{} 項失敗），請完全關閉遊戲後再按一次「移除翻譯」。\n\
已刪除工具加入的檔案：{} 個；已還原原檔：{} 個\n失敗項目：\n{}",
            outcome.failures.len(),
            outcome.removed_files.len(),
            outcome.restored_files.len(),
            outcome.failures.join("\n")
        ));
    }
    let backup_dir = apply_record::instance_backup_dir(&mc);
    Ok(RestoreResult {
        backup_dir: if long_path(&backup_dir).is_dir() {
            backup_dir.display().to_string()
        } else {
            String::new()
        },
        removed: outcome.removed_files.len(),
        restored: outcome.restored_files.len(),
        player_summary: with_notice(knowledge.notice.as_deref(), apply_restore::describe(&outcome)),
        skipped_modified: outcome.skipped_modified,
        uncertain: outcome.uncertain,
        unrestorable: outcome.unrestorable.iter().chain(outcome.backup_deleted.iter()).cloned().collect(),
        restored_files: outcome.restored_files,
        removed_files: outcome.removed_files,
        repaired: outcome.repaired,
        quarantined: outcome.quarantined,
        warnings: Vec::new(),
    })
}

/// 存紀錄前把「目前知道的」併進去：舊版清單提過的檔（舊備份刪掉後仍記得），
/// 以及這份紀錄開始時就無法判斷的狀態。
/// 會被「這輪沒產出就還原」處理的位置：文字產出（模組檔另有指紋檢查、主資源包每輪重建）。
fn is_retirable_text(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    match parts.first().copied() {
        // resourcepacks/<檔>＝主翻譯資源包與它的標記檔；資料夾型資源包裡的檔才算文字產出
        Some("resourcepacks") => parts.len() > 2,
        Some(
            "config" | "kubejs" | "scripts" | "datapacks" | "defaultconfigs" | "global_packs" | "paxi"
            | "patchouli_books" | "data" | "assets" | "guideme" | "hqm" | "minemenu",
        ) => parts.len() > 1,
        // 模組檔（另有指紋檢查）、options.txt 備份等其他工具檔不在這裡處理
        _ => false,
    }
}

fn remember_knowledge(knowledge: &mut Knowledge) {
    let touched = apply_knowledge::legacy_touched_set(&knowledge.legacy);
    knowledge.record.legacy_touched.extend(touched);
    if knowledge.uncertain.is_some() {
        knowledge.record.started_uncertain = true;
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteBackupResult {
    pub deleted: usize,
    pub failed: Vec<String>,
    /// 無法確認屬於這個整合包、所以沒有刪的備份
    pub kept: Vec<String>,
    pub player_summary: String,
}

/// 刪除這個整合包的全部工具備份。每一份都要先過 require_backup_owned／require_legacy_owned：
/// 標記屬本包的才刪；無法確認的（別的整合包的、沒有清單的舊備份、缺標記的）一律不刪並列出。
/// 刪舊版備份前先把舊版清單提過的檔記進套用紀錄，之後才不會把舊版翻譯當成原檔備份。
pub fn delete_apply_backups_in(
    instance_path: &Path,
    result_root: Option<&Path>,
) -> Result<DeleteBackupResult, String> {
    let mc = resolve_minecraft_dir(instance_path)?;
    let mut knowledge = Knowledge::load(&mc, result_root)?;
    let mut ctx = Ctx::existing(&mc)?;
    if !knowledge.legacy.is_empty() {
        // 前置條件：存紀錄前先確認（或建立）整合包識別碼，紀錄才會存到正確的位置
        if ctx.is_none() {
            ctx = Some(Ctx::begin(&mc)?);
            knowledge = Knowledge::load(&mc, result_root)?;
        }
        remember_knowledge(&mut knowledge);
        apply_record::save(&mc, &mut knowledge.record)?;
    }
    let mut deleted = 0usize;
    let mut failed = Vec::new();
    let mut kept = Vec::new();

    if let Some(ctx) = &ctx {
        let rels = apply_guard::backup_rels(&mc);
        for rel in rels {
            if !apply_guard::require_backup_owned(ctx, &rel) {
                kept.push(format!("備份區的 {rel}（沒有屬於這個整合包的標記）"));
                continue;
            }
            match apply_guard::delete_backup(&mc, &rel) {
                Ok(()) => deleted += 1,
                Err(error) => failed.push(error),
            }
        }
        apply_guard::remove_empty_backup_dirs(&mc);
        if deleted > 0 {
            // 記下來：之後「移除翻譯」要照實說被覆蓋的檔已無法還原
            knowledge.record.backups_deleted = true;
            apply_record::save(&mc, &mut knowledge.record)?;
        }
    }
    let legacy_count = knowledge.legacy.len();
    for legacy in &knowledge.legacy {
        match fs::remove_dir_all(long_path(&legacy.dir)) {
            Ok(()) => deleted += 1,
            Err(error) => failed.push(format!("{}：{}", legacy.dir.display(), error)),
        }
    }
    for legacy in &knowledge.unowned_legacy {
        kept.push(format!("{}（無法確認是不是這個整合包的舊版備份）", legacy.dir.display()));
    }

    let legacy_note = if legacy_count > 0 {
        format!("（其中 {legacy_count} 個是舊版工具的備份：之後將無法還原舊版工具改過的檔案）")
    } else {
        String::new()
    };
    let mut player_summary = if failed.is_empty() {
        if deleted == 0 {
            "沒有找到可以刪除的工具備份。".to_string()
        } else {
            format!("已刪除 {deleted} 個備份檔{legacy_note}。")
        }
    } else {
        format!("已刪除 {} 個備份檔{legacy_note}，但有 {} 個無法刪除。", deleted, failed.len())
    };
    if !kept.is_empty() {
        player_summary.push_str(&format!(
            "\n以下 {} 項無法確認屬於這個整合包，沒有刪：\n  - {}",
            kept.len(),
            kept.join("\n  - ")
        ));
    }
    Ok(DeleteBackupResult { deleted, failed, kept, player_summary })
}

/// 這個遊戲資料夾有沒有「可以移除的翻譯」：有套用紀錄、舊版清單，或看得到工具的備份。
/// 只讀取，不會修改檔案；供 UI 決定是否顯示「移除翻譯」與「刪除備份」。
/// `.mcpl` 被刪時只判斷認不認得回來，不補回、不寫紀錄——真正的認回留給套用／移除，說明才會出現在那次的結果裡。
/// 紀錄讀不出來時也回 true：讓玩家按下去看到原因與「重設套用紀錄」。
pub fn has_apply_backups_in(
    instance_path: &Path,
    result_root: Option<&Path>,
) -> Result<bool, String> {
    let mc = resolve_minecraft_dir(instance_path)?;
    let Ok(knowledge) = Knowledge::peek(&mc, result_root) else {
        return Ok(true);
    };
    // 只算翻譯：只裝過字體包或只修復過清單時，不顯示「移除翻譯」
    Ok(knowledge.record.has_translation()
        || !knowledge.legacy_listed().is_empty()
        || !knowledge.legacy.is_empty()
        || !knowledge.unowned_legacy.is_empty()
        || !apply_guard::backup_rels(&mc).is_empty())
}

/// 一鍵套用。**這是唯一的寫入入口**，遊戲關閉檢查在這裡做（P0-02）。
///
/// 為什麼不能只靠前端：one_click／補翻／修復／單獨套用最後全都走到這個函式，
/// 前端檢查與實際寫入之間還有空窗，遊戲可能在那段時間被開起來。
pub fn apply_to_instance(
    instance_path: &Path,
    output_or_work: &Path,
    pack_name_hint: Option<&str>,
    policy: BackupPolicy,
) -> Result<ApplyResult, String> {
    let running = game_process::is_game_running(instance_path);
    apply_to_instance_with_game_state(instance_path, output_or_work, pack_name_hint, policy, running)
}

/// 一個即將寫入的項目，以及寫入前判斷出來的狀態。
struct Decision {
    index: usize,
    rel: String,
    class: Class,
    planned_sha: String,
    existed: bool,
}

/// 套用本體。「偵測遊戲行程」留在外層，這裡只依傳入的判定行事——測試才能覆蓋
/// Yes／No／Unknown 而不必真的啟動 Minecraft。
///
/// 順序（每一步都是下一步的前置條件）：
/// 1. 會讓套用停下來的檢查（遊戲開著、沒有 options.txt、還沒選備份、不備份時要蓋掉原檔）→ 零寫入回狀態。
/// 2. 逐檔判斷原本是什麼；原檔先建立有效備份、來源不明的先移入隔離區、舊版備份先認領。
/// 3. 先存紀錄與全部標記（含資源包清單與語言設定的標記）。
/// 4. 才寫遊戲檔與設定檔。
pub fn apply_to_instance_with_game_state(
    instance_path: &Path,
    output_or_work: &Path,
    pack_name_hint: Option<&str>,
    policy: BackupPolicy,
    running: GameRunning,
) -> Result<ApplyResult, String> {
    cancel::check()?;
    if !instance_path.exists() {
        return Err("找不到遊戲資料夾，請重新選擇。".into());
    }
    let mc = resolve_minecraft_dir(instance_path)?;
    crate::dev_log!(
        "apply",
        "開始套用 instance={} 來源={} 資源包提示={:?} 備份={:?}",
        instance_path.display(),
        output_or_work.display(),
        pack_name_hint,
        policy
    );
    let layout = ensure_result_layout(output_or_work)?;
    let work = &layout.work_root;
    let pack_name = resolve_pack_name(work, pack_name_hint);
    let zip_src = find_zip_in_layout(&layout, &pack_name);
    // 指紋標記：供下次「開始翻譯」判斷這個 zip 是不是這個整合包產生的
    let zip_meta = zip_src.as_ref().map(|_| {
        let meta = serde_json::json!({ "modsFingerprint": super::session::mods_fingerprint(instance_path) });
        serde_json::to_string_pretty(&meta).unwrap_or_default().into_bytes()
    });
    let mut plan = apply_plan::build_plan(&mc, &layout, &pack_name, zip_src.as_deref(), zip_meta);
    if plan.is_empty() {
        return Err(format!(
            "在「{}」找不到可套用的翻譯內容。請先完成一鍵翻譯。",
            work.display()
        ));
    }

    // ── 1. 會讓套用停下來的檢查（任何一項不過就一個檔都不動）──
    if let GameRunning::Yes { detail } = &running {
        crate::dev_log!("apply", "遊戲正在執行，延後套用 instance={}", instance_path.display());
        return Ok(ApplyResult::pending(
            ApplyStatus::GameRunning,
            format!(
                "翻譯已完成，但 Minecraft 正在使用這個模組整合包，所以還沒裝進遊戲。\n\
現在寫入可能讓檔案被鎖住、只裝一半。請完全關閉遊戲（含啟動器裡載入中的遊戲）後，按「套用到遊戲」。\n\
（{detail}）"
            ),
            Vec::new(),
        ));
    }
    let options_path = mc.join("options.txt");
    if !long_path(&options_path).is_file() {
        return Ok(ApplyResult::pending(
            ApplyStatus::NoOptionsTxt,
            "翻譯已完成，但這個遊戲資料夾還沒有啟動過遊戲，所以還沒裝進遊戲。\n\
請先用啟動器開一次遊戲，看到標題畫面後關掉，再按「套用到遊戲」。不用重新翻譯。"
                .into(),
            Vec::new(),
        ));
    }
    if policy == BackupPolicy::Ask {
        return Ok(ApplyResult::pending(
            ApplyStatus::NeedsBackupChoice,
            "翻譯已完成。第一次把翻譯裝進這個遊戲前，請先決定要不要備份會被覆蓋的檔案。".into(),
            Vec::new(),
        ));
    }
    // 讀紀錄前先確認遊戲資料夾有識別碼（紀錄、備份、隔離區都以它為鍵）
    let identity_known = super::mcpl_marker::read_instance(&mc)?.is_some();
    // 先只判斷、不寫（還沒有識別碼時用舊鍵，或用認得回來的紀錄）：讀不到、或下面要先問玩家時都是零寫入
    let mut knowledge = Knowledge::peek(&mc, Some(work))?;

    // 前置條件：翻譯後的模組檔只放在「翻譯時用的同一個模組」上
    let outdated_mods = super::jar_sources::drop_outdated(work, &mc, &mut plan, &knowledge.record);
    // B3：整檔替換／新增的文字檔同樣只放在「翻譯時用的同一份來源」上
    // 審查 F-b：stale、outdated 只列出、維持遊戲現狀（它們不在退休名單上，退休只收
    // text_sources 確認「產出者跑完、來源已不在」的檔）
    // 審查 F1：只放「確認過的產出、內容未動、來源未變」的文字檔
    let mut dropped_texts = super::text_sources::drop_unconfirmed(work, &mc, &mut plan, &knowledge.record);
    // B6a-2：主資源包裡的書頁同樣只放「來源還是翻譯當時那份」的（拿掉的列入 outdatedTexts，原 zip 不動）
    let _filtered_copy = super::pack_books::filter_main_pack(work, &mc, &mut plan, &knowledge.record, &mut dropped_texts)?;

    // ── 2a. 逐檔判斷「原本是什麼」──
    let mut decisions = Vec::new();
    for (index, item) in plan.items.iter().enumerate() {
        let planned_sha = match &item.source {
            ItemSource::File(src) => apply_record::file_sha256(src)
                .ok_or_else(|| format!("讀不到翻譯結果裡的檔案：{}", src.display()))?,
            ItemSource::Bytes(bytes) => sha256_hex(bytes),
        };
        let rel = apply_record::rel_key(&mc, &item.dest);
        let existed = long_path(&item.dest).is_file();
        let class = knowledge.classify(&mc, &item.dest, &planned_sha);
        // 看起來像工具產物、但沒有工具標記或舊版清單佐證的檔：當來源不明（原則 A），先隔離再覆蓋
        decisions.push(Decision { index, rel, class, planned_sha, existed });
    }

    if policy == (BackupPolicy::NoBackup { overwrite_confirmed: false }) {
        let unprotected: Vec<String> = decisions
            .iter()
            .filter(|d| d.class == Class::Original || super::apply_pending::overwrites_unprotected(&d.class))
            .map(|d| d.rel.clone())
            .collect();
        if !unprotected.is_empty() {
            return Ok(ApplyResult::pending(
                ApplyStatus::NeedsOverwriteConfirm,
                format!(
                    "翻譯已完成。這次會覆蓋遊戲裡 {} 個原本的檔案，你選了不備份，覆蓋後就無法還原這些檔案。",
                    unprotected.len()
                ),
                unprotected,
            ));
        }
    }

    let mut warnings = Vec::new();
    if matches!(running, GameRunning::Unknown) {
        // 失效方向＝放行（查不到不能變成新的卡關），但不得偽裝成已確認關閉。
        warnings.push(
            "這次無法確認 Minecraft 是否已關閉（偵測不可用），已照常套用。若遊戲內出現殘缺翻譯，請關閉遊戲後再套用一次。"
                .into(),
        );
    }

    // ── 2b. 前置條件：確定身分後，原檔先備份、來源不明先隔離、舊版備份先認領 ──
    // 真的要動檔了：`.mcpl` 被刪而認得回來時，先認回原本的識別碼，才確認或建立識別碼
    let reclaim_notice = if identity_known { None } else { super::apply_identity::require_identity(&mc)? };
    let ctx = Ctx::begin(&mc)?;
    if !identity_known {
        // 第一次建立識別碼時舊鍵資料可能剛搬到新鍵：重新讀一次（認回的說明留著）
        let notice = knowledge.notice.take();
        knowledge = Knowledge::load(&mc, Some(work))?;
        knowledge.notice = knowledge.notice.take().or(reclaim_notice).or(notice);
    }
    let backup_dir = apply_record::instance_backup_dir(&mc);
    let backup_existed = long_path(&backup_dir).is_dir();
    let mut backed_up = 0usize;
    let mut quarantined_files = Vec::new();
    let mut markers: Vec<super::mcpl_marker::FileMarker> = Vec::new();
    let mut unconfirmed: Vec<String> = Vec::new();
    for decision in &decisions {
        cancel::check()?;
        let item = &plan.items[decision.index];
        let previous = knowledge.record.files.get(&decision.rel).cloned();
        let marker_id = previous
            .as_ref()
            .map(|p| p.marker_id.clone())
            .filter(|id| !id.is_empty())
            .unwrap_or_else(super::mcpl_marker::new_marker_id);
        let original_sha = if decision.existed && decision.class != Class::ToolVersion {
            apply_record::file_sha256(&item.dest).unwrap_or_default()
        } else {
            String::new()
        };
        let mut backup_id = String::new();
        let mut quarantine_id = String::new();
        let (kind, origin) = match &decision.class {
            Class::Absent => (FileKind::Added, Origin::Known),
            Class::ToolVersion => {
                // 前置條件：覆蓋工具版本前，遊戲檔標記要完整（缺了從紀錄重建；重建不了就不動）
                if let apply_guard::Check::Broken(why) =
                    apply_guard::require_game_marker(&ctx, &knowledge.record, &decision.rel)
                {
                    crate::dev_log!("apply", "{}：{why}", decision.rel);
                    unconfirmed.push(decision.rel.clone());
                    continue;
                }
                let p = previous.as_ref().expect("工具版本一定在紀錄上");
                backup_id = p.backup_id.clone();
                quarantine_id = p.quarantine_id.clone();
                (p.kind, p.origin)
            }
            Class::Original => {
                if policy == BackupPolicy::Backup {
                    backup_id = apply_guard::require_original_backup(&ctx, &item.dest, &decision.rel, &marker_id)?;
                    backed_up += 1;
                }
                (FileKind::Overwritten, Origin::Known)
            }
            Class::LegacyBacked { original, manifest_dir } => {
                quarantine_id = apply_guard::require_quarantined(&ctx, &item.dest, &decision.rel, &marker_id)?;
                quarantined_files.push(decision.rel.clone());
                backup_id = apply_guard::require_legacy_claimed(&ctx, &decision.rel, original, manifest_dir, &marker_id)?;
                (FileKind::Overwritten, Origin::Known)
            }
            Class::LegacyAdded => {
                quarantine_id = apply_guard::require_quarantined(&ctx, &item.dest, &decision.rel, &marker_id)?;
                quarantined_files.push(decision.rel.clone());
                (FileKind::Added, Origin::Known)
            }
            Class::Unknown => {
                quarantine_id = apply_guard::require_quarantined(&ctx, &item.dest, &decision.rel, &marker_id)?;
                quarantined_files.push(decision.rel.clone());
                (FileKind::Overwritten, Origin::Unknown)
            }
            Class::Unwritten(previous) => {
                // 上次中途中斷、還沒寫入：沿用上次的分類與連結；連結另一端壞了就不覆蓋
                if let Err(why) = super::apply_pending::require_links_intact(&ctx, &decision.rel, previous) {
                    crate::dev_log!("apply", "{}：{why}", decision.rel);
                    unconfirmed.push(decision.rel.clone());
                    continue;
                }
                backup_id = previous.backup_id.clone();
                quarantine_id = previous.quarantine_id.clone();
                if !quarantine_id.is_empty() {
                    quarantined_files.push(decision.rel.clone());
                }
                (previous.kind, previous.origin)
            }
        };
        knowledge.record.unconfirmed.remove(&decision.rel);
        knowledge
            .record
            .set_entry(&decision.rel, kind, origin, None, decision.planned_sha.clone());
        if let Some(entry) = knowledge.record.files.get_mut(&decision.rel) {
            entry.marker_id = marker_id.clone();
            entry.backup_id = backup_id.clone();
            entry.quarantine_id = quarantine_id.clone();
        }
        let original_for_marker = match (&decision.class, previous.as_ref()) {
            (Class::ToolVersion, _) => super::mcpl_marker::read_file_marker(&mc, &decision.rel)
                .map(|m| m.original_sha256)
                .unwrap_or_default(),
            (Class::LegacyBacked { original, .. }, _) => apply_record::file_sha256(original).unwrap_or_default(),
            (Class::Unwritten(previous), _) => previous.original_sha.clone(),
            _ => original_sha,
        };
        markers.push(game_marker(
            &ctx,
            &marker_id,
            &decision.rel,
            kind,
            origin,
            &decision.planned_sha,
            &original_for_marker,
            &backup_id,
            &quarantine_id,
        ));
    }

    // ── 2c. 設定檔：先算好要怎麼改，並準備清單項目與語言設定的標記 ──
    let options_plan = plan_options(&ctx, &mc, plan.zip_name.as_deref(), &mut knowledge.record, &mut markers)?;

    // ── 3. 先存紀錄與全部標記 ──
    remember_knowledge(&mut knowledge);
    let extra_ids = options_plan.marker_ids();
    apply_guard::require_record_and_markers(
        &ctx,
        &mut knowledge.record,
        &markers,
        Some(&options_plan.marker),
        &extra_ids,
    )?;

    // ── 4. 才寫遊戲檔與設定檔 ──
    // 每寫完一個檔才把它的標記標為「已寫入」；沒寫完的維持「待確認」
    for decision in decisions.iter().filter(|d| !unconfirmed.contains(&d.rel)) {
        cancel::check()?;
        write_item(&plan.items[decision.index])?;
        super::mcpl_marker::mark_written(&mc, &decision.rel)?;
    }
    let (lang_set, original_lang) = write_options(&mc, &options_plan, plan.zip_name.as_deref())?;

    // 審查 F-a：退休（把遊戲裡上一版譯文拿掉）必須先確認「確定不再產出」：
    // 產出者本輪完整跑完、而且來源檔已不在遊戲（text_sources 的退休名單）。
    // 再走「移除翻譯」同一套前置條件：內容仍是工具版本、備份屬本包才還原／刪除。
    let mut retired_files = Vec::new();
    let mut retire_skipped = Vec::new();
    {
        // 這次要放進遊戲的檔絕不退休
        let placing: std::collections::HashSet<String> =
            plan.items.iter().map(|item| apply_record::rel_key(&mc, &item.dest)).collect();
        // 套用當下再確認：產出清單屬於這個遊戲資料夾、來源現在仍確定不在（審查第三輪 2）
        let candidates = super::text_sources::confirm_retire(work, &mc);
        let retire: std::collections::HashSet<String> = knowledge
            .record
            .files
            .iter()
            .filter(|(rel, entry)| {
                entry.owner.is_empty()
                    && candidates.contains(*rel)
                    && !placing.contains(*rel)
                    && is_retirable_text(rel)
            })
            .map(|(rel, _)| rel.clone())
            .collect();
        if !retire.is_empty() {
            let outcome = apply_restore::restore_only(&ctx, &mut knowledge, "", &retire);
            apply_record::save(&mc, &mut knowledge.record)?;
            retired_files.extend(outcome.restored_files);
            retired_files.extend(outcome.removed_files);
            retire_skipped.extend(outcome.skipped_modified);
            retire_skipped.extend(outcome.uncertain);
            retire_skipped.extend(outcome.unrestorable);
            retire_skipped.extend(outcome.backup_deleted);
            warnings.extend(outcome.failures);
        }
    }

    if let Some(name) = plan.zip_name.as_deref() {
        warnings.extend(warn_enabled_packs_covering_font(&mc, name));
        warnings.extend(collect_post_apply_warnings(&mc, work, Some(name)));
    }

    let backup_now = long_path(&backup_dir).is_dir();
    let result = ApplyResult {
        status: ApplyStatus::Applied,
        backup_dir: if backup_now {
            backup_dir.display().to_string()
        } else {
            String::new()
        },
        backup_created: policy == BackupPolicy::Backup && !backup_existed && backed_up > 0,
        backup_reused: policy == BackupPolicy::Backup && backup_existed,
        zip_copied: plan
            .zip_name
            .as_ref()
            .map(|name| mc.join("resourcepacks").join(name).display().to_string()),
        jars_copied: plan.count(Group::Mods),
        quests_copied: plan
            .items
            .iter()
            .any(|item| item.dest.starts_with(mc.join("config").join("ftbquests"))),
        minemenu_copied: plan.has(Group::Minemenu),
        lang_set,
        original_lang,
        pending_overwrites: Vec::new(),
        unknown_files: quarantined_files.clone(),
        skipped_changed: Vec::new(),
        quarantined_files,
        unconfirmed_files: unconfirmed,
        outdated_mods,
        outdated_texts: dropped_texts.outdated,
        stale_outputs: dropped_texts.stale,
        retired_files,
        retire_skipped,
        source_removed_texts: dropped_texts.source_removed,
        unverifiable_texts: dropped_texts.unconfirmed,
        player_summary: String::new(),
        warnings,
    };
    let player_summary =
        with_notice(knowledge.notice.as_deref(), describe_applied(&plan, &result, policy, knowledge.uncertain.as_deref()));
    Ok(ApplyResult { player_summary, ..result })
}

#[allow(clippy::too_many_arguments)]
fn game_marker(
    ctx: &Ctx,
    id: &str,
    rel: &str,
    kind: FileKind,
    origin: Origin,
    tool_sha: &str,
    original_sha: &str,
    backup_id: &str,
    quarantine_id: &str,
) -> super::mcpl_marker::FileMarker {
    use super::mcpl_marker as mk;
    let mut links = vec![mk::link(&ctx.batch_id, mk::REL_BATCH)];
    if !backup_id.is_empty() {
        links.push(mk::link(backup_id, mk::REL_BACKUP));
    }
    if !quarantine_id.is_empty() {
        links.push(mk::link(quarantine_id, mk::REL_QUARANTINE));
    }
    let history = ctx_history(ctx, rel);
    mk::FileMarker {
        tool_history: history,
        state: "pending".into(),
        id: id.to_string(),
        instance_id: ctx.instance_id.clone(),
        rel: rel.to_string(),
        role: if kind == FileKind::Added { "added".into() } else { "overwritten".into() },
        tool_sha256: tool_sha.to_string(),
        original_sha256: original_sha.to_string(),
        backup: if backup_id.is_empty() {
            String::new()
        } else {
            apply_record::instance_backup_dir(&ctx.mc).join(rel).display().to_string()
        },
        tool_version: env!("CARGO_PKG_VERSION").into(),
        written_at: mk::now_secs(),
        origin: if origin == Origin::Unknown { "unknown".into() } else { "known".into() },
        links,
    }
}

/// 這個檔以前工具寫過的所有版本指紋（紀錄的歷史＋舊標記的歷史），寫進新標記。
fn ctx_history(ctx: &Ctx, rel: &str) -> Vec<String> {
    let mut history: Vec<String> = apply_record::load(&ctx.mc)
        .ok()
        .and_then(|r| r.files.get(rel).map(|e| {
            let mut h = e.history.clone();
            h.push(e.sha256.clone());
            h
        }))
        .unwrap_or_default();
    if let Some(old) = super::mcpl_marker::read_file_marker(&ctx.mc, rel) {
        history.extend(old.tool_history);
        history.push(old.tool_sha256);
    }
    history.sort();
    history.dedup();
    history
}

fn write_item(item: &apply_plan::PlanItem) -> Result<(), String> {
    let dest = &item.dest;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(long_path(parent)).map_err(|e| e.to_string())?;
    }
    match &item.source {
        ItemSource::File(src) => apply_record::copy_atomic(src, dest).map_err(|e| {
            format!(
                "套用複製失敗（請關遊戲後重試）{} → {}：{e}",
                src.display(),
                dest.display()
            )
        }),
        ItemSource::Bytes(bytes) => apply_record::write_atomic(dest, bytes)
            .map_err(|e| format!("寫入 {} 失敗：{e}", dest.display())),
    }
}

fn describe_applied(
    plan: &apply_plan::ApplyPlan,
    result: &ApplyResult,
    policy: BackupPolicy,
    uncertain: Option<&str>,
) -> String {
    let mut lines = vec!["已把翻譯套用到遊戲，直接開遊戲就是繁體中文。".to_string()];
    if let Some(zip) = &plan.zip_name {
        lines.push(format!("• 翻譯資源包「{zip}」：已啟用並排在最高優先"));
    }
    if result.lang_set {
        lines.push(format!(
            "• 遊戲語言：已設成繁體中文（台灣）（原本是 {}，移除翻譯時會改回來）",
            language_display_name(result.original_lang.as_deref())
        ));
    } else {
        lines.push("• 遊戲語言：本來就是繁體中文（台灣）".into());
    }
    if result.jars_copied > 0 {
        lines.push(format!("• 翻譯後的模組檔：{} 個", result.jars_copied));
    }
    if !result.outdated_mods.is_empty() {
        lines.push(format!(
            "• 模組已更新，需重新翻譯：{} 個模組在翻譯之後被更新、改名或刪除，舊的翻譯沒有放進遊戲，\
原本的模組檔沒有動（{}）",
            result.outdated_mods.len(),
            preview(&result.outdated_mods)
        ));
    }
    if !result.outdated_texts.is_empty() {
        lines.push(format!(
            "• 模組整合包已更新，需重新翻譯：{} 個文字檔的來源在翻譯之後被改過，舊的翻譯沒有放進遊戲（{}）",
            result.outdated_texts.len(),
            preview(&result.outdated_texts)
        ));
    }
    if !result.retired_files.is_empty() {
        lines.push(format!(
            "• 模組整合包已移除原文、不再需要的舊翻譯已從遊戲拿掉（還原原檔或刪除工具加的檔）：{} 個（{}）",
            result.retired_files.len(),
            preview(&result.retired_files)
        ));
    }
    if !result.source_removed_texts.is_empty() {
        lines.push(format!(
            "• 有 {} 個譯文的英文原文已被模組整合包移除，沒有放進遊戲（{}）",
            result.source_removed_texts.len(),
            preview(&result.source_removed_texts)
        ));
    }
    if !result.unverifiable_texts.is_empty() {
        lines.push(format!(
            "• 有 {} 個譯文讀不到它的英文原文（可能是網路磁碟暫時讀不到），無法確認，這次沒有處理，遊戲裡保持原樣（{}）",
            result.unverifiable_texts.len(),
            preview(&result.unverifiable_texts)
        ));
    }
    if !result.retire_skipped.is_empty() {
        lines.push(format!(
            "• 舊翻譯中有 {} 個檔無法確認或被改過，保持原樣沒有動（{}）",
            result.retire_skipped.len(),
            preview(&result.retire_skipped)
        ));
    }
    if !result.stale_outputs.is_empty() {
        lines.push(format!(
            "• 翻譯結果裡有 {} 個檔沒有放進遊戲：它們是舊版工具留下的產物，或產出之後又被改過，無法確認是這次翻譯的結果。遊戲裡那份沒有動——如果它是舊版工具寫的中文，要回到英文請按「移除翻譯」（{}）",
            result.stale_outputs.len(),
            preview(&result.stale_outputs)
        ));
    }
    let text_files = plan.items.len()
        - plan.count(Group::Zip)
        - plan.count(Group::ZipMeta)
        - plan.count(Group::Mods);
    if text_files > 0 {
        lines.push(format!("• 任務、手冊與其他文字檔：{text_files} 個"));
    }
    lines.push(match policy {
        BackupPolicy::Backup if result.backup_reused => {
            "• 備份：已確認這個模組整合包目前的原檔都有有效備份".to_string()
        }
        BackupPolicy::Backup if result.backup_created => "• 備份：已備份會被覆蓋的原檔".to_string(),
        BackupPolicy::Backup => "• 備份：這次沒有需要備份的原檔".to_string(),
        _ => "• 備份：依你的選擇沒有備份".to_string(),
    });
    if let Some(reason) = uncertain {
        lines.push(format!(
            "\n{reason}，工具無法確定遊戲裡哪些檔案是原本的、哪些是以前翻譯過的，\
所以這些檔案沒有當成原檔備份，而是先移到隔離區再換成新的翻譯。"
        ));
    }
    if !result.quarantined_files.is_empty() {
        lines.push(format!(
            "\n以下 {} 個檔案無法確定是不是工具以前改的，覆蓋前已先移到隔離區保存（在工具資料夾裡，不會自動放回）：\n{}",
            result.quarantined_files.len(),
            preview(&result.quarantined_files)
        ));
    }
    lines.push(
        "\n想回到原版：按「移除翻譯」（工具加的檔案會刪掉、設定會改回來）。\n\
萬一遊戲開不起來，多半是模組整合包本身缺模組；可以先按「移除翻譯」再開一次，排除是不是翻譯造成的。"
            .into(),
    );
    let mut summary = lines.join("\n");
    if !result.warnings.is_empty() {
        summary.push_str(&format!("\n\n注意：\n• {}", result.warnings.join("\n• ")));
    }
    summary
}

/// 識別碼認回時的說明（例如部分檔案被整合包更新改過）放在結果最前面。
fn with_notice(notice: Option<&str>, summary: String) -> String {
    match notice {
        Some(notice) => format!("{notice}\n\n{summary}"),
        None => summary,
    }
}

fn preview(items: &[String]) -> String {
    let mut shown: Vec<String> =
        items.iter().take(10).map(|s| format!("  - {}", apply_restore::display_rel(s))).collect();
    if items.len() > 10 {
        shown.push(format!("  …另有 {} 個", items.len() - 10));
    }
    shown.join("\n")
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
                "遊戲的資源包資料夾裡有 {tool_count} 個本工具做的翻譯資源包（名稱開頭是「模組包翻譯工具+」）；請在遊戲的資源包畫面只啟用最新一個，停用舊的以免混亂。"
            ));
        }
    }
    if let Some(zip_name) = applied_zip_name {
        if !options_lists_resource_pack(mc, zip_name) {
            out.push(format!(
                "遊戲設定檔裡似乎沒有啟用這次的「{zip_name}」；若進遊戲後沒看到繁中，請在遊戲的資源包畫面手動啟用。"
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

/// 設定檔要怎麼改（寫入前算好，連同清單項目與語言設定的標記一起先存）。
struct OptionsPlan {
    original: String,
    edit: options_txt::OptionsEdit,
    entry: Option<String>,
    marker: super::mcpl_marker::OptionsMarker,
    ids: Vec<String>,
}

impl OptionsPlan {
    fn marker_ids(&self) -> Vec<String> {
        self.ids.clone()
    }
}

/// 準備設定檔的修改：資源包清單項目標記 ↔ 資源包檔標記互指、語言設定標記記下原語言、
/// 設定檔旁的另存檔也有自己的遊戲檔標記。全部只改記憶體，由呼叫端一起先存。
fn plan_options(
    ctx: &Ctx,
    mc: &Path,
    zip_name: Option<&str>,
    record: &mut ApplyRecord,
    markers: &mut Vec<super::mcpl_marker::FileMarker>,
) -> Result<OptionsPlan, String> {
    use super::mcpl_marker as mk;
    let options = mc.join("options.txt");
    let original = fs::read_to_string(long_path(&options)).map_err(|e| format!("讀取遊戲設定檔失敗：{e}"))?;
    let entry = zip_name.map(options_txt::pack_entry);
    let edit = options_txt::enable_pack_last(&original, entry.as_deref(), true);
    let mut marker = mk::read_options_marker(mc);
    let mut ids = Vec::new();

    if let (Some(entry), Some(zip)) = (entry.as_deref(), zip_name) {
        // 舊版工具啟用過的翻譯資源包也算工具加的（移除翻譯時要從清單拿掉）
        let tool_named = is_tool_resource_pack(zip.trim_end_matches(".zip"));
        if (edit.pack_added || tool_named) && !record.options.packs_added.iter().any(|e| e == entry) {
            record.options.packs_added.push(entry.to_string());
        }
        if record.options.packs_added.iter().any(|e| e == entry) {
            let zip_rel = format!("resourcepacks/{zip}");
            if let Some(zip_marker) = markers.iter_mut().find(|m| m.rel == zip_rel) {
                let entry_id = marker
                    .entries
                    .iter()
                    .find(|e| e.entry == entry)
                    .map(|e| e.id.clone())
                    .unwrap_or_else(mk::new_marker_id);
                marker.entries.retain(|e| e.entry != entry);
                marker.entries.push(mk::EntryMarker {
                    id: entry_id.clone(),
                    entry: entry.to_string(),
                    links: vec![mk::link(&zip_marker.id, mk::REL_GAME_FILE), mk::link(&ctx.batch_id, mk::REL_BATCH)],
                });
                mk::set_link(&mut zip_marker.links, mk::REL_PACK_ENTRY, &entry_id);
                ids.push(entry_id);
            }
        }
    }
    if edit.lang_changed && !record.options.lang_changed {
        record.options.lang_changed = true;
        record.options.original_lang = edit.original_lang.clone();
    }
    if record.options.lang_changed {
        let id = marker.lang.as_ref().map(|l| l.id.clone()).unwrap_or_else(mk::new_marker_id);
        marker.lang = Some(mk::LangMarker {
            id: id.clone(),
            original_lang: record.options.original_lang.clone(),
            links: vec![mk::link(&ctx.batch_id, mk::REL_BATCH)],
        });
        record.options.lang_marker_id = id.clone();
        record.options.lang_batch = ctx.batch_id.clone();
        ids.push(id);
    }
    // 設定檔旁的另存檔：只在第一次建立，並先有自己的遊戲檔標記
    markers.extend(super::pack_repair::options_bak_marker(ctx, record, &original, ""));
    Ok(OptionsPlan { original, edit, entry, marker, ids })
}

/// 紀錄與標記都存好之後才寫設定檔。寫完重讀驗證：原本清單的每一項都還在、翻譯包排最後、
/// 語言是繁中；不對就把原檔寫回去並回報——寧可不啟用翻譯包，也不能讓遊戲開不起來。
fn write_options(mc: &Path, plan: &OptionsPlan, zip_name: Option<&str>) -> Result<(bool, Option<String>), String> {
    let options = mc.join("options.txt");
    // 只在第一次建立（已存在就不覆寫），這樣它的內容永遠是工具第一次動之前的樣子
    options_txt::backup_beside_once(&options, &plan.original)
        .map_err(|e| format!("另存遊戲設定檔失敗，已停止：{e}"))?;
    if plan.edit.text != plan.original {
        apply_record::write_atomic(&options, plan.edit.text.as_bytes())
            .map_err(|e| format!("寫入遊戲設定檔失敗：{e}"))?;
    }
    let after = fs::read_to_string(long_path(&options)).unwrap_or_default();
    if let Err(problem) = options_txt::verify_enabled(&plan.original, &after, plan.entry.as_deref(), true) {
        let _ = apply_record::write_atomic(&options, plan.original.as_bytes());
        return Err(format!(
            "設定翻譯資源包時發現問題（{problem}），已把遊戲設定還原成原本的樣子。\n\
請在遊戲的資源包畫面手動啟用「{}」，語言選繁體中文（台灣）。",
            zip_name.unwrap_or("翻譯資源包")
        ));
    }
    Ok((plan.edit.lang_changed, plan.edit.original_lang.clone()))
}

/// 已啟用且含 `assets/*/font/` 的資源包可能蓋掉翻譯／自訂字體 → 警告（不做 codec 重寫）。
fn warn_enabled_packs_covering_font(mc: &Path, our_zip_name: &str) -> Vec<String> {
    let suspects = enabled_packs_covering_font(mc, &|name| name == our_zip_name);
    if suspects.is_empty() {
        return Vec::new();
    }
    vec![format!(
        "以下已啟用資源包含 font/，可能蓋過翻譯或自訂字體顯示：{}。請在資源包選單把「繁中翻譯／字體包」置頂，或暫時停用上述包後重開遊戲。",
        suspects.join("、")
    )]
}

/// 已啟用（options.txt resourcePacks）且含 `font/` 的資源包名稱；`skip` 為真的略過。
/// B2：output_guard 的「字體可能不支援中文」共用這一份判斷。
pub(crate) fn enabled_packs_covering_font(mc: &Path, skip: &dyn Fn(&str) -> bool) -> Vec<String> {
    let options = mc.join("options.txt");
    let Ok(text) = fs::read_to_string(&options) else {
        return Vec::new();
    };
    let Some(list_line) = text.lines().find(|l| l.starts_with("resourcePacks:")) else {
        return Vec::new();
    };
    let value = list_line.strip_prefix("resourcePacks:").unwrap_or("").trim();
    let mut suspects = Vec::new();
    for raw in value.split('"') {
        let entry = raw.trim();
        if entry.is_empty() || entry == "vanilla" || entry == "," || entry == "[" || entry == "]" {
            continue;
        }
        let Some(name) = entry.strip_prefix("file/") else {
            continue;
        };
        if !skip(name) && pack_contains_font_override(mc, name) {
            suspects.push(name.to_string());
        }
    }
    suspects
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
#[path = "apply_instance_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "apply_instance_safety_tests.rs"]
mod safety_tests;

#[cfg(test)]
#[path = "apply_instance_review_tests.rs"]
mod review_tests;

#[cfg(test)]
#[path = "apply_instance_round5_tests.rs"]
mod round5_tests;

#[cfg(test)]
#[path = "apply_instance_round5b_tests.rs"]
mod round5b_tests;

#[cfg(test)]
#[path = "apply_instance_final_tests.rs"]
mod final_tests;

#[cfg(test)]
#[path = "apply_instance_readonly_tests.rs"]
mod readonly_tests;

#[cfg(test)]
#[path = "apply_instance_b5c_tests.rs"]
mod b5c_tests;

#[cfg(test)]
#[path = "apply_instance_b3_tests.rs"]
mod b3_tests;

#[cfg(test)]
#[path = "apply_instance_b6a1_tests.rs"]
mod b6a1_tests;

#[cfg(test)]
#[path = "apply_instance_b6a2_tests.rs"]
mod b6a2_tests;
