//! 由使用者字體檔建立 Minecraft 字體資源包（人性化、固定安全路徑）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use super::security::{check_font_file, ensure_under_base, sanitize_folder_name};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontPackResult {
    pub pack_path: String,
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
            if value.is_finite() { value } else { fallback }
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

pub fn build_font_resource_pack_with_options(
    font_file: &Path,
    output_dir: &Path,
    pack_name: &str,
    pack_desc: &str,
    options: &FontPackOptions,
) -> Result<FontPackResult, String> {
    check_font_file(font_file)?;
    let name = sanitize_folder_name(pack_name)?;
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
    let _ = ensure_under_base(&work, &pack_root);

    if pack_root.exists() {
        fs::remove_dir_all(&pack_root).map_err(|e| e.to_string())?;
    }

    let font_dir = pack_root
        .join("assets")
        .join("minecraft")
        .join("font")
        .join("include");
    fs::create_dir_all(&font_dir).map_err(|e| e.to_string())?;

    let dest_font = font_dir.join("cjk_font.ttf");
    fs::copy(font_file, &dest_font).map_err(|e| format!("複製字體失敗：{e}"))?;

    let prov = serde_json::json!({
        "providers": [{
            "type": "ttf",
            "file": "minecraft:include/cjk_font.ttf",
            "shift": [options.shift_x, options.shift_y],
            "size": options.provider_size(),
            "oversample": options.oversample
        }]
    });
    let font_json = serde_json::to_string_pretty(&prov).unwrap() + "\n";
    let font_base = pack_root.join("assets").join("minecraft").join("font");
    fs::write(font_base.join("default.json"), &font_json).map_err(|e| e.to_string())?;
    fs::write(font_base.join("uniform.json"), &font_json).map_err(|e| e.to_string())?;

    let meta = serde_json::json!({
        "pack": {
            "pack_format": 15,
            "description": desc
        }
    });
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
) -> Result<FontPackResult, String> {
    let font = PathBuf::from(font_path.trim().trim_matches('"'));
    let out = PathBuf::from(output_dir.trim().trim_matches('"'));
    build_font_resource_pack_with_options(&font, &out, pack_name, pack_desc, options)
}

#[cfg(test)]
mod tests {
    use super::FontPackOptions;

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
}
