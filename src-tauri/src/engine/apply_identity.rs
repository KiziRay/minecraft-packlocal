//! 整合包識別碼：複製出來的資料夾、原位置連不到、標記壞掉、標記被刪掉時怎麼認。
//!
//! - 標記讀不懂：停下來，附位置與修復方法（mcpl_marker::read_instance 的訊息）。
//! - 標記不見了：用紀錄記的遊戲資料夾位置＋每個檔案的指紋重新配對；
//!   剛好一份對得上 → 沿用原本的識別碼並補回 `.mcpl`；對不上 → 舊紀錄保留，這次當新的整合包，並在結果裡說明。
//!   「對得上」＝全部相符，或多數相符（見 [`RECLAIM_MIN_MATCHED`]、[`RECLAIM_MIN_PERCENT`]）；
//!   多數相符時，不符的檔記成「無法確認」（不當原檔備份、移除時不動），並說明「部分檔案被整合包更新改過，其餘已認回」。
//! - 紀錄記的位置跟目前不同：原位置還在、識別碼相同 → 是整份複製出來的，停下並提供
//!   「把這份當成新的整合包」；原位置不在但所在磁碟連得到 → 搬家或改名；連不到 → 停下說明。

use std::fs;
use std::path::{Path, PathBuf};

use super::apply_guard::Ctx;
use super::apply_knowledge::Knowledge;
use super::apply_record::{self, ApplyRecord};
use super::mcpl_marker as mk;
use super::paths::long_path;

/// 給前端辨識「可以提供『把這份當成新的整合包』」的固定字樣。
pub const FORK_HINT: &str = "把這份當成新的整合包";

/// 部分認回的門檻：紀錄上還在遊戲裡的工具檔，至少要有這麼多個指紋相符。
/// 只有一兩個相符可能是碰巧（例如同一版整合包的另一份翻譯結果），不足以證明是同一個整合包。
pub const RECLAIM_MIN_MATCHED: usize = 3;
/// 部分認回的門檻：相符的比例（百分比）。整合包更新常只改少數檔，
/// 80% 讓少數被改過的檔不致讓整份紀錄作廢；大半都不同時則不認，免得認錯整合包。
pub const RECLAIM_MIN_PERCENT: usize = 80;

/// 認回的判斷結果：還沒寫任何檔。
#[derive(Debug, Clone)]
pub struct Reclaim {
    /// 沿用的原本識別碼
    pub id: String,
    /// 紀錄上的內容（還沒加上「無法確認」）
    record: ApplyRecord,
    /// 指紋對不上的檔
    pub changed: Vec<String>,
}

impl Reclaim {
    /// 認回後的紀錄：不符的檔記成「無法確認」（套用時先隔離、移除時不動）。
    pub fn reclaimed_record(&self) -> ApplyRecord {
        let mut record = self.record.clone();
        record.unconfirmed.extend(self.changed.iter().cloned());
        record
    }

    /// 部分認回時給玩家看的說明（全部相符時為 `None`）。
    pub fn notice(&self) -> Option<String> {
        if self.changed.is_empty() {
            return None;
        }
        let mut shown: Vec<String> =
            self.changed.iter().take(10).map(|r| format!("  - {}", super::apply_restore::display_rel(r))).collect();
        if self.changed.len() > 10 {
            shown.push(format!("  …另有 {} 個", self.changed.len() - 10));
        }
        Some(format!(
            "找到這個遊戲資料夾以前的套用紀錄：部分檔案被整合包更新改過，其餘已認回。\n\
以下 {} 個檔案和紀錄對不上，無法確認是不是工具放的，工具不會把它們當成原本的檔案，也不會刪掉或蓋回：\n{}",
            self.changed.len(),
            shown.join("\n")
        ))
    }
}

/// 只判斷、不寫任何檔（查詢狀態用）：識別碼標記要讀得懂；不見了就看紀錄能不能重新配對。
/// 標記在、或認不回來時回 `None`。
pub fn find_reclaim(mc: &Path) -> Result<Option<Reclaim>, String> {
    if mk::read_instance(mc)?.is_some() {
        return Ok(None);
    }
    let mut paired: Vec<Reclaim> = records_for_path(mc)
        .into_iter()
        .filter_map(|(id, record)| fingerprints_match(mc, &record).map(|changed| Reclaim { id, record, changed }))
        .collect();
    // 只有剛好一份對得上才沿用；兩份以上無法判斷是哪一份
    if paired.len() != 1 {
        return Ok(None);
    }
    Ok(paired.pop())
}

/// 前置條件（只在會動檔的動作裡用）：判斷後真的認回——補回 `.mcpl`、把不符的檔記進紀錄。
/// 部分認回時回傳給玩家看的說明（全部相符或沒有認回時為 `None`）。
pub fn require_identity(mc: &Path) -> Result<Option<String>, String> {
    let Some(reclaim) = find_reclaim(mc)? else {
        return Ok(None);
    };
    mk::create_instance_with_id(mc, &reclaim.id)?;
    crate::dev_log!("apply", "識別碼標記不見了，依紀錄位置與檔案指紋認回：{}（不符 {} 個）", reclaim.id, reclaim.changed.len());
    if !reclaim.changed.is_empty() {
        apply_record::save(mc, &mut reclaim.reclaimed_record())?;
    }
    Ok(reclaim.notice())
}

/// 前置條件（字體包、修復資源包清單）：先讀紀錄（含 `.mcpl` 被刪時的認回），才確認或建立識別碼。
/// 順序反過來會先產生新的隨機識別碼，認回就被跳過、原本的紀錄接不回來。
pub fn load_then_begin(mc: &Path, result_root: Option<&Path>) -> Result<(Ctx, Knowledge), String> {
    let identity_known = mk::read_instance(mc)?.is_some();
    let mut knowledge = Knowledge::load(mc, result_root)?;
    let ctx = Ctx::begin(mc)?;
    if !identity_known {
        // 第一次建立識別碼時舊鍵資料可能剛搬到新鍵：重新讀一次（認回的說明留著）
        let notice = knowledge.notice.take();
        knowledge = Knowledge::load(mc, result_root)?;
        knowledge.notice = knowledge.notice.take().or(notice);
    }
    Ok((ctx, knowledge))
}

/// 同一個遊戲資料夾位置、但識別碼不是目前這個的舊紀錄（重新配對不上的）。
pub fn orphan_records(mc: &Path) -> Vec<PathBuf> {
    let current = mk::read_instance(mc).ok().flatten().map(|info| info.id);
    records_for_path(mc)
        .into_iter()
        .filter(|(id, _)| current.as_deref() != Some(id.as_str()))
        .map(|(id, _)| records_root().join(id))
        .collect()
}

fn records_root() -> PathBuf {
    apply_record::store_root().join("apply-records")
}

/// 紀錄記的遊戲資料夾就是這裡、而且真的記了檔案的紀錄（不含舊鍵，舊鍵由 ensure_instance 搬移）。
fn records_for_path(mc: &Path) -> Vec<(String, ApplyRecord)> {
    let here = apply_record::normalized_key_source(mc);
    let legacy = apply_record::legacy_path_key(mc);
    let Ok(entries) = fs::read_dir(long_path(&records_root())) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().to_string();
            if id == legacy {
                return None;
            }
            let text = fs::read_to_string(entry.path().join(apply_record::RECORD_FILE)).ok()?;
            let record: ApplyRecord = serde_json::from_str(&text).ok()?;
            let same_place = !record.mc_dir.is_empty()
                && apply_record::normalized_key_source(Path::new(&record.mc_dir)) == here;
            (same_place && !record.files.is_empty()).then_some((id, record))
        })
        .collect()
}

/// 紀錄上還在遊戲裡的檔：全部是工具寫入的版本（至少一個），或多數相符（達兩個門檻）。
/// 對得上回傳不符的檔（全部相符為空）；對不上回 `None`。
fn fingerprints_match(mc: &Path, record: &ApplyRecord) -> Option<Vec<String>> {
    let mut matched = 0usize;
    let mut changed = Vec::new();
    for rel in record.files.keys() {
        let path = mc.join(rel);
        if !long_path(&path).is_file() {
            continue;
        }
        let sha = apply_record::file_sha256(&path);
        if record.is_tool_version(rel, sha.as_deref()) {
            matched += 1;
        } else {
            changed.push(rel.clone());
        }
    }
    let present = matched + changed.len();
    let all = matched > 0 && changed.is_empty();
    let most = matched >= RECLAIM_MIN_MATCHED && matched * 100 >= present * RECLAIM_MIN_PERCENT;
    (all || most).then_some(changed)
}

/// 前置條件：紀錄記的遊戲資料夾跟目前選的不同時，分清楚是搬家／改名、整份複製、還是原位置暫時連不到。
pub fn check_recorded_location(mc: &Path, recorded: &str) -> Result<(), String> {
    if recorded.is_empty()
        || apply_record::normalized_key_source(Path::new(recorded)) == apply_record::normalized_key_source(mc)
    {
        return Ok(());
    }
    let other = Path::new(recorded);
    match location_state(other) {
        Ok(true) => {
            let same_identity = mk::read_instance(other).ok().flatten().map(|i| i.id)
                == mk::read_instance(mc).ok().flatten().map(|i| i.id);
            if same_identity {
                return Err(format!(
                    "這份套用紀錄記的遊戲資料夾與你目前選的不符：這個資料夾看起來是從另一個遊戲資料夾整份複製來的，\
兩邊不能共用同一份紀錄。為了不改到另一個遊戲，已經停止，沒有動任何檔案。\n\
如果這份是你另外要玩的模組整合包，請按「{FORK_HINT}」：工具會替它建立自己的紀錄，\
從另一份複製過來的翻譯檔會先保存到隔離區再處理，另一份不受影響。\n\
紀錄記的：{recorded}\n目前選的：{}",
                    mc.display()
                ));
            }
            Ok(())
        }
        // 原位置不在、但它所在的磁碟連得到：搬家或改名，照常沿用
        Ok(false) => Ok(()),
        Err(reason) => Err(format!(
            "這份套用紀錄記的遊戲資料夾現在連不到（{reason}）：{recorded}\n\
可能是外接硬碟、隨身碟或網路磁碟沒有接上。無法確定目前這個遊戲資料夾是從那裡搬過來、還是複製出來的，\
為了不改錯，已經停止，沒有動任何檔案。\n\
請先接上那個位置再試一次。如果那個位置已經永久不在了，可以按「{FORK_HINT}」。\n目前選的：{}",
            mc.display()
        )),
    }
}

/// Ok(true)＝在；Ok(false)＝確定不在（所在的磁碟或上層資料夾連得到）；Err＝連不到、無法判斷。
fn location_state(path: &Path) -> Result<bool, String> {
    match fs::metadata(long_path(path)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let reachable = path
                .ancestors()
                .skip(1)
                .filter(|a| !a.as_os_str().is_empty())
                .any(|a| fs::metadata(long_path(a)).is_ok());
            if reachable {
                Ok(false)
            } else {
                Err("找不到它所在的磁碟或網路位置".into())
            }
        }
        Err(error) => Err(error.to_string()),
    }
}

/// 把目前這個遊戲資料夾當成新的整合包（給複製出來、或原位置已永久不在的資料夾用）。回傳新的識別碼。
/// 舊的標記不刪：搬到 `.mcpl/copied-from-<舊識別碼>/` 留作痕跡，新紀錄一律從「無法確定」開始。
pub fn fork_instance(mc: &Path) -> Result<String, String> {
    let old = mk::read_instance(mc)?.ok_or_else(|| "這個遊戲資料夾還沒有工具的識別碼，不需要另外建立。".to_string())?;
    let dir = mk::marker_dir(mc);
    let keep = dir.join(format!("{}{}", mk::COPIED_FROM_PREFIX, old.id));
    fs::create_dir_all(long_path(&keep)).map_err(|e| format!("無法保存舊的標記（{e}）：{}", keep.display()))?;
    for name in ["files", "options.json"] {
        let from = dir.join(name);
        if long_path(&from).exists() {
            fs::rename(long_path(&from), long_path(&keep.join(name)))
                .map_err(|e| format!("無法保存舊的標記（{e}）：{}", from.display()))?;
        }
    }
    fs::rename(long_path(&dir.join("instance.json")), long_path(&keep.join("instance.json")))
        .map_err(|e| format!("無法保存舊的識別碼（{e}）：{}", dir.display()))?;
    let (id, _) = apply_record::ensure_instance(mc)?;
    crate::dev_log!("apply", "把 {} 當成新的整合包：{} → {id}", mc.display(), old.id);
    Ok(id)
}
