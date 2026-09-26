//! 遊戲資料夾裡的隱藏標記 `.mcpl/`（B1 檔案安全設計）。
//!
//! - `.mcpl/instance.json`：整合包識別碼。套用紀錄、備份區、隔離區都以它為鍵——
//!   資料夾改名、搬家、網路磁碟路徑不穩都不影響；CurseForge 把多個整合包放在同一個上層
//!   資料夾時也不會認錯。
//! - `.mcpl/files/<相對路徑>.json`：每個工具寫過的檔一份標記（角色、工具寫入指紋、原檔指紋、
//!   備份位置）。**先寫標記、後動檔案**，所以任何工具寫過的檔都找得到標記。
//!
//! 只把 `.mcpl/` 資料夾本身設成隱藏；裡面的檔案不設隱藏屬性（Windows 會拒絕用覆寫方式
//! 寫入隱藏檔），寫入一律暫存檔＋改名。零散標記不放進 mods/、config/ 這類會被模組掃描的地方。

use std::fs;
use std::path::{Path, PathBuf};

use super::hashutil::sha256_hex;
use super::paths::long_path;

pub const MARKER_DIR: &str = ".mcpl";
const INSTANCE_FILE: &str = "instance.json";
const FILES_DIR: &str = "files";
/// 「把這份當成新的整合包」時，舊標記搬進 `.mcpl/copied-from-<舊識別碼>/` 留作痕跡。
pub const COPIED_FROM_PREFIX: &str = "copied-from-";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceInfo {
    pub id: String,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub tool_version: String,
}

/// 標記之間的關聯（雙向：兩端都要記對方的 id）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub id: String,
    /// 關係："backup"／"gameFile"／"quarantine"／"batch"／"packEntry"／"legacyManifest"
    pub relation: String,
}

pub const REL_BACKUP: &str = "backup";
pub const REL_GAME_FILE: &str = "gameFile";
pub const REL_QUARANTINE: &str = "quarantine";
pub const REL_BATCH: &str = "batch";
pub const REL_PACK_ENTRY: &str = "packEntry";
pub const REL_LEGACY: &str = "legacyManifest";

pub fn link(id: &str, relation: &str) -> Link {
    Link { id: id.to_string(), relation: relation.to_string() }
}

/// 找某種關係的對方 id。
pub fn linked(links: &[Link], relation: &str) -> Option<String> {
    links.iter().find(|l| l.relation == relation).map(|l| l.id.clone())
}

pub fn links_to(links: &[Link], relation: &str, id: &str) -> bool {
    links.iter().any(|l| l.relation == relation && l.id == id)
}

/// 換掉某種關係的對方（一種關係只指向一個對象）。
pub fn set_link(links: &mut Vec<Link>, relation: &str, id: &str) {
    links.retain(|l| l.relation != relation);
    links.push(link(id, relation));
}

/// 一個工具寫過的檔的標記。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMarker {
    /// 標記自己的 id
    #[serde(default)]
    pub id: String,
    pub instance_id: String,
    pub rel: String,
    /// "added"＝工具新增；"overwritten"＝工具覆蓋了原本的檔
    pub role: String,
    /// 工具寫入內容的指紋
    pub tool_sha256: String,
    /// 被覆蓋前原檔的指紋（新增的為空）
    #[serde(default)]
    pub original_sha256: String,
    /// 原檔備份的位置（沒有備份為空）
    #[serde(default)]
    pub backup: String,
    #[serde(default)]
    pub tool_version: String,
    #[serde(default)]
    pub written_at: u64,
    /// "known"＝原本是什麼很清楚；"unknown"＝來源不明（原本的內容已移入隔離區）
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub links: Vec<Link>,
    /// 工具以前寫過的其他版本指紋（中途中斷或重跑時，舊版本也認得是工具寫的）
    #[serde(default)]
    pub tool_history: Vec<String>,
    /// "pending"＝紀錄已寫、檔案還沒寫完（待確認）；"written"＝已寫入
    #[serde(default)]
    pub state: String,
}

impl FileMarker {
    /// 內容指紋等於任一版工具寫入的版本。
    pub fn is_tool_content(&self, sha: Option<&str>) -> bool {
        sha.is_some_and(|sha| self.tool_sha256 == sha || self.tool_history.iter().any(|h| h == sha))
    }
}

/// 把遊戲檔標記標為「已寫入」。
pub fn mark_written(mc: &Path, rel: &str) -> Result<(), String> {
    let Some(mut marker) = read_file_marker(mc, rel) else {
        return Err(format!("寫完檔案後找不到它的標記：{rel}"));
    };
    marker.state = "written".into();
    write_file_marker(mc, &marker)
}

/// 設定檔（options.txt）裡由工具加的資源包清單項目與語言設定的標記：`.mcpl/options.json`。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionsMarker {
    #[serde(default)]
    pub entries: Vec<EntryMarker>,
    #[serde(default)]
    pub lang: Option<LangMarker>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryMarker {
    pub id: String,
    /// 資源包清單裡的項目，例如 `file/繁體中文翻譯.zip`
    pub entry: String,
    /// gameFile＝對應資源包檔的標記；batch＝那次套用
    #[serde(default)]
    pub links: Vec<Link>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LangMarker {
    pub id: String,
    /// 工具改之前的語言；`None`＝原本沒有語言設定
    #[serde(default)]
    pub original_lang: Option<String>,
    #[serde(default)]
    pub links: Vec<Link>,
}

fn options_marker_path(mc: &Path) -> PathBuf {
    marker_dir(mc).join("options.json")
}

pub fn read_options_marker(mc: &Path) -> OptionsMarker {
    fs::read_to_string(long_path(&options_marker_path(mc)))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn write_options_marker(mc: &Path, marker: &OptionsMarker) -> Result<(), String> {
    let path = options_marker_path(mc);
    fs::create_dir_all(long_path(&marker_dir(mc))).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(marker).map_err(|e| e.to_string())?;
    write_atomic(&path, json.as_bytes()).map_err(|e| format!("無法寫入設定標記（{e}）：{}", path.display()))
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn marker_dir(mc: &Path) -> PathBuf {
    mc.join(MARKER_DIR)
}

fn instance_path(mc: &Path) -> PathBuf {
    marker_dir(mc).join(INSTANCE_FILE)
}

/// 讀整合包識別碼；沒有標記回 `Ok(None)`，標記壞掉回錯（不自己重建，以免換掉身分）。
pub fn read_instance(mc: &Path) -> Result<Option<InstanceInfo>, String> {
    let path = instance_path(mc);
    match fs::read_to_string(long_path(&path)) {
        Ok(text) => serde_json::from_str::<InstanceInfo>(&text)
            .ok()
            .filter(|info| !info.id.trim().is_empty())
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "遊戲資料夾裡的工具標記壞了（標記壞了），為了不認錯整合包，已經停止，沒有動任何檔案。\n\
標記位置：{}\n\
修復方法：只刪掉上面這一個檔案，再按一次。工具會用套用紀錄記的遊戲資料夾位置和每個檔案的指紋認回這個整合包；\
認不回來時，舊的紀錄與備份會保留，並在結果裡說明。",
                    path.display()
                )
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "讀不到遊戲資料夾裡的工具標記（{error}），已經停止，沒有動任何檔案。\n標記位置：{}",
            path.display()
        )),
    }
}

/// 新的標記 id。
pub fn new_marker_id() -> String {
    new_id(Path::new("marker"))
}

/// 產生新的識別碼（UUID 格式；不新增依賴，用時間、行程、路徑與計數做雜湊）。
fn new_id(mc: &Path) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seed = format!(
        "{nanos}|{}|{}|{}|{:?}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        mc.display(),
        std::thread::current().id()
    );
    let h = sha256_hex(seed.as_bytes());
    format!("{}-{}-4{}-a{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

/// 建立 `.mcpl/instance.json`（已存在就不動），並把 `.mcpl/` 設為隱藏。回傳識別碼與「是不是這次才建立」。
pub fn create_instance(mc: &Path) -> Result<(InstanceInfo, bool), String> {
    if let Some(info) = read_instance(mc)? {
        return Ok((info, false));
    }
    let info = create_instance_with_id(mc, &new_id(mc))?;
    Ok((info, true))
}

/// 用指定的識別碼建立 `.mcpl/instance.json`（重新配對到舊紀錄時沿用原本的識別碼）。
pub fn create_instance_with_id(mc: &Path, id: &str) -> Result<InstanceInfo, String> {
    let info = InstanceInfo {
        id: id.to_string(),
        created_at: now_secs(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let dir = marker_dir(mc);
    fs::create_dir_all(long_path(&dir))
        .map_err(|e| format!("無法在遊戲資料夾建立工具標記（{e}）：{}", dir.display()))?;
    let json = serde_json::to_string_pretty(&info).map_err(|e| e.to_string())?;
    write_atomic(&instance_path(mc), json.as_bytes())
        .map_err(|e| format!("無法寫入工具標記（{e}）：{}", instance_path(mc).display()))?;
    set_hidden(&dir);
    Ok(info)
}

fn file_marker_path(mc: &Path, rel: &str) -> PathBuf {
    marker_dir(mc).join(FILES_DIR).join(format!("{rel}.json"))
}

pub fn read_file_marker(mc: &Path, rel: &str) -> Option<FileMarker> {
    let text = fs::read_to_string(long_path(&file_marker_path(mc, rel))).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_file_marker(mc: &Path, marker: &FileMarker) -> Result<(), String> {
    let path = file_marker_path(mc, &marker.rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(long_path(parent)).map_err(|e| format!("無法寫入檔案標記（{e}）：{}", path.display()))?;
    }
    let json = serde_json::to_string_pretty(marker).map_err(|e| e.to_string())?;
    write_atomic(&path, json.as_bytes()).map_err(|e| format!("無法寫入檔案標記（{e}）：{}", path.display()))
}

pub fn remove_file_marker(mc: &Path, rel: &str) {
    let _ = fs::remove_file(long_path(&file_marker_path(mc, rel)));
}

/// 全部檔案標記（紀錄遺失時用來重建）。
pub fn all_file_markers(mc: &Path) -> Vec<FileMarker> {
    let root = marker_dir(mc).join(FILES_DIR);
    let long_root = long_path(&root);
    if !long_root.is_dir() {
        return Vec::new();
    }
    walkdir::WalkDir::new(&long_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| fs::read_to_string(e.path()).ok())
        .filter_map(|text| serde_json::from_str::<FileMarker>(&text).ok())
        .collect()
}

fn write_atomic(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = dest.with_file_name(format!("{name}.mcpl-tmp"));
    fs::write(long_path(&tmp), bytes)?;
    fs::rename(long_path(&tmp), long_path(dest)).inspect_err(|_| {
        let _ = fs::remove_file(long_path(&tmp));
    })
}

/// 只把資料夾設為隱藏。失敗只記錄，不中止（隱藏只是為了不打擾玩家）。
#[cfg(windows)]
fn set_hidden(dir: &Path) {
    use std::os::windows::ffi::OsStrExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileAttributesW(lpFileName: *const u16) -> u32;
        fn SetFileAttributesW(lpFileName: *const u16, dwFileAttributes: u32) -> i32;
    }
    let wide: Vec<u16> = long_path(dir).as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let current = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if current == INVALID_FILE_ATTRIBUTES {
        crate::dev_log!("apply", "無法讀取 .mcpl 屬性，略過設為隱藏：{}", dir.display());
        return;
    }
    if current & FILE_ATTRIBUTE_HIDDEN != 0 {
        return;
    }
    let ok = unsafe { SetFileAttributesW(wide.as_ptr(), current | FILE_ATTRIBUTE_HIDDEN) };
    if ok == 0 {
        crate::dev_log!("apply", "無法把 .mcpl 設為隱藏（不影響功能）：{}", dir.display());
    }
}

#[cfg(not(windows))]
fn set_hidden(_dir: &Path) {}

#[cfg(test)]
pub fn is_hidden(dir: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        fs::metadata(dir).map(|m| m.file_attributes() & 0x2 != 0).unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_marker_is_created_once_and_folder_is_hidden() {
        let root = std::env::temp_dir().join(format!("mcpl-marker-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        fs::create_dir_all(&mc).unwrap();
        let (first, created) = create_instance(&mc).unwrap();
        assert!(created);
        let (second, created_again) = create_instance(&mc).unwrap();
        assert!(!created_again);
        assert_eq!(first.id, second.id, "識別碼建立後不會變");
        assert!(is_hidden(&marker_dir(&mc)), ".mcpl 資料夾要設為隱藏");
        // 裡面的檔案不設隱藏，之後仍能用暫存檔＋改名覆寫
        let marker = FileMarker {
            id: new_marker_id(),
            origin: "known".into(),
            links: vec![link("b1", REL_BATCH)],
            tool_history: Vec::new(),
            state: "written".into(),
            instance_id: first.id.clone(),
            rel: "config/a b/c.txt".into(),
            role: "added".into(),
            tool_sha256: "x".into(),
            original_sha256: String::new(),
            backup: String::new(),
            tool_version: String::new(),
            written_at: 0,
        };
        write_file_marker(&mc, &marker).unwrap();
        write_file_marker(&mc, &marker).unwrap();
        assert_eq!(read_file_marker(&mc, "config/a b/c.txt"), Some(marker));
        assert_eq!(all_file_markers(&mc).len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn two_folders_get_different_ids() {
        let root = std::env::temp_dir().join(format!("mcpl-marker-two-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        let (a, _) = create_instance(&root.join("a")).unwrap();
        let (b, _) = create_instance(&root.join("b")).unwrap();
        assert_ne!(a.id, b.id);
        let _ = fs::remove_dir_all(root);
    }
}
