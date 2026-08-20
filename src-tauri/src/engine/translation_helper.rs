//! 遊戲內補充翻譯輔助模組。
//!
//! 這裡只處理「確實需要遊戲載入後才看得到」的 FTB Quests 匯出流程。
//! 一次只準備一個相容版本；工具自己下載的檔案會留下狀態，完成補充掃描後只刪除
//! 自己建立的那一個，不碰玩家原本安裝的模組。

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::jar_scan::resolve_minecraft_dir;
use super::pack_out::detect_minecraft_version;
use super::security::sanitize_folder_name;

const STATE_FILE: &str = "輔助翻譯模組狀態.json";
const MAX_HELPER_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationHelperStatus {
    pub needed: bool,
    pub supported: bool,
    pub state: String,
    pub helper_name: String,
    pub minecraft_version: String,
    pub loader: String,
    pub command: String,
    pub message: String,
    pub source_url: String,
    pub mod_path: Option<String>,
    pub installed_by_tool: bool,
    pub changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelperState {
    instance_path: String,
    mod_path: String,
    helper_id: String,
    installed_by_tool: bool,
}

#[derive(Debug, Clone, Copy)]
struct HelperSpec {
    id: &'static str,
    name: &'static str,
    project: &'static str,
    command: &'static str,
    source_url: &'static str,
    markers: &'static [&'static str],
}

const FTB_QUEST_LOCALIZER: HelperSpec = HelperSpec {
    id: "ftb-quest-localizer",
    name: "FTB Quest Localizer",
    project: "ftb-quest-localizer",
    command: "/ftblang export en_us",
    source_url: "https://modrinth.com/mod/ftb-quest-localizer",
    markers: &["ftbquestlocalizer", "ftb-quest-localizer"],
};

const FTB_QUEST_PRECISION_LOCALIZER: HelperSpec = HelperSpec {
    id: "ftb-quests-precision-localizer",
    name: "FTB Quests Precision Localizer",
    project: "ftb-quests-precision-localizer",
    command: "/ftblang en_us ftbquests normal",
    source_url: "https://modrinth.com/mod/ftb-quests-precision-localizer",
    markers: &["ftbquestsprecisionlocalizer", "ftb-quests-precision-localizer"],
};

pub fn inspect_translation_helper(
    instance_or_minecraft: &Path,
    output_dir: Option<&Path>,
) -> Result<TranslationHelperStatus, String> {
    let minecraft_dir = resolve_minecraft_dir(instance_or_minecraft)?;
    let version = detect_minecraft_version(&minecraft_dir).unwrap_or_default();
    let loader = detect_loader(instance_or_minecraft, &minecraft_dir);
    if let Some(state) = read_owned_state(output_dir) {
        let path = PathBuf::from(&state.mod_path);
        if state.installed_by_tool
            && path.is_file()
            && same_minecraft_target(&state.instance_path, &minecraft_dir)
            && is_owned_mod_path(&minecraft_dir, &path)
        {
            let spec = helper_spec(&state.helper_id);
            return Ok(status(
                true,
                spec.is_some(),
                "installed",
                spec.map(|item| item.name).unwrap_or("已下載的輔助模組"),
                &version,
                &loader,
                spec.map(|item| item.command).unwrap_or(""),
                "工具先前準備過一個輔助模組；完成翻譯後會自動清理，也可以現在手動清理。",
                spec.map(|item| item.source_url).unwrap_or(""),
                Some(path),
                true,
                false,
            ));
        }
    }
    if !has_ftb_quests(&minecraft_dir) {
        return Ok(status(
            false,
            false,
            "not-needed",
            "",
            "",
            "",
            "",
            "這個整合包沒有偵測到 FTB Quests，不需要額外輔助模組。",
            "",
            None,
            false,
            false,
        ));
    }

    
    if let Some(state) = read_owned_state(output_dir) {
        let path = PathBuf::from(&state.mod_path);
        if path.is_file() && same_path(&state.instance_path, instance_or_minecraft) && state.installed_by_tool {
            let spec = helper_spec(&state.helper_id);
            return Ok(status(
                true,
                spec.is_some(),
                "installed",
                spec.map(|item| item.name).unwrap_or("遊戲內任務翻譯輔助"),
                &version,
                &loader,
                spec.map(|item| item.command).unwrap_or(""),
                if spec.is_some() {
                    "輔助模組已準備好。請啟動一次遊戲，執行下方指令，關閉遊戲後回來：勾選確認再按「③ 重新翻譯任務文字」。"
                } else {
                    "這個輔助模組版本目前已不在支援清單，但它是工具先前下載的檔案；可以直接清理。"
                },
                spec.map(|item| item.source_url).unwrap_or(""),
                Some(path),
                true,
                false,
            ));
        }
    }

    let Some(spec) = choose_helper(&version, &loader) else {
        return Ok(status(
            true,
            false,
            "unsupported",
            "",
            &version,
            &loader,
            "",
            &format!(
                "FTB Quests 已找到，但目前沒有適合 Minecraft {version}／{loader} 的單一輔助模組，已略過。工具仍會使用內建掃描。"
            ),
            "https://modrinth.com/mod/ftb-quest-localizer",
            None,
            false,
            false,
        ));
    };

    if let Some(state) = read_owned_state(output_dir) {
        let path = PathBuf::from(&state.mod_path);
        if path.is_file() && same_minecraft_target(&state.instance_path, &minecraft_dir) {
            return Ok(status(
                true,
                true,
                "installed",
                spec.name,
                &version,
                &loader,
                spec.command,
            "輔助模組已準備好。請啟動一次遊戲，執行下方指令，關閉遊戲後回來：勾選確認再按「③ 重新翻譯任務文字」。",
                spec.source_url,
                Some(path),
                true,
                false,
            ));
        }
    }

    if let Some(existing) = find_existing_helper(&minecraft_dir, spec) {
        return Ok(status(
            true,
            true,
            "existing",
            spec.name,
            &version,
            &loader,
            spec.command,
            "已找到相容的輔助模組，不會重複下載。請啟動一次遊戲，執行下方指令，關閉遊戲後回來：勾選確認再按「③ 重新翻譯任務文字」。",
            spec.source_url,
            Some(existing),
            false,
            false,
        ));
    }

    Ok(status(
        true,
        true,
        "available",
        spec.name,
        &version,
        &loader,
        spec.command,
        "這是選用的補充步驟，不會阻擋一般翻譯。需要更多 FTB 任務文字時，工具會準備一個相容版本。",
        spec.source_url,
        None,
        false,
        false,
    ))
}

pub fn prepare_translation_helper(
    instance_or_minecraft: &Path,
    output_dir: &Path,
) -> Result<TranslationHelperStatus, String> {
    let before = inspect_translation_helper(instance_or_minecraft, Some(output_dir))?;
    if before.state != "available" {
        return Ok(before);
    }

    let minecraft_dir = resolve_minecraft_dir(instance_or_minecraft)?;
    let version = before.minecraft_version.clone();
    let loader = before.loader.clone();
    let spec = choose_helper(&version, &loader).ok_or_else(|| before.message.clone())?;
    let (filename, url) = fetch_modrinth_file(spec, &version, &loader)?;
    let mods_dir = minecraft_dir.join("mods");
    fs::create_dir_all(&mods_dir).map_err(|e| format!("無法建立 mods 資料夾：{e}"))?;
    let filename = safe_jar_name(&filename)?;
    let target = mods_dir.join(filename);
    if target.exists() {
        return Err("目標 mods 已有同名檔案，為避免覆蓋玩家檔案，已停止準備。".into());
    }

    let temporary = target.with_extension("jar.download");
    fs::write(&temporary, download_bytes(&url)?)
        .map_err(|e| format!("無法寫入輔助模組：{e}"))?;
    fs::rename(&temporary, &target).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        format!("無法放入輔助模組：{e}")
    })?;

    write_owned_state(
        output_dir,
        &HelperState {
            instance_path: instance_or_minecraft.display().to_string(),
            mod_path: target.display().to_string(),
            helper_id: spec.id.to_string(),
            installed_by_tool: true,
        },
    )?;

    Ok(status(
        true,
        true,
        "installed",
        spec.name,
        &version,
        &loader,
        spec.command,
        "已準備好。請啟動一次遊戲，執行下方指令，關閉遊戲後回來：勾選確認再按「③ 重新翻譯任務文字」。",
        spec.source_url,
        Some(target),
        true,
        true,
    ))
}

pub fn cleanup_translation_helper(
    instance_or_minecraft: &Path,
    output_dir: &Path,
) -> Result<TranslationHelperStatus, String> {
    let minecraft_dir = resolve_minecraft_dir(instance_or_minecraft)?;
    let mut removed = false;
    for state_path in state_paths(output_dir) {
        let Some(state) = read_state_file(&state_path) else {
            continue;
        };
        if state.installed_by_tool && is_owned_mod_path(&minecraft_dir, Path::new(&state.mod_path)) {
            if Path::new(&state.mod_path).is_file() {
                fs::remove_file(&state.mod_path)
                    .map_err(|e| format!("無法清理工具下載的輔助模組：{e}"))?;
                removed = true;
            }
        }
        let _ = fs::remove_file(state_path);
    }

    let mut result = inspect_translation_helper(instance_or_minecraft, Some(output_dir))?;
    result.changed = removed;
    result.message = if removed {
        "補充翻譯完成，已刪除工具暫時加入的輔助模組。玩家原本安裝的模組不會被刪除。".into()
    } else {
        "補充翻譯完成，沒有需要由工具清理的輔助模組。".into()
    };
    if result.state == "installed" {
        result.state = "existing".into();
        result.installed_by_tool = false;
    }
    Ok(result)
}

fn status(
    needed: bool,
    supported: bool,
    state: &str,
    helper_name: &str,
    version: &str,
    loader: &str,
    command: &str,
    message: &str,
    source_url: &str,
    mod_path: Option<PathBuf>,
    installed_by_tool: bool,
    changed: bool,
) -> TranslationHelperStatus {
    TranslationHelperStatus {
        needed,
        supported,
        state: state.into(),
        helper_name: helper_name.into(),
        minecraft_version: version.into(),
        loader: loader.into(),
        command: command.into(),
        message: message.into(),
        source_url: source_url.into(),
        mod_path: mod_path.map(|p| p.display().to_string()),
        installed_by_tool,
        changed,
    }
}

fn choose_helper(version: &str, loader: &str) -> Option<HelperSpec> {
    let version = version.split('-').next().unwrap_or(version);
    let loader = loader.to_ascii_lowercase();
    if matches!(version, "1.18.2" | "1.19.2" | "1.20.1" | "1.20.4")
        && matches!(loader.as_str(), "forge" | "neoforge")
    {
        return Some(FTB_QUEST_LOCALIZER);
    }
    if matches!(version, "1.20.2" | "1.20.3") && loader == "forge" {
        return Some(FTB_QUEST_PRECISION_LOCALIZER);
    }
    None
}

fn helper_spec(id: &str) -> Option<HelperSpec> {
    match id {
        "ftb-quest-localizer" => Some(FTB_QUEST_LOCALIZER),
        "ftb-quests-precision-localizer" => Some(FTB_QUEST_PRECISION_LOCALIZER),
        _ => None,
    }
}

fn same_path(saved: &str, current: &Path) -> bool {
    let saved = fs::canonicalize(saved).unwrap_or_else(|_| PathBuf::from(saved));
    let current = fs::canonicalize(current).unwrap_or_else(|_| current.to_path_buf());
    saved.to_string_lossy().eq_ignore_ascii_case(&current.to_string_lossy())
}

fn same_minecraft_target(saved: &str, current_minecraft: &Path) -> bool {
    let saved_path = Path::new(saved);
    if let Ok(saved_minecraft) = resolve_minecraft_dir(saved_path) {
        return same_path(&saved_minecraft.to_string_lossy(), current_minecraft);
    }
    same_path(saved, current_minecraft)
}

fn has_ftb_quests(minecraft_dir: &Path) -> bool {
    if minecraft_dir.join("config").join("ftbquests").is_dir() {
        return true;
    }
    let mods = minecraft_dir.join("mods");
    fs::read_dir(mods)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .contains("ftbquests")
        })
}

fn detect_loader(instance: &Path, minecraft_dir: &Path) -> String {
    let mut roots = vec![instance.to_path_buf(), minecraft_dir.to_path_buf()];
    if let Some(parent) = instance.parent() {
        roots.push(parent.to_path_buf());
    }
    for root in roots {
        for filename in ["mmc-pack.json", "minecraftinstance.json", "profile.json", "instance.json"] {
            if let Ok(text) = fs::read_to_string(root.join(filename)) {
                let lower = text.to_ascii_lowercase();
                if lower.contains("neoforge") {
                    return "neoforge".into();
                }
                if lower.contains("forge") {
                    return "forge".into();
                }
                if lower.contains("fabric") {
                    return "fabric".into();
                }
            }
        }
    }
    let lower_names = fs::read_dir(minecraft_dir.join("mods"))
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase())
        .collect::<Vec<_>>();
    if lower_names.iter().any(|name| name.contains("neoforge")) {
        return "neoforge".into();
    }
    if lower_names.iter().any(|name| name.contains("forge")) {
        return "forge".into();
    }
    if lower_names.iter().any(|name| name.contains("fabric")) {
        return "fabric".into();
    }
    "未知載入器".into()
}

fn find_existing_helper(minecraft_dir: &Path, spec: HelperSpec) -> Option<PathBuf> {
    let mods = minecraft_dir.join("mods");
    let entries = fs::read_dir(mods).ok()?;
    entries.flatten().map(|entry| entry.path()).find(|path| {
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jar"))
            && {
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
                spec.markers.iter().any(|marker| name.contains(marker))
            }
    })
}

fn fetch_modrinth_file(spec: HelperSpec, version: &str, loader: &str) -> Result<(String, String), String> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .user_agent("ZeitFrei-ModpackTranslator/1.0")
        .build()
        .map_err(|e| format!("無法準備下載連線：{e}"))?;
    let url = format!("https://api.modrinth.com/v2/project/{}/version", spec.project);
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("找不到相容輔助模組版本：{e}"))?;
    if !response.status().is_success() {
        return Err(format!("輔助模組版本查詢失敗（HTTP {}）。", response.status()));
    }
    let versions: Vec<Value> = response
        .json()
        .map_err(|e| format!("輔助模組版本資料無法讀取：{e}"))?;
    let wanted_version = version.split('-').next().unwrap_or(version);
    let wanted_loader = loader.to_ascii_lowercase();
    for release in versions {
        let game_versions = release
            .get("game_versions")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        let loaders = release
            .get("loaders")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        if !game_versions.iter().any(|item| *item == wanted_version)
            || !loaders.iter().any(|item| item.eq_ignore_ascii_case(&wanted_loader))
        {
            continue;
        }
        let files = release.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
        let file = files
            .iter()
            .find(|item| item.get("primary").and_then(Value::as_bool) == Some(true))
            .or_else(|| files.first())
            .ok_or_else(|| "相容版本沒有可下載的 JAR。".to_string())?;
        let filename = file.get("filename").and_then(Value::as_str).unwrap_or("");
        let url = file.get("url").and_then(Value::as_str).unwrap_or("");
        if filename.ends_with(".jar") && url.starts_with("https://cdn.modrinth.com/") {
            return Ok((filename.into(), url.into()));
        }
    }
    Err(format!("找不到 Minecraft {wanted_version}／{wanted_loader} 的相容版本。"))
}

fn download_bytes(url: &str) -> Result<Vec<u8>, String> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .user_agent("ZeitFrei-ModpackTranslator/1.0")
        .build()
        .map_err(|e| format!("無法準備下載連線：{e}"))?;
    let response = client.get(url).send().map_err(|e| format!("下載輔助模組失敗：{e}"))?;
    if !response.status().is_success() {
        return Err(format!("下載輔助模組失敗（HTTP {}）。", response.status()));
    }
    if response.content_length().is_some_and(|length| length > MAX_HELPER_BYTES) {
        return Err("輔助模組檔案超過安全大小上限。".into());
    }
    let bytes = response.bytes().map_err(|e| format!("讀取輔助模組失敗：{e}"))?;
    if bytes.len() as u64 > MAX_HELPER_BYTES {
        return Err("輔助模組檔案超過安全大小上限。".into());
    }
    Ok(bytes.to_vec())
}

fn safe_jar_name(name: &str) -> Result<String, String> {
    let filename = Path::new(name).file_name().and_then(|value| value.to_str()).unwrap_or("");
    if filename.is_empty() || filename != name || !filename.ends_with(".jar") || filename.contains("..") {
        return Err("下載檔案名稱不安全，已停止安裝。".into());
    }
    sanitize_folder_name(filename.strip_suffix(".jar").unwrap_or(filename))?;
    Ok(filename.into())
}

fn is_owned_mod_path(minecraft_dir: &Path, candidate: &Path) -> bool {
    let mods = fs::canonicalize(minecraft_dir.join("mods")).ok();
    let path = fs::canonicalize(candidate).ok();
    match (mods, path) {
        (Some(mods), Some(path)) => path.starts_with(mods) && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("jar")),
        _ => false,
    }
}

fn app_data_root() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("modpack-i18n-tool"))
}

fn helper_state_path(output_dir: &Path) -> PathBuf {
    let key = output_dir
        .to_string_lossy()
        .replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], "_");
    app_data_root()
        .unwrap_or_else(|| output_dir.to_path_buf())
        .join("helper-state")
        .join(format!("{key}.json"))
}

fn state_paths(output_dir: &Path) -> Vec<PathBuf> {
    let mut paths = vec![helper_state_path(output_dir)];
    // 舊版曾寫在結果目錄；仍讀取以便遷移
    paths.push(output_dir.join(STATE_FILE));
    if output_dir.file_name().and_then(|name| name.to_str()) != Some("翻譯結果") {
        paths.push(output_dir.join("翻譯結果").join(STATE_FILE));
    }
    paths
}

fn read_owned_state(output_dir: Option<&Path>) -> Option<HelperState> {
    let output_dir = output_dir?;
    state_paths(output_dir).into_iter().find_map(|path| read_state_file(&path))
}

fn read_state_file(path: &Path) -> Option<HelperState> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_owned_state(output_dir: &Path, state: &HelperState) -> Result<(), String> {
    let path = helper_state_path(output_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("無法建立狀態資料夾：{e}"))?;
    }
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("無法保存輔助模組狀態：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_ftb_localizer_for_supported_forge_and_neoforge() {
        assert_eq!(choose_helper("1.20.1", "forge").unwrap().id, FTB_QUEST_LOCALIZER.id);
        assert_eq!(choose_helper("1.20.4", "neoforge").unwrap().id, FTB_QUEST_LOCALIZER.id);
    }

    #[test]
    fn chooses_precision_localizer_only_for_the_gap_versions() {
        assert_eq!(choose_helper("1.20.2", "forge").unwrap().id, FTB_QUEST_PRECISION_LOCALIZER.id);
        assert!(choose_helper("1.20.2", "neoforge").is_none());
    }

    #[test]
    fn refuses_unknown_or_unsupported_loader() {
        assert!(choose_helper("1.21.1", "forge").is_none());
        assert!(choose_helper("1.20.1", "fabric").is_none());
    }

    #[test]
    fn rejects_unsafe_download_names() {
        assert!(safe_jar_name("../helper.jar").is_err());
        assert!(safe_jar_name("helper.zip").is_err());
        assert_eq!(safe_jar_name("helper-1.0.jar").unwrap(), "helper-1.0.jar");
    }
}
