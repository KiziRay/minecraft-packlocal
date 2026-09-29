//! B5d：選資料夾當下，唯讀判斷這個遊戲資料夾的身分（規格 S05–S07）。
//!
//! 跟 apply_record::load＋apply_identity::check_recorded_location 判斷的是同一件事，差別是：
//! - 只讀（G1.36）：不建 `.mcpl`、不認回、不改紀錄、不設隱藏；
//! - 回分類結果（給狀態卡），不是一段錯誤訊息；
//! - 原位置檢查放背景並設上限，外接硬碟或網路位置斷線時幾秒內就回「連不到」。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::apply_identity;
use super::apply_record::{self, ApplyRecord};
use super::folder_check::run_with_timeout;
use super::mcpl_marker as mk;
use super::paths::long_path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityCheck {
    /// ok｜new｜marker_broken｜record_broken｜copied｜unreachable
    pub state: String,
    /// marker_broken／record_broken：壞掉的那個檔；copied／unreachable：紀錄記的原位置
    pub path: String,
    /// copied：原位置的資料夾名稱（狀態卡「這份是從「<原包名>」複製來的」）
    pub origin_name: String,
}

impl IdentityCheck {
    /// 整體逾時：判斷不了就照舊（失效安全＝舊行為：選資料夾時不擋，套用前的檢查仍在）。
    pub fn unknown() -> Self {
        Self::new("unknown", String::new())
    }

    fn new(state: &str, path: impl Into<String>) -> Self {
        let path = path.into();
        let origin_name = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        Self { state: state.into(), path, origin_name }
    }
}

enum Location {
    Same,
    Moved,
    Copied,
    Unreachable,
}

/// 唯讀判斷身分。`location_timeout`＝原位置檢查的上限。
pub fn inspect_identity(mc: &Path, location_timeout: Duration) -> IdentityCheck {
    let info = match mk::read_instance(mc) {
        Err(_) => {
            return IdentityCheck::new("marker_broken", mk::marker_dir(mc).join("instance.json").display().to_string())
        }
        // 沒有標記：第一次翻、或標記被刪（真正的認回在套用前做，G1.36）
        Ok(None) => return IdentityCheck::new("new", String::new()),
        Ok(Some(info)) => info,
    };
    let record_path = apply_record::record_dir(mc).join(apply_record::RECORD_FILE);
    let record: ApplyRecord = match fs::read_to_string(long_path(&record_path)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return IdentityCheck::new("ok", String::new()),
        Err(_) => return IdentityCheck::new("record_broken", record_path.display().to_string()),
        Ok(text) => match serde_json::from_str(&text) {
            Ok(record) => record,
            Err(_) => return IdentityCheck::new("record_broken", record_path.display().to_string()),
        },
    };
    let recorded = record.mc_dir.trim().to_string();
    let here = mc.to_path_buf();
    let id = info.id.clone();
    let recorded_for_job = recorded.clone();
    let verdict = run_with_timeout(location_timeout, move || locate(&here, &recorded_for_job, &id))
        .unwrap_or(Location::Unreachable);
    match verdict {
        Location::Same | Location::Moved => IdentityCheck::new("ok", String::new()),
        Location::Copied => IdentityCheck::new("copied", recorded),
        Location::Unreachable => IdentityCheck::new("unreachable", recorded),
    }
}

fn locate(mc: &Path, recorded: &str, id: &str) -> Location {
    if recorded.is_empty()
        || apply_record::normalized_key_source(Path::new(recorded)) == apply_record::normalized_key_source(mc)
    {
        return Location::Same;
    }
    let other = PathBuf::from(recorded);
    match apply_identity::location_state(&other) {
        Ok(true) => {
            let same = mk::read_instance(&other).ok().flatten().map(|i| i.id).as_deref() == Some(id);
            if same {
                Location::Copied
            } else {
                Location::Moved
            }
        }
        Ok(false) => Location::Moved,
        Err(_) => Location::Unreachable,
    }
}
