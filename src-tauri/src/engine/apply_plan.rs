//! 套用清單：翻譯結果裡「哪個檔要放到遊戲資料夾的哪裡」。只讀、不寫。
//!
//! 先把全部目標列出來，套用本體才能在第一次寫入前做完所有檢查（遊戲開著、沒有
//! options.txt、還沒選備份、不備份時要覆蓋原檔），任何一項不過就一個檔都不動。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::out_layout::ResultLayout;
use super::paths::long_path;
use super::session::is_tool_resource_pack;

#[derive(Debug, Clone)]
pub enum ItemSource {
    File(PathBuf),
    /// 工具當場產生的小檔（例如資源包旁的指紋標記）
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Zip,
    ZipMeta,
    ResourcepackOverlays,
    Config,
    Minemenu,
    Patchouli,
    Kubejs,
    Datapacks,
    Defaultconfigs,
    GlobalPacks,
    Paxi,
    Mods,
    /// 遊戲資料夾裡的 data/、assets/、guideme/、hqm/ 與資料夾型資源包的文字覆寫
    GameTextOverlay,
}

#[derive(Debug, Clone)]
pub struct PlanItem {
    pub source: ItemSource,
    pub dest: PathBuf,
    pub group: Group,
}

#[derive(Debug, Clone, Default)]
pub struct ApplyPlan {
    pub items: Vec<PlanItem>,
    /// 主翻譯資源包在遊戲 resourcepacks 裡的檔名
    pub zip_name: Option<String>,
    index: HashMap<PathBuf, usize>,
}

impl ApplyPlan {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    #[cfg(test)]
    pub fn targets(&self) -> Vec<PathBuf> {
        self.items.iter().map(|item| item.dest.clone()).collect()
    }

    pub fn count(&self, group: Group) -> usize {
        self.items.iter().filter(|item| item.group == group).count()
    }

    pub fn has(&self, group: Group) -> bool {
        self.count(group) > 0
    }

    fn push(&mut self, source: ItemSource, dest: PathBuf, group: Group) {
        // 同一個目標只寫一次，以後加入的為準
        let item = PlanItem { source, dest: dest.clone(), group };
        if let Some(&at) = self.index.get(&dest) {
            self.items[at] = item;
        } else {
            self.index.insert(dest, self.items.len());
            self.items.push(item);
        }
    }

    fn push_tree(&mut self, source_root: &Path, dest_root: &Path, group: Group) {
        for (file, rel) in files_under(source_root) {
            self.push(ItemSource::File(file), dest_root.join(rel), group);
        }
    }
}

/// 列出資料夾底下所有檔案（含相對路徑）。長路徑也讀得到。
fn files_under(root: &Path) -> Vec<(PathBuf, PathBuf)> {
    let long_root = long_path(root);
    if !long_root.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(&long_root)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(&long_root) else {
            continue;
        };
        out.push((root.join(rel), rel.to_path_buf()));
    }
    out
}

/// 建立套用清單。`zip_src`＝要放進遊戲的主翻譯資源包；`zip_meta`＝旁邊的指紋標記內容。
pub fn build_plan(
    mc: &Path,
    layout: &ResultLayout,
    pack_name: &str,
    zip_src: Option<&Path>,
    zip_meta: Option<Vec<u8>>,
) -> ApplyPlan {
    let work = &layout.work_root;
    let mut plan = ApplyPlan::default();
    let resourcepacks = mc.join("resourcepacks");

    if let Some(zip) = zip_src {
        if let Some(name) = zip.file_name().and_then(|s| s.to_str()) {
            plan.push(ItemSource::File(zip.to_path_buf()), resourcepacks.join(name), Group::Zip);
            plan.zip_name = Some(name.to_string());
            if let Some(meta) = zip_meta {
                let meta_name = format!("{}.meta.json", name.trim_end_matches(".zip"));
                plan.push(ItemSource::Bytes(meta), resourcepacks.join(meta_name), Group::ZipMeta);
            }
        }
    }
    plan.push_tree(&work.join("resourcepacks-extra"), &resourcepacks, Group::ResourcepackOverlays);
    plan.push_tree(&work.join("config"), &mc.join("config"), Group::Config);
    let menu = work.join("minemenu").join("menu.json");
    if long_path(&menu).is_file() {
        plan.push(ItemSource::File(menu), mc.join("minemenu").join("menu.json"), Group::Minemenu);
    }
    for (name, group) in [
        ("patchouli_books", Group::Patchouli),
        ("kubejs", Group::Kubejs),
        ("datapacks", Group::Datapacks),
        ("defaultconfigs", Group::Defaultconfigs),
        ("global_packs", Group::GlobalPacks),
        ("paxi", Group::Paxi),
    ] {
        plan.push_tree(&work.join(name), &mc.join(name), group);
    }
    plan.push_tree(&work.join("jar-translated"), &mc.join("mods"), Group::Mods);
    let zip_name = plan.zip_name.clone();
    push_game_text_overlays(&mut plan, mc, work, pack_name, zip_name.as_deref());
    plan
}

/// 翻譯遊戲資料夾裡 data/、assets/、guideme/、hqm/ 與資料夾型資源包的文字時，
/// 輸出放在翻譯結果的同名相對位置。這裡只收「遊戲裡本來就有對應來源」的檔：
/// 翻譯結果的 data/ 也放了 JAR 書本的除錯副本，那些不屬於遊戲資料夾，不能搬進去。
fn push_game_text_overlays(
    plan: &mut ApplyPlan,
    mc: &Path,
    work: &Path,
    pack_name: &str,
    zip_name: Option<&str>,
) {
    for root in ["data", "assets", "guideme", "hqm"] {
        let game_root = mc.join(root);
        if !long_path(&game_root).is_dir() {
            continue;
        }
        for (file, rel) in files_under(&work.join(root)) {
            if game_has_source(&game_root, &rel) {
                plan.push(ItemSource::File(file), game_root.join(&rel), Group::GameTextOverlay);
            }
        }
    }

    // 資料夾型資源包：翻譯結果的 resourcepacks/ 也放著工具自己的包，要排除
    let own_stems: Vec<String> = [Some(pack_name), zip_name.map(|z| z.trim_end_matches(".zip"))]
        .into_iter()
        .flatten()
        .map(|s| s.trim().to_lowercase())
        .collect();
    let work_packs = work.join("resourcepacks");
    let Ok(entries) = std::fs::read_dir(long_path(&work_packs)) else {
        return;
    };
    let mut folders: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    folders.sort();
    for folder in folders {
        if own_stems.contains(&folder.to_lowercase()) || is_tool_resource_pack(&folder) {
            continue;
        }
        let game_pack = mc.join("resourcepacks").join(&folder);
        if !long_path(&game_pack).is_dir() {
            continue;
        }
        for (file, rel) in files_under(&work_packs.join(&folder)) {
            if game_has_source(&game_pack, &rel) {
                plan.push(ItemSource::File(file), game_pack.join(&rel), Group::GameTextOverlay);
            }
        }
    }
}

/// 翻譯檔在遊戲裡有沒有對應的來源：同路徑的原檔，或（新寫出的 zh_tw 版本）
/// 同一本書／同一組語言檔的上層資料夾已經存在。
fn game_has_source(game_root: &Path, rel: &Path) -> bool {
    if long_path(&game_root.join(rel)).is_file() {
        return true;
    }
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    for (index, part) in parts.iter().enumerate() {
        if !part.to_ascii_lowercase().contains("zh_tw") || index == 0 {
            continue;
        }
        let parent: PathBuf = parts[..index].iter().collect();
        if long_path(&game_root.join(parent)).is_dir() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn layout_for(work: &Path) -> ResultLayout {
        ResultLayout {
            user_base: work.to_path_buf(),
            work_root: work.to_path_buf(),
            resourcepacks: work.join("resourcepacks"),
            config: work.join("config"),
            minemenu: work.join("minemenu"),
        }
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn plan_includes_game_data_assets_guideme_hqm_and_folder_packs() {
        let root = std::env::temp_dir().join(format!("mcpl-plan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let work = root.join("翻譯結果");
        // 遊戲裡的來源
        write(&mc.join("data/pack/quests/q.json"), "en");
        write(&mc.join("assets/ex/books/b/en_us/p.json"), "en");
        write(&mc.join("guideme/ae2/index.md"), "en");
        write(&mc.join("hqm/quests.json"), "en");
        write(&mc.join("resourcepacks/FolderPack/assets/ns/lang/en_us.json"), "{}");
        // 翻譯結果
        write(&work.join("data/pack/quests/q.json"), "中");
        write(&work.join("assets/ex/books/b/zh_tw/p.json"), "中");
        write(&work.join("guideme/ae2/index.md"), "中");
        write(&work.join("hqm/quests.json"), "中");
        write(&work.join("resourcepacks/FolderPack/assets/ns/lang/zh_tw.json"), "{}");
        // 工具自己的包資料夾與 JAR 書本除錯副本不能搬進遊戲
        write(&work.join("resourcepacks/繁體中文翻譯/assets/ns/lang/zh_tw.json"), "{}");
        write(&work.join("data/fromjar/patchouli_books/x/zh_tw/e.json"), "中");

        let plan = build_plan(&mc, &layout_for(&work), "繁體中文翻譯", None, None);
        let targets: Vec<String> = plan
            .targets()
            .iter()
            .map(|p| p.strip_prefix(&mc).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        for expected in [
            "data/pack/quests/q.json",
            "assets/ex/books/b/zh_tw/p.json",
            "guideme/ae2/index.md",
            "hqm/quests.json",
            "resourcepacks/FolderPack/assets/ns/lang/zh_tw.json",
        ] {
            assert!(targets.contains(&expected.to_string()), "缺 {expected}：{targets:?}");
        }
        assert!(!targets.iter().any(|t| t.contains("繁體中文翻譯")), "{targets:?}");
        assert!(!targets.iter().any(|t| t.contains("fromjar")), "{targets:?}");
        let _ = fs::remove_dir_all(root);
    }
}
