//! B2 第二輪 F5：建包時的英文原文表（不會縮減的來源）。
//!
//! 問題：補翻、修復、貼回翻譯都不重掃，會從舊資源包讀回譯文再寫出一次；
//! 工作階段的待補清單（pending_en）只有「還沒翻的」而且每輪收尾會縮，
//! 用它查原文時，正要重新寫出的舊 AI 譯文反而查不到原文，只做了空白與亂碼檢查。
//!
//! 做法：掃描時收集**完整** en_us（jar_scan），整輪翻譯寫進翻譯結果的
//! 「英文原文表.json」，之後不再縮。所有建包路徑建包前一律：清空原文表 → 載入這份全表。
//! 舊版翻譯結果沒有這份表：先依工作階段記的遊戲資料夾重掃補齊並存檔（必須先有原文，才能建包）；
//! 遊戲資料夾已不在、重掃不了：照常建包，但查不到原文的條目只做不需原文的檢查，
//! 並逐條記「缺原文未完整檢查」（output_guard::lang_entry）。

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::jar_scan::LangMap;

pub const CATALOG_FILE: &str = "英文原文表.json";

/// 建包前準備原文表的結果（寫進日誌給玩家看）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogSource {
    /// 讀到翻譯結果裡的英文原文表
    Saved,
    /// 舊結果沒有原文表：重掃遊戲資料夾補齊並存檔
    Rescanned,
    /// 沒有原文表、遊戲資料夾也不在：查不到原文的條目只做部分檢查
    Missing,
    /// 沒有原文表、遊戲資料夾還在，但重掃失敗（按了停止、不是遊戲資料夾、讀取出錯）
    RescanFailed,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CatalogFile {
    version: u32,
    /// 完整英文原文（en_us）
    en: LangMap,
    /// 模組自帶 zh_tw（模組作者自己的翻譯，不套長度等檢查）
    #[serde(default)]
    native: LangMap,
    /// 使用者選的參考包／CFPA 人工譯文（寫出時只免長度檢查）；舊檔沒有這欄
    #[serde(default)]
    reference: LangMap,
}

/// 整輪翻譯掃描後存下完整英文原文表（暫存檔＋改名）。
pub fn save(work: &Path, en: &LangMap) -> Result<(), String> {
    let native = super::output_guard::snapshot_native();
    let reference = super::output_guard::snapshot_reference();
    fs::create_dir_all(work).map_err(|e| e.to_string())?;
    let body = serde_json::to_string(&CatalogFile {
        version: 1,
        en: en.clone(),
        native,
        reference,
    })
    .map_err(|e| e.to_string())?;
    let path = work.join(CATALOG_FILE);
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body).map_err(|e| format!("寫入英文原文表失敗：{e}"))?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("置換英文原文表失敗：{e}"))?;
    }
    fs::rename(&tmp, &path).map_err(|e| format!("儲存英文原文表失敗：{e}"))
}

fn load_file(work: &Path) -> Option<CatalogFile> {
    let text = fs::read_to_string(work.join(CATALOG_FILE)).ok()?;
    serde_json::from_str::<CatalogFile>(&text).ok().filter(|c| !c.en.is_empty())
}

/// 讀翻譯結果裡的英文原文表；沒有或壞掉回 None（測試用；正式流程走 prepare_build_sources）。
#[cfg(test)]
pub fn load(work: &Path) -> Option<LangMap> {
    load_file(work).map(|c| c.en)
}

/// 所有建包路徑建包前呼叫：清空原文表 → 載入全表；舊結果沒有全表時先重掃遊戲資料夾補齊並存檔。
pub fn prepare_build_sources(work: &Path, instance: &Path) -> CatalogSource {
    super::output_guard::reset_sources();
    if let Some(file) = load_file(work) {
        super::output_guard::remember_sources(&file.en);
        super::output_guard::remember_native(&file.native);
        super::output_guard::remember_reference(&file.reference);
        return CatalogSource::Saved;
    }
    // 前置條件：必須先有英文原文，才能完整檢查要寫出的譯文 → 重掃補齊（只讀，不動遊戲資料夾）
    if !instance.exists() {
        return CatalogSource::Missing;
    }
    let scanned = super::jar_scan::scan_instance(instance, &HashMap::new(), false, false, |_, _| {});
    if scanned.is_ok() {
        let en = super::output_guard::snapshot_sources();
        if !en.is_empty() {
            let _ = save(work, &en);
            return CatalogSource::Rescanned;
        }
    }
    super::output_guard::reset_sources();
    CatalogSource::RescanFailed
}

/// 參考包合併後，由參考包補進來的條目（來源標記 RefPack 且參考包有這個鍵）實際要寫出的值。
/// 登記的是轉繁、詞典處理後 zh 裡的值，寫出時才比對得上。
pub fn reference_values(zh: &LangMap, ref_zh: &LangMap, prov: &super::lang_provenance::ProvenanceMap) -> LangMap {
    use super::lang_provenance::{get_source, LangSource};
    let mut out = LangMap::new();
    for (ns, entries) in ref_zh {
        let Some(zh_ns) = zh.get(ns) else {
            continue;
        };
        for key in entries.keys() {
            if get_source(prov, ns, key) != Some(LangSource::RefPack) {
                continue;
            }
            if let Some(value) = zh_ns.get(key) {
                out.entry(ns.clone()).or_default().insert(key.clone(), value.clone());
            }
        }
    }
    out
}

/// 參考包合併。使用者**明確指定**的參考包（人工譯文）先登記候選值，合併時的 guard 只免長度；
/// 自動搜到的參考包（下載／文件／桌面）可能是本工具以前輸出的 AI 譯文，不登記、照常完整檢查。
pub fn merge_reference(zh: &mut LangMap, ref_zh: &LangMap, user_chosen: bool) -> usize {
    if user_chosen {
        super::output_guard::remember_reference(ref_zh);
    }
    super::merge_ref::merge_fill_missing(zh, ref_zh)
}

/// 合併後經過轉繁、詞典處理，再登記實際要寫出的值（只限使用者明確指定的參考包）。
pub fn remember_reference_after_merge(
    zh: &LangMap,
    ref_zh: &LangMap,
    prov: &super::lang_provenance::ProvenanceMap,
    user_chosen: bool,
) {
    if user_chosen {
        super::output_guard::remember_reference(&reference_values(zh, ref_zh, prov));
    }
}

impl CatalogSource {
    /// 給日誌的白話說明。
    pub fn player_note(self) -> Option<&'static str> {
        match self {
            CatalogSource::Saved => None,
            CatalogSource::Rescanned => Some("這份翻譯結果是舊版產生的，已重新讀取遊戲的英文原文，之後寫出的每條譯文都會完整檢查。"),
            CatalogSource::Missing => Some(
                "找不到遊戲資料夾（可能已搬走或刪除），讀不到英文原文：這次寫出的譯文只做了部分安全檢查（空白、亂碼、方框字元、結尾色碼），沒有和原文比對格式碼與長度。把遊戲資料夾放回原處再試，或重新執行一次完整翻譯，就會恢復完整檢查。",
            ),
            CatalogSource::RescanFailed => Some(
                "遊戲資料夾還在，但讀取英文原文時出錯或被停止：這次寫出的譯文只做了部分安全檢查（空白、亂碼、方框字元、結尾色碼），沒有和原文比對格式碼與長度。可以稍後再試一次，或重新執行一次完整翻譯。",
            ),
        }
    }
}

#[cfg(test)]
#[path = "source_catalog_tests.rs"]
mod tests;
