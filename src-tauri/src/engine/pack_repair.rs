//! 修復資源包清單：與套用走同一套前置檢查、紀錄、標記流程。
//!
//! 1. 遊戲開著 → 不動。
//! 2. 先存紀錄（這次修復是一個批次）與設定檔旁另存檔的標記，再寫檔。
//! 3. 設定檔旁的另存檔只在第一次建立（內容永遠是工具第一次動之前的樣子）。
//! 4. 設定檔用暫存檔＋改名寫入。

use std::fs;
use std::path::Path;

use super::apply_guard::{self, Ctx};
use super::apply_record::{self, ApplyRecord, FileKind, Origin, OWNER_REPAIR};
use super::game_process::{self, GameRunning};
use super::hashutil::sha256_hex;
use super::mcpl_marker as mk;
use super::options_txt;
use super::paths::long_path;

/// 修復結果：加回幾個資源包，以及識別碼認回時要告訴玩家的話。
#[derive(Debug, Clone, Default)]
pub struct RepairOutcome {
    pub added: usize,
    pub notice: Option<String>,
}

pub fn repair_pack_list(mc: &Path) -> Result<RepairOutcome, String> {
    let running = game_process::is_game_running(mc);
    repair_pack_list_reporting(mc, running)
}

/// 測試用：只看加回幾個。
#[cfg(test)]
pub fn repair_pack_list_with_game_state(mc: &Path, running: GameRunning) -> Result<usize, String> {
    repair_pack_list_reporting(mc, running).map(|outcome| outcome.added)
}

pub fn repair_pack_list_reporting(mc: &Path, running: GameRunning) -> Result<RepairOutcome, String> {
    if let GameRunning::Yes { detail } = &running {
        return Err(format!(
            "Minecraft 正在使用這個模組整合包，遊戲關閉時會把設定寫回去，現在修復會被蓋掉。\n\
請完全關閉遊戲後再修復資源包清單。（{detail}）"
        ));
    }
    let options = mc.join("options.txt");
    let original = match fs::read_to_string(long_path(&options)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("讀不到遊戲設定檔，沒有做任何修改：{e}")),
    };
    let Some((updated, added)) = super::resource_pack_guard::repaired_options(mc, &original) else {
        return Ok(RepairOutcome::default());
    };
    // 前置條件：先讀紀錄（`.mcpl` 被刪時先認回原本的識別碼），才確認或建立識別碼
    let (ctx, mut knowledge) = super::apply_identity::load_then_begin(mc, None)?;
    let markers: Vec<mk::FileMarker> =
        options_bak_marker(&ctx, &mut knowledge.record, &original, OWNER_REPAIR).into_iter().collect();
    apply_guard::require_record_and_markers(&ctx, &mut knowledge.record, &markers, None, &[])?;
    options_txt::backup_beside_once(&options, &original).map_err(|e| format!("另存遊戲設定檔失敗，已停止：{e}"))?;
    for marker in &markers {
        mk::mark_written(mc, &marker.rel)?;
    }
    apply_record::write_atomic(&options, updated.as_bytes()).map_err(|e| format!("寫入遊戲設定檔失敗：{e}"))?;
    Ok(RepairOutcome { added, notice: knowledge.notice })
}

/// 設定檔旁的另存檔還不存在時：先在紀錄與標記登記它（工具新增的檔）。已存在就不動。
/// `owner`＝實際建立它的功能（空＝翻譯、字體包、修復清單），由那個功能的「移除」處理。
pub(super) fn options_bak_marker(
    ctx: &Ctx,
    record: &mut ApplyRecord,
    original: &str,
    owner: &str,
) -> Option<mk::FileMarker> {
    let bak = ctx.mc.join("options.txt.mcpl-bak");
    if long_path(&bak).exists() {
        return None;
    }
    let rel = apply_record::rel_key(&ctx.mc, &bak);
    let sha = sha256_hex(original.as_bytes());
    let id = mk::new_marker_id();
    record.set_entry(&rel, FileKind::Added, Origin::Known, None, sha.clone());
    if let Some(entry) = record.files.get_mut(&rel) {
        entry.marker_id = id.clone();
        entry.owner = owner.to_string();
    }
    Some(mk::FileMarker {
        id,
        instance_id: ctx.instance_id.clone(),
        rel,
        role: "added".into(),
        tool_sha256: sha,
        original_sha256: String::new(),
        backup: String::new(),
        tool_version: env!("CARGO_PKG_VERSION").into(),
        written_at: mk::now_secs(),
        origin: "known".into(),
        links: vec![mk::link(&ctx.batch_id, mk::REL_BATCH)],
        tool_history: Vec::new(),
        state: "pending".into(),
    })
}
