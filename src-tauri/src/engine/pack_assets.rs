//! B3#5：assets 類書本（Patchouli `assets/<ns>/patchouli_books`、GuideME／Lavender 手冊）
//! 改放進主翻譯資源包，不再改寫模組 JAR 或別人的資源包資料夾。
//!
//! 翻譯時寫到翻譯結果的 `pack-assets/assets/…`（內容已在翻譯端過 output guard）；
//! 建主資源包時整棵複製進資源包。資源包排在清單最上層，遊戲會讀到這份 zh_tw。

use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

pub const PACK_ASSETS_DIR: &str = "pack-assets";

/// 書頁譯文的輸出位置：路徑中有 `assets/<ns>/…` 的放進 `pack-assets/assets/<ns>/…`；
/// 其餘（data/ 書本、遊戲資料夾的 patchouli_books）維持原相對位置。
pub fn book_destination(rel: &Path) -> PathBuf {
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    match parts.iter().position(|p| p.eq_ignore_ascii_case("assets")) {
        Some(i) if i + 2 < parts.len() => {
            let mut out = PathBuf::from(PACK_ASSETS_DIR);
            for p in &parts[i..] {
                out.push(p);
            }
            out
        }
        _ => rel.to_path_buf(),
    }
}

/// 把 `from/pack-assets` 底下的檔移到 `to/pack-assets`（JAR 書本的暫存輸出 → 翻譯結果）。
pub fn move_into(from_root: &Path, to_root: &Path) -> Result<usize, String> {
    copy_tree(&from_root.join(PACK_ASSETS_DIR), &to_root.join(PACK_ASSETS_DIR))
}

/// 本輪書本輸出的暫存換新（審查 4）。
///
/// 整輪翻譯開頭把上一輪的 `pack-assets` 改名成 `pack-assets.old`，三個產出者（JAR 書本、覆寫文字、
/// 資料包 ZIP）把新書頁寫進新的 `pack-assets`。結束時：
/// - 失敗、停止、AI 不可用、任何提早離開（含 panic）：舊書頁全部併回（新的優先）——不丟已翻的書頁；
/// - 三個產出者都完整跑完：舊書頁只留「產出清單上仍有效」的（來源還在、只是這輪沒重做，照 G3.26 不退休），
///   清單上沒有的（來源已被拿掉、舊版沒記的）才真的丟掉。
/// 建包（`copy_into_pack`）在換新期間會把舊書頁當底、新的蓋上去，中途建出的 zip 也不會少書。
pub struct Swap {
    work: PathBuf,
    done: bool,
}

const OLD_DIR: &str = "pack-assets.old";

fn merge_missing(from: &Path, to: &Path, keep: &dyn Fn(&Path) -> bool) -> Result<(), String> {
    if !from.is_dir() {
        return Ok(());
    }
    for entry in WalkDir::new(from).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(from) else { continue };
        let target = to.join(rel);
        if target.exists() || !keep(rel) {
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::copy(entry.path(), &target).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    Ok(())
}

impl Swap {
    /// 把上一輪的書本輸出移到旁邊。上次當機留下的舊份先併回。資料夾不存在不算錯。
    pub fn begin(work_root: &Path) -> Result<Swap, String> {
        let cur = work_root.join(PACK_ASSETS_DIR);
        let old = work_root.join(OLD_DIR);
        if old.is_dir() {
            merge_missing(&old, &cur, &|_| true)?;
            fs::remove_dir_all(&old).map_err(|e| e.to_string())?;
        }
        if cur.is_dir() {
            fs::rename(&cur, &old).map_err(|e| format!("無法暫存上一輪的書本輸出：{e}"))?;
        }
        Ok(Swap { work: work_root.to_path_buf(), done: false })
    }

    /// `success`＝三個產出者都完整跑完；`keep(相對 pack-assets 的路徑)`＝成功時哪些舊書頁仍有效。
    pub fn finish(mut self, success: bool, keep: &dyn Fn(&Path) -> bool) {
        self.done = true;
        self.settle(success, keep);
    }

    fn settle(&self, success: bool, keep: &dyn Fn(&Path) -> bool) {
        let old = self.work.join(OLD_DIR);
        let result = merge_missing(&old, &self.work.join(PACK_ASSETS_DIR), &|rel| !success || keep(rel));
        match result {
            // 併回失敗就留著舊份（下一輪開頭會再併回），不刪
            Err(e) => crate::dev_log!("translate", "書本輸出併回失敗，舊書頁留在 {OLD_DIR}：{e}"),
            Ok(()) => {
                let _ = fs::remove_dir_all(&old);
            }
        }
    }
}

impl Drop for Swap {
    fn drop(&mut self) {
        if !self.done {
            self.settle(false, &|_| true);
        }
    }
}

/// 建主資源包時呼叫：`work_root/pack-assets/assets` → `pack_dir/assets`。回傳複製的檔數。
pub fn copy_into_pack(work_root: &Path, pack_dir: &Path) -> Result<usize, String> {
    // 本輪書本輸出換新期間（Swap），上一輪的舊書頁當底、新的蓋上去
    let old = copy_tree(&work_root.join(OLD_DIR).join("assets"), &pack_dir.join("assets"))?;
    Ok(old + copy_tree(&work_root.join(PACK_ASSETS_DIR).join("assets"), &pack_dir.join("assets"))?)
}

fn copy_tree(src: &Path, dest: &Path) -> Result<usize, String> {
    if !src.is_dir() {
        return Ok(0);
    }
    let mut n = 0usize;
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(src) else { continue };
        let target = dest.join(rel);
        // 審查 F7：內容在翻譯端已過逐條檢查；複製前再過一次檔案層檢查（編碼、BOM、讀得回來），
        // 不合格的檔不放進資源包（遊戲會退回原本的語言）
        let bytes = fs::read(entry.path()).map_err(|e| format!("{}: {e}", entry.path().display()))?;
        let name = rel.to_string_lossy().replace('\\', "/");
        let empty: &[u8] = if name.ends_with(".json") { b"{}" } else { b"" };
        let checked = super::output_guard::finish_file(&name, empty, bytes);
        if checked == empty {
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&target, checked).map_err(|e| format!("{}: {e}", target.display()))?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b3_main_resource_pack_contains_pack_assets_books() {
        let work = crate::engine::tool_products::test_support::temp_game("packassets");
        crate::engine::tool_products::test_support::write(
            &work.join("pack-assets/assets/ns/patchouli_books/b/zh_tw/e.json"),
            r#"{"name":"書頁"}"#,
        );
        let opts = crate::engine::pack_out::BuildOptions {
            output_dir: work.display().to_string(),
            pack_folder_name: "p".into(),
            pack_description: "t".into(),
            pack_format: 15,
            target_version: Some("1.20.1".into()),
        };
        crate::engine::pack_out::build_resource_pack(&crate::engine::jar_scan::LangMap::new(), &opts).unwrap();
        assert!(work.join("resourcepacks/p/assets/ns/patchouli_books/b/zh_tw/e.json").is_file());
        let _ = fs::remove_dir_all(&work);
    }

    #[test]
    fn b3_assets_books_go_to_pack_assets_data_books_stay() {
        assert_eq!(
            book_destination(Path::new("resourcepacks/UserPack/assets/ns/patchouli_books/b/zh_tw/e.json")),
            PathBuf::from("pack-assets/assets/ns/patchouli_books/b/zh_tw/e.json")
        );
        assert_eq!(
            book_destination(Path::new("kubejs/assets/ns/patchouli_books/b/zh_tw/e.json")),
            PathBuf::from("pack-assets/assets/ns/patchouli_books/b/zh_tw/e.json")
        );
        assert_eq!(
            book_destination(Path::new("data/ns/patchouli_books/b/zh_tw/e.json")),
            PathBuf::from("data/ns/patchouli_books/b/zh_tw/e.json")
        );
    }
}
