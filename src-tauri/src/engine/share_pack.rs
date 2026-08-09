//! 把「翻譯結果」工作目錄打包成單一 zip，讓使用者可以手動把整包翻譯檔分享給別人。
//!
//! 這是「勾選『建立打包檔案』」才會用到的路徑：預設的一鍵流程是直接覆蓋安裝進遊戲、
//! 不留資料夾；只有想分享時才打包成一個可命名的檔。
//!
//! （上傳 R2 + 產生分享短連結為後續版本；本版先做本機打包成檔。）

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::security::sanitize_folder_name;

/// 打包 `work_root`（翻譯結果工作目錄）全部內容成 `dest_dir/<name>.zip`。
/// 回傳產生的 zip 路徑。空目錄或不存在會回錯，不會產生空包。
pub fn package_translation(work_root: &Path, dest_dir: &Path, name: &str) -> Result<PathBuf, String> {
    if !work_root.is_dir() {
        return Err("找不到翻譯結果資料夾，請先完成翻譯再打包。".into());
    }
    let has_files = WalkDir::new(work_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|e| e.path().is_file());
    if !has_files {
        return Err("翻譯結果是空的，沒有可打包的檔案。".into());
    }
    if dest_dir.as_os_str().is_empty() {
        return Err("請先選擇打包檔要存到哪個資料夾。".into());
    }

    std::fs::create_dir_all(dest_dir).map_err(|e| format!("無法建立輸出資料夾：{e}"))?;
    let safe = match sanitize_folder_name(name) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => "模組包翻譯分享".to_string(),
    };
    let zip_path = dest_dir.join(format!("{safe}.zip"));
    zip_dir(work_root, &zip_path)?;
    Ok(zip_path)
}

fn zip_dir(src: &Path, zip_path: &Path) -> Result<(), String> {
    let file = File::create(zip_path).map_err(|e| format!("無法建立打包檔：{e}"))?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = match path.strip_prefix(src) {
            Ok(r) if !r.as_os_str().is_empty() => r,
            _ => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if path.is_file() {
            zip.start_file(rel_str, opts).map_err(|e| e.to_string())?;
            let bytes = std::fs::read(path)
                .map_err(|e| format!("讀取失敗 {}：{e}", path.display()))?;
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
        } else if path.is_dir() {
            let _ = zip.add_directory(format!("{}/", rel_str.trim_end_matches('/')), opts);
        }
    }
    zip.finish().map_err(|e| format!("打包完成失敗：{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mi18n_pkg_{tag}_{}", std::process::id()))
    }

    #[test]
    fn packages_work_dir_into_named_zip() {
        let root = scratch("ok");
        let _ = fs::remove_dir_all(&root);
        let work = root.join("翻譯結果");
        let rp = work.join("resourcepacks");
        fs::create_dir_all(&rp).unwrap();
        fs::write(rp.join("pack.zip"), b"dummy-pack").unwrap();
        fs::write(work.join("覆蓋範圍說明.txt"), "coverage").unwrap();

        let dest = root.join("out");
        let zip = package_translation(&work, &dest, "我的分享包").unwrap();
        assert!(zip.is_file());
        assert!(zip.file_name().unwrap().to_string_lossy().ends_with(".zip"));

        let f = fs::File::open(&zip).unwrap();
        let mut ar = zip::ZipArchive::new(f).unwrap();
        let names: Vec<String> = (0..ar.len())
            .map(|i| ar.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(names.iter().any(|n| n.contains("resourcepacks/pack.zip")), "{names:?}");
        assert!(names.iter().any(|n| n.contains("覆蓋範圍說明.txt")), "{names:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_or_missing_work_dir_errors() {
        let root = scratch("empty");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        // 空目錄 → 錯
        assert!(package_translation(&root, &root.join("out"), "x").is_err());
        // 不存在 → 錯
        assert!(package_translation(&root.join("nope"), &root.join("out"), "x").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn bad_name_falls_back_to_default() {
        let root = scratch("name");
        let _ = fs::remove_dir_all(&root);
        let work = root.join("w");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("a.txt"), "x").unwrap();
        // 全是不合法字元 → 用預設名
        let zip = package_translation(&work, &root.join("out"), "///").unwrap();
        assert_eq!(zip.file_name().unwrap().to_string_lossy(), "模組包翻譯分享.zip");
        let _ = fs::remove_dir_all(&root);
    }
}
