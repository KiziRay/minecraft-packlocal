//! 測試用：在假遊戲資料夾裡造出「工具寫過」的檔與原檔備份。

use std::path::{Path, PathBuf};

use crate::engine::apply_guard::{self, BackupMarker};
use crate::engine::apply_record;
use crate::engine::mcpl_marker::{self, FileMarker};

fn marker(rel: &str, tool_sha: String, original_sha: String, role: &str) -> FileMarker {
    FileMarker {
        id: "test".into(),
        instance_id: "test-instance".into(),
        rel: rel.into(),
        role: role.into(),
        tool_sha256: tool_sha,
        original_sha256: original_sha,
        backup: String::new(),
        tool_version: "test".into(),
        written_at: 0,
        origin: "known".into(),
        links: Vec::new(),
        tool_history: Vec::new(),
        state: "written".into(),
    }
}

/// 像真的套用過一樣：遊戲資料夾有識別碼、套用紀錄記著這個檔。
fn record_applied(mc: &Path, rel: &str, sha: &str, kind: apply_record::FileKind) {
    if mcpl_marker::read_instance(mc).ok().flatten().is_none() {
        mcpl_marker::create_instance(mc).expect("識別碼");
    }
    let mut record = apply_record::load(mc).unwrap_or_default();
    record.set_entry(rel, kind, apply_record::Origin::Known, None, sha.to_string());
    apply_record::save(mc, &mut record).expect("紀錄");
}

/// 替遊戲資料夾裡的檔寫一份「工具新增」標記（指紋＝目前內容）。
pub fn mark_as_tool_written(mc: &Path, rel: &str) {
    let sha = apply_record::file_sha256(&mc.join(rel)).expect("檔案存在");
    record_applied(mc, rel, &sha, apply_record::FileKind::Added);
    mcpl_marker::write_file_marker(mc, &marker(rel, sha, String::new(), "added")).expect("寫標記");
}

/// 遊戲裡的 `rel` 目前是工具覆蓋後的版本（內容＝`tool_text`），原檔（`original_text`）有備份。
pub fn mark_overwritten_with_backup(mc: &Path, rel: &str, tool_text: &str, original_text: Option<&str>) {
    write(&mc.join(rel), tool_text);
    let tool_sha = apply_record::file_sha256(&mc.join(rel)).unwrap();
    let original_sha = original_text.map(|t| crate::engine::hashutil::sha256_hex(t.as_bytes())).unwrap_or_default();
    record_applied(mc, rel, &tool_sha, apply_record::FileKind::Overwritten);
    mcpl_marker::write_file_marker(mc, &marker(rel, tool_sha, original_sha.clone(), "overwritten")).unwrap();
    if let Some(text) = original_text {
        write(&apply_guard::backup_file_path(mc, rel), text);
        let backup = BackupMarker {
            id: "b".into(),
            instance_id: mcpl_marker::read_instance(mc).unwrap().unwrap().id,
            rel: rel.into(),
            original_sha256: original_sha,
            state: "valid".into(),
            links: Vec::new(),
            created_at: 0,
            legacy_manifest: String::new(),
            legacy_manifest_sha256: String::new(),
        };
        write(&apply_guard::backup_marker_path(mc, rel), &serde_json::to_string(&backup).unwrap());
    }
}

pub fn temp_game(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mcpl-b3-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}
