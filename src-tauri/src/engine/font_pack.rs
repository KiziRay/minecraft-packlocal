//! 由使用者字體檔建立 Minecraft 字體資源包（人性化、固定安全路徑）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::jar_scan::resolve_minecraft_dir;
use super::pack_out::{pack_format_for_version, pack_mcmeta_value};
use super::security::{check_font_file, ensure_under_base, sanitize_folder_name};

/// 空名稱時的字體包預設顯示名（勿依賴 sanitize 的翻譯包預設「繁體中文翻譯」）。
const DEFAULT_FONT_PACK_NAME: &str = "繁體中文遊戲字體";
/// 未指定版本／format 時的保底（約對應 1.21）。
const DEFAULT_FONT_PACK_FORMAT: u32 = 34;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontPackResult {
    pub pack_path: String,
    pub player_summary: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontPackApplyResult {
    pub copied_path: String,
    pub backup_path: Option<String>,
    pub player_summary: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FontPackOptions {
    pub size: f32,
    pub weight: f32,
    pub shift_x: f32,
    pub shift_y: f32,
    pub oversample: f32,
}

impl Default for FontPackOptions {
    fn default() -> Self {
        Self {
            size: 11.0,
            weight: 400.0,
            shift_x: 0.0,
            shift_y: 0.5,
            oversample: 4.0,
        }
    }
}

impl FontPackOptions {
    fn normalized(&self) -> Self {
        let finite_or = |value: f32, fallback: f32| {
            if value.is_finite() {
                value
            } else {
                fallback
            }
        };
        Self {
            size: finite_or(self.size, 11.0).clamp(8.0, 24.0),
            weight: finite_or(self.weight, 400.0).clamp(100.0, 900.0),
            shift_x: finite_or(self.shift_x, 0.0).clamp(-3.0, 3.0),
            shift_y: finite_or(self.shift_y, 0.5).clamp(-3.0, 3.0),
            oversample: finite_or(self.oversample, 4.0).clamp(1.0, 8.0),
        }
    }

    fn provider_size(&self) -> f32 {
        let weight_adjustment = (self.weight - 400.0) / 400.0 * 0.9;
        (self.size + weight_adjustment).clamp(8.0, 24.0)
    }
}

/// 依來源副檔名決定資源包內字體檔名；拒絕 `.ttc`。
pub(crate) fn cjk_font_file_name(font_file: &Path) -> Result<&'static str, String> {
    let ext = font_file
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "ttf" => Ok("cjk_font.ttf"),
        "otf" => Ok("cjk_font.otf"),
        "ttc" => Err(
            "不支援 .ttc（TrueType Collection 集合字體）。請改用單一字形的 .ttf 或 .otf 檔。"
                .into(),
        ),
        _ => Err("請使用 .ttf 或 .otf 字體檔。".into()),
    }
}

fn resolve_font_pack_format(pack_format: Option<u16>, target_version: Option<&str>) -> u32 {
    if let Some(f) = pack_format {
        return u32::from(f);
    }
    if let Some(v) = target_version {
        if let Some(f) = pack_format_for_version(v) {
            return f;
        }
    }
    DEFAULT_FONT_PACK_FORMAT
}

fn font_pack_display_name(pack_name: &str) -> Result<String, String> {
    let raw = pack_name.trim();
    let for_sanitize = if raw.is_empty() {
        DEFAULT_FONT_PACK_NAME
    } else {
        raw
    };
    sanitize_folder_name(for_sanitize)
}

pub fn build_font_resource_pack_with_options(
    font_file: &Path,
    output_dir: &Path,
    pack_name: &str,
    pack_desc: &str,
    options: &FontPackOptions,
    pack_format: Option<u16>,
    target_version: Option<&str>,
) -> Result<FontPackResult, String> {
    let font_name = cjk_font_file_name(font_file)?;
    check_font_file(font_file)?;
    let name = font_pack_display_name(pack_name)?;
    let options = options.normalized();
    let desc = if pack_desc.trim().is_empty() {
        "自訂遊戲字體資源包".to_string()
    } else {
        pack_desc.trim().chars().take(120).collect()
    };

    // 字體包也進「翻譯結果/resourcepacks」（若已是工作根則直接用）
    let work = if output_dir
        .file_name()
        .and_then(|s| s.to_str())
        == Some(super::out_layout::RESULT_DIR_NAME)
    {
        output_dir.to_path_buf()
    } else {
        super::out_layout::ensure_result_layout(output_dir)
            .map(|l| l.work_root)
            .unwrap_or_else(|_| output_dir.join(super::out_layout::RESULT_DIR_NAME))
    };
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let pack_root = work.join("resourcepacks").join(&name);
    ensure_under_base(&work, &pack_root)?;

    if pack_root.exists() {
        fs::remove_dir_all(&pack_root).map_err(|e| e.to_string())?;
    }

    let font_dir = pack_root
        .join("assets")
        .join("minecraft")
        .join("font")
        .join("include");
    fs::create_dir_all(&font_dir).map_err(|e| e.to_string())?;

    let dest_font = font_dir.join(font_name);
    fs::copy(font_file, &dest_font).map_err(|e| format!("複製字體失敗：{e}"))?;

    let provider_file = format!("minecraft:include/{font_name}");
    let prov = serde_json::json!({
        "providers": [{
            "type": "ttf",
            "file": provider_file,
            "shift": [options.shift_x, options.shift_y],
            "size": options.provider_size(),
            "oversample": options.oversample
        }]
    });
    let font_json = serde_json::to_string_pretty(&prov).unwrap() + "\n";
    let font_base = pack_root.join("assets").join("minecraft").join("font");
    fs::write(font_base.join("default.json"), &font_json).map_err(|e| e.to_string())?;
    fs::write(font_base.join("uniform.json"), &font_json).map_err(|e| e.to_string())?;

    let fmt = resolve_font_pack_format(pack_format, target_version);
    let meta = pack_mcmeta_value(target_version, fmt, &desc);
    fs::write(
        pack_root.join("pack.mcmeta"),
        serde_json::to_string_pretty(&meta).unwrap() + "\n",
    )
    .map_err(|e| e.to_string())?;

    let readme = pack_root.join("使用說明.txt");
    fs::write(
        readme,
        format!(
            "【自訂字體資源包】\n\
1. 把整個「{}」資料夾放到遊戲的 resourcepacks。\n\
2. 設定 → 資源包 → 啟用它。\n\
3. 建議關閉「強制使用 Unicode 字型」後重開遊戲。\n\
4. 若字太糊或太細，可換另一個字體檔再產生一次。\n",
            name
        ),
    )
    .ok();

    Ok(FontPackResult {
        pack_path: pack_root.display().to_string(),
        player_summary: format!(
            "字體資源包已建立！\n\
• 名稱：{}\n\
• 位置：\n{}\n\n\
• 設定：大小 {:.1}、字重感 {:.0}、位移 ({:.1}, {:.1})、清晰度 {:.1}\n\n\
【怎麼用】\n\
1. 複製到遊戲 resourcepacks\n\
2. 啟用此資源包\n\
3. 關閉「強制使用 Unicode 字型」後重開遊戲",
            name,
            pack_root.display(),
            options.size,
            options.weight,
            options.shift_x,
            options.shift_y,
            options.oversample
        ),
    })
}

pub fn build_font_pack_str_with_options(
    font_path: &str,
    output_dir: &str,
    pack_name: &str,
    pack_desc: &str,
    options: &FontPackOptions,
    pack_format: Option<u16>,
    target_version: Option<&str>,
) -> Result<FontPackResult, String> {
    let font = PathBuf::from(font_path.trim().trim_matches('"'));
    let out = PathBuf::from(output_dir.trim().trim_matches('"'));
    build_font_resource_pack_with_options(
        &font,
        &out,
        pack_name,
        pack_desc,
        options,
        pack_format,
        target_version,
    )
}

pub fn apply_font_pack_to_instance(
    instance_path: &Path,
    font_pack_path: &Path,
) -> Result<FontPackApplyResult, String> {
    if !font_pack_path.exists() {
        return Err("找不到要套用的字體資源包。請先建立字體包。".into());
    }
    if !font_pack_path.is_dir() && !font_pack_path.is_file() {
        return Err("字體資源包路徑必須是資料夾或 .zip 檔。".into());
    }
    let mc = resolve_minecraft_dir(instance_path)?;
    let resourcepacks = mc.join("resourcepacks");
    fs::create_dir_all(&resourcepacks).map_err(|e| format!("無法建立 resourcepacks：{e}"))?;
    let name = font_pack_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "字體資源包名稱無法辨識。".to_string())?;
    let dest = resourcepacks.join(name);
    if same_path(font_pack_path, &dest) {
        return Ok(FontPackApplyResult {
            copied_path: dest.display().to_string(),
            backup_path: None,
            player_summary: format!(
                "字體資源包已經位於目前實例 resourcepacks：\n{}",
                dest.display()
            ),
        });
    }

    let backup_path = if dest.exists() {
        let backup_root = backup_root_for(font_pack_path).join(format!("字體套用備份_{}", backup_stamp()));
        fs::create_dir_all(&backup_root).map_err(|e| format!("無法建立字體備份目錄：{e}"))?;
        let backup_dest = backup_root.join("resourcepacks").join(name);
        if dest.is_dir() {
            copy_dir_recursive(&dest, &backup_dest)?;
            fs::remove_dir_all(&dest).map_err(|e| format!("移除舊字體資源包失敗：{e}"))?;
        } else {
            if let Some(parent) = backup_dest.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(&dest, &backup_dest).map_err(|e| format!("備份舊字體資源包失敗：{e}"))?;
            fs::remove_file(&dest).map_err(|e| format!("移除舊字體資源包失敗：{e}"))?;
        }
        Some(backup_dest.display().to_string())
    } else {
        None
    };

    if font_pack_path.is_dir() {
        copy_dir_recursive(font_pack_path, &dest)?;
    } else {
        fs::copy(font_pack_path, &dest).map_err(|e| format!("複製字體資源包失敗：{e}"))?;
    }

    let backup_line = backup_path
        .as_deref()
        .map(|p| format!("\n• 已備份同名舊資源包：\n{p}"))
        .unwrap_or_else(|| "\n• 目標沒有同名字體包，未建立備份。".to_string());
    Ok(FontPackApplyResult {
        copied_path: dest.display().to_string(),
        backup_path,
        player_summary: format!(
            "字體資源包已套用到目前實例 resourcepacks。\n• 位置：\n{}{}",
            dest.display(),
            backup_line
        ),
    })
}

fn backup_root_for(font_pack_path: &Path) -> PathBuf {
    font_pack_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| {
            font_pack_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."))
        })
}

fn backup_stamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in walkdir::WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path.strip_prefix(src).unwrap_or(path);
        let target = dst.join(rel);
        if path.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else if path.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(path, &target)
                .map_err(|e| format!("複製字體資源包檔案失敗 {}：{e}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        apply_font_pack_to_instance, cjk_font_file_name, font_pack_display_name,
        resolve_font_pack_format, FontPackOptions, DEFAULT_FONT_PACK_FORMAT,
        DEFAULT_FONT_PACK_NAME,
    };
    use std::{fs, path::Path};

    #[test]
    fn font_options_are_clamped_to_safe_renderer_ranges() {
        let options = FontPackOptions {
            size: 99.0,
            weight: -10.0,
            shift_x: f32::NAN,
            shift_y: -99.0,
            oversample: 99.0,
        };
        let normalized = options.normalized();

        assert_eq!(normalized.size, 24.0);
        assert_eq!(normalized.weight, 100.0);
        assert_eq!(normalized.shift_x, 0.0);
        assert_eq!(normalized.shift_y, -3.0);
        assert_eq!(normalized.oversample, 8.0);
    }

    #[test]
    fn cjk_font_preserves_ttf_and_otf_extensions() {
        assert_eq!(
            cjk_font_file_name(Path::new(r"C:\fonts\NotoSans.ttf")).unwrap(),
            "cjk_font.ttf"
        );
        assert_eq!(
            cjk_font_file_name(Path::new("/tmp/SourceHan.OTF")).unwrap(),
            "cjk_font.otf"
        );
    }

    #[test]
    fn cjk_font_rejects_ttc_with_clear_message() {
        let err = cjk_font_file_name(Path::new("mingliu.ttc")).unwrap_err();
        assert!(err.contains(".ttc"), "{err}");
        assert!(err.contains("ttf") || err.contains("otf"), "{err}");
    }

    #[test]
    fn empty_pack_name_uses_font_specific_default() {
        assert_eq!(
            font_pack_display_name("").unwrap(),
            DEFAULT_FONT_PACK_NAME
        );
        assert_eq!(
            font_pack_display_name("   ").unwrap(),
            DEFAULT_FONT_PACK_NAME
        );
        assert_eq!(font_pack_display_name("我的字體").unwrap(), "我的字體");
    }

    #[test]
    fn pack_format_prefers_explicit_then_version_then_default() {
        assert_eq!(resolve_font_pack_format(Some(15), Some("1.21.1")), 15);
        assert_eq!(resolve_font_pack_format(None, Some("1.21.1")), 34);
        assert_eq!(resolve_font_pack_format(None, Some("1.20.1")), 15);
        assert_eq!(
            resolve_font_pack_format(None, None),
            DEFAULT_FONT_PACK_FORMAT
        );
    }

    #[test]
    fn apply_font_pack_backs_up_same_name_pack() {
        let root = std::env::temp_dir().join(format!("font_apply_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let generated = root.join("翻譯結果").join("resourcepacks").join("繁體中文遊戲字體");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(mc.join("resourcepacks/繁體中文遊戲字體")).unwrap();
        fs::create_dir_all(generated.join("assets/minecraft/font")).unwrap();
        fs::write(mc.join("resourcepacks/繁體中文遊戲字體/old.txt"), "old").unwrap();
        fs::write(generated.join("assets/minecraft/font/default.json"), "{}").unwrap();

        let result = apply_font_pack_to_instance(&mc, &generated).unwrap();

        assert!(result.backup_path.is_some());
        assert!(mc
            .join("resourcepacks/繁體中文遊戲字體/assets/minecraft/font/default.json")
            .is_file());
        assert!(Path::new(result.backup_path.as_deref().unwrap()).is_file()
            || Path::new(result.backup_path.as_deref().unwrap()).is_dir());
        let _ = fs::remove_dir_all(root);
    }
}
