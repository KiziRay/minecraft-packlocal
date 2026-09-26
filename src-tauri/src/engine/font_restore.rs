//! 移除字體包：只處理字體工具放進遊戲的東西（字體包檔案、資源包清單裡的字體包項目），翻譯不動。
//! 與「移除翻譯」走同一套前置檢查（apply_restore.rs）：遊戲開著不動、標記要對得上、
//! 內容要是工具寫入的版本、原本不是工具的檔在隔離區（列出位置）。

use std::fs;
use std::path::Path;

use super::apply_guard::Ctx;
use super::apply_knowledge::Knowledge;
use super::apply_record::{self, OWNER_FONT};
use super::apply_restore;
use super::game_process::{self, GameRunning};
use super::jar_scan::resolve_minecraft_dir;
use super::paths::long_path;

pub fn remove_font_pack_in(instance_path: &Path) -> Result<String, String> {
    let running = game_process::is_game_running(instance_path);
    remove_font_pack_with_game_state(instance_path, running)
}

pub fn remove_font_pack_with_game_state(instance_path: &Path, running: GameRunning) -> Result<String, String> {
    if let GameRunning::Yes { detail } = &running {
        return Err(format!(
            "Minecraft 正在使用這個模組整合包，現在移除字體包可能讓檔案被鎖住或只移除一半。\n\
請完全關閉遊戲後再按「移除字體包」。（{detail}）"
        ));
    }
    let mc = resolve_minecraft_dir(instance_path)?;
    let mut knowledge = Knowledge::load(&mc, None)?;
    let has_font = knowledge.record.files.values().any(|e| e.owner == OWNER_FONT)
        || !knowledge.record.options.font_packs_added.is_empty();
    let Some(ctx) = Ctx::existing(&mc)?.filter(|_| has_font) else {
        return Err("這個遊戲資料夾沒有用工具裝過字體包，沒有需要移除的字體包。".into());
    };
    let outcome = apply_restore::restore_all(&ctx, &mut knowledge, OWNER_FONT);
    apply_record::save(&mc, &mut knowledge.record)?;
    remove_empty_pack_dirs(&mc, &outcome.removed_files);
    if !outcome.failures.is_empty() {
        return Err(format!(
            "移除字體包沒有全部完成（{} 項失敗），請完全關閉遊戲後再按一次「移除字體包」。\n失敗項目：\n{}",
            outcome.failures.len(),
            outcome.failures.join("\n")
        ));
    }
    let summary = apply_restore::describe(&outcome)
        .replacen("已移除翻譯。", "已移除字體包（翻譯不受影響）。", 1)
        .replace("工具加的資源包", "字體包");
    // 識別碼認回的說明（部分檔案被整合包更新改過）放在最前面
    Ok(match knowledge.notice.as_deref() {
        Some(notice) => format!("{notice}\n\n{summary}"),
        None => summary,
    })
}

/// 刪掉字體包檔案後留下的空資料夾（只清到 resourcepacks 為止）。
fn remove_empty_pack_dirs(mc: &Path, removed: &[String]) {
    let stop = mc.join("resourcepacks");
    for rel in removed {
        let mut dir = mc.join(rel).parent().map(Path::to_path_buf);
        while let Some(current) = dir {
            if current == stop || !current.starts_with(&stop) {
                break;
            }
            // 只刪空的；有任何東西就停
            if fs::remove_dir(long_path(&current)).is_err() {
                break;
            }
            dir = current.parent().map(Path::to_path_buf);
        }
    }
}
