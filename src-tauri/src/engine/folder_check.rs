//! B5d 選資料夾就判定：選好資料夾的當下就說清楚能不能翻、為什麼、下一步是什麼。
//!
//! 這裡的每一個判定都**只讀**（G1.36）：不建 `.mcpl`、不寫測試檔、不改工具紀錄。
//! - 寫入檢查：Windows 用「以新增檔案／子資料夾的權限開啟資料夾」做存取檢查，不真的建檔；
//!   套用時的實寫探測（disk::probe_writable）另外依同一套分類說原因。
//! - 可能碰到網路位置的探測都放背景並設上限（[`INSPECT_TIMEOUT`]），逾時就說「連不到」，不卡畫面。
//! - 錯誤依 io::ErrorKind／os error 分類（[`WriteIssue`]），不再比對翻好的中文字串；
//!   只有「本機磁碟＋系統保護位置＋拒絕存取」才給管理員按鈕，網路磁碟一律不給。

use serde::Serialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::elevate::{looks_like_protected_location, WriteAccessReport};
use super::instance_validate::{validate_instance_path, InstanceValidation};
use super::paths::long_path;

/// 選資料夾時任何一項探測的上限。
pub const INSPECT_TIMEOUT: Duration = Duration::from_secs(3);
/// 選到啟動器清單資料夾時，最多列幾個候選（規格 S02 (b)）。
pub const MAX_CANDIDATES: usize = 5;
/// 掃清單資料夾時最多看幾個子資料夾（避免選到磁碟根目錄時掃太久）。
const MAX_CHILDREN_SCANNED: usize = 60;

/// 寫不進去的真正原因（分類碼給前端決定狀態卡的句子與唯一主要按鈕）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteIssue {
    /// 本機磁碟的系統保護位置，且 Windows 拒絕存取：以系統管理員身分重新開啟有用
    NeedsAdmin,
    /// 拒絕存取但不在保護位置（常見是防毒的受控資料夾存取或唯讀屬性）：管理員不一定有用
    Denied,
    DiskFull,
    /// UNC 或對應的網路磁碟機、或網路類錯誤：可能是連線不穩；以管理員身分開啟看不到對應的磁碟機代號（推測），不給管理員鈕
    Network,
    /// OneDrive 等雲端檔案（os error 358–406 為雲端檔案類錯誤，範圍為推測）
    Cloud,
    /// 防毒攔截（os error 225／226）
    Antivirus,
    /// 檔案正被其他程式使用
    Locked,
    Missing,
    Unknown,
}

impl WriteIssue {
    pub fn code(self) -> &'static str {
        match self {
            Self::NeedsAdmin => "needs_admin",
            Self::Denied => "denied",
            Self::DiskFull => "disk_full",
            Self::Network => "network",
            Self::Cloud => "cloud",
            Self::Antivirus => "antivirus",
            Self::Locked => "locked",
            Self::Missing => "missing",
            Self::Unknown => "unknown",
        }
    }
}

/// 依 os error／ErrorKind 分類；`path` 用來判斷是不是網路磁碟、系統保護位置、OneDrive。
pub fn classify_io_error(err: &io::Error, path: &Path) -> WriteIssue {
    let network = is_network_path(path);
    if let Some(code) = err.raw_os_error() {
        match code {
            112 | 39 => return WriteIssue::DiskFull,
            32 | 33 => return WriteIssue::Locked,
            225 | 226 => return WriteIssue::Antivirus,
            358..=406 => return WriteIssue::Cloud,
            51 | 53 | 54 | 59 | 64 | 65 | 67 | 121 | 1222 | 1231 | 1232 | 2250 => return WriteIssue::Network,
            5 => return denied_issue(path, network),
            2 | 3 => return if network { WriteIssue::Network } else { WriteIssue::Missing },
            _ => {}
        }
    }
    match err.kind() {
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => denied_issue(path, network),
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => WriteIssue::DiskFull,
        io::ErrorKind::ResourceBusy => WriteIssue::Locked,
        io::ErrorKind::NotFound if !network => WriteIssue::Missing,
        _ if network => WriteIssue::Network,
        io::ErrorKind::TimedOut | io::ErrorKind::NetworkUnreachable | io::ErrorKind::HostUnreachable => {
            WriteIssue::Network
        }
        _ => WriteIssue::Unknown,
    }
}

fn denied_issue(path: &Path, network: bool) -> WriteIssue {
    if network {
        WriteIssue::Network
    } else if looks_like_protected_location(path) {
        WriteIssue::NeedsAdmin
    } else if looks_like_onedrive(path) {
        WriteIssue::Cloud
    } else {
        WriteIssue::Denied
    }
}

/// 路徑裡有一段以 OneDrive 開頭（「OneDrive」「OneDrive - 公司」）。
pub fn looks_like_onedrive(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase().starts_with("onedrive"))
}

/// UNC（`\\server\share`、`\\?\UNC\`）或對應的網路磁碟機（GetDriveTypeW＝DRIVE_REMOTE）。
pub fn is_network_path(path: &Path) -> bool {
    let text = path.to_string_lossy().replace('/', "\\");
    if let Some(rest) = text.strip_prefix("\\\\?\\") {
        return rest.to_ascii_uppercase().starts_with("UNC\\") || drive_is_remote(rest);
    }
    if text.starts_with("\\\\") {
        return true;
    }
    drive_is_remote(&text)
}

#[cfg(windows)]
fn drive_is_remote(text: &str) -> bool {
    use std::os::windows::ffi::OsStrExt;
    let bytes = text.as_bytes();
    if bytes.len() < 2 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    let root: Vec<u16> = std::ffi::OsStr::new(&format!("{}:\\", bytes[0] as char))
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    extern "system" {
        fn GetDriveTypeW(lpRootPathName: *const u16) -> u32;
    }
    const DRIVE_REMOTE: u32 = 4;
    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
}

#[cfg(not(windows))]
fn drive_is_remote(_text: &str) -> bool {
    false
}

/// 狀態卡以外（紀錄、套用時的錯誤）用的完整白話說明。
pub fn issue_message(issue: WriteIssue, path: &Path, detail: &str) -> String {
    let at = path.display();
    let head = match issue {
        WriteIssue::NeedsAdmin => "這個資料夾在系統保護的位置，需要系統管理員權限才寫得進去。".to_string(),
        WriteIssue::Denied => {
            "Windows 拒絕寫入這個資料夾，可能是防毒軟體的「受控資料夾存取」擋住了，或資料夾被設成唯讀。".to_string()
        }
        WriteIssue::DiskFull => "磁碟空間不足，寫不進這個資料夾。請清出一些空間再試。".to_string(),
        WriteIssue::Network => "網路磁碟暫時寫不進去，可能是連線不穩。請確認網路磁碟接上後再試。".to_string(),
        WriteIssue::Cloud => {
            "這個資料夾在 OneDrive，雲端同步擋住了寫入。請確認 OneDrive 正在執行，並把資料夾設為「一律保留在此裝置上」。"
                .to_string()
        }
        WriteIssue::Antivirus => "防毒軟體擋住了寫入（檔案可能被移到隔離區），請到 Windows 安全性查看。".to_string(),
        WriteIssue::Locked => "有檔案正被其他程式使用（遊戲或啟動器可能還開著），請關閉後再試。".to_string(),
        WriteIssue::Missing => "找不到這個資料夾，可能已被移動或刪除。".to_string(),
        WriteIssue::Unknown => "寫不進這個資料夾。".to_string(),
    };
    if detail.trim().is_empty() {
        format!("{head}\n路徑：{at}")
    } else {
        format!("{head}\n路徑：{at}\n（細節：{}）", detail.trim())
    }
}

/// 只讀的寫入檢查：以「新增檔案／子資料夾」的權限開啟資料夾控制代碼，不建任何檔案。
pub fn probe_dir_readonly(dir: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_ADD_FILE: u32 = 0x0002;
        const FILE_ADD_SUBDIRECTORY: u32 = 0x0004;
        const FILE_SHARE_ALL: u32 = 0x0007;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        fs::OpenOptions::new()
            .access_mode(FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY)
            .share_mode(FILE_SHARE_ALL)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(long_path(dir))
            .map(|_| ())
    }
    #[cfg(not(windows))]
    {
        let meta = fs::metadata(dir)?;
        if meta.permissions().readonly() {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        Ok(())
    }
}

/// 選資料夾時的寫入檢查（唯讀）：遊戲資料夾本身與已存在的 mods、config、resourcepacks。
pub fn check_write_access_readonly(mc: &Path) -> WriteAccessReport {
    let targets = [mc.to_path_buf(), mc.join("mods"), mc.join("config"), mc.join("resourcepacks")];
    for (i, dir) in targets.iter().enumerate() {
        if i > 0 && !long_path(dir).is_dir() {
            continue;
        }
        if let Err(e) = probe_dir_readonly(dir) {
            let issue = classify_io_error(&e, dir);
            return WriteAccessReport {
                writable: false,
                needs_admin: issue == WriteIssue::NeedsAdmin,
                code: issue.code().into(),
                path: dir.display().to_string(),
                message: issue_message(issue, dir, &e.to_string()),
            };
        }
    }
    WriteAccessReport {
        writable: true,
        needs_admin: false,
        code: "ok".into(),
        path: mc.display().to_string(),
        message: String::new(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub path: String,
    pub name: String,
}

/// 資料夾「長什麼樣」：對、選到 mods、選到啟動器清單、找不到 mods、伺服器資料夾。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderShape {
    /// ok｜mods_selected｜launcher_list｜no_mods｜server｜invalid
    pub kind: String,
    /// 選到 mods 時的上一層（可一鍵改用）
    pub parent: Option<String>,
    /// 選到啟動器清單時底下可翻的模組整合包（最多 [`MAX_CANDIDATES`] 個）
    pub candidates: Vec<Candidate>,
    /// 有 options.txt（遊戲啟動過一次）
    pub has_options: bool,
}

fn leaf_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// 依驗證結果判斷形狀；只讀（is_dir／is_file／read_dir）。
pub fn folder_shape(path: &Path, validation: &InstanceValidation) -> FolderShape {
    let shape = |kind: &str| FolderShape { kind: kind.into(), parent: None, candidates: Vec::new(), has_options: false };
    if validation.ok {
        let mc = PathBuf::from(&validation.mc_dir);
        let has_options = long_path(&mc.join("options.txt")).is_file();
        let server = long_path(&mc.join("server.properties")).is_file() || long_path(&mc.join("eula.txt")).is_file();
        return FolderShape { has_options, ..shape(if server { "server" } else { "ok" }) };
    }
    if !long_path(path).is_dir() {
        return shape("invalid");
    }
    if leaf_name(path).eq_ignore_ascii_case("mods") {
        if let Some(parent) = path.parent().filter(|p| validate_instance_path(p).ok) {
            return FolderShape { parent: Some(parent.display().to_string()), ..shape("mods_selected") };
        }
    }
    let candidates = launcher_candidates(path);
    if !candidates.is_empty() {
        return FolderShape { candidates, ..shape("launcher_list") };
    }
    shape("no_mods")
}

/// 底下可以通過驗證的子資料夾（依名稱排序，最多 [`MAX_CANDIDATES`] 個）。
pub fn launcher_candidates(path: &Path) -> Vec<Candidate> {
    let Ok(entries) = fs::read_dir(long_path(path)) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .take(MAX_CHILDREN_SCANNED)
        .collect();
    dirs.sort();
    dirs.into_iter()
        .filter(|p| validate_instance_path(p).ok)
        .take(MAX_CANDIDATES)
        .map(|p| Candidate { name: leaf_name(&p), path: p.display().to_string() })
        .collect()
}

/// 選資料夾當下的一次檢查（驗證、形狀、寫入）；全部只讀。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInspection {
    /// false＝逾時或連不到這個資料夾本身
    pub reachable: bool,
    pub validation: InstanceValidation,
    pub shape: FolderShape,
    /// 驗證通過才檢查
    pub write: Option<WriteAccessReport>,
}

pub fn inspect_folder(path: &Path) -> FolderInspection {
    let validation = validate_instance_path(path);
    let shape = folder_shape(path, &validation);
    let write = validation.ok.then(|| check_write_access_readonly(Path::new(&validation.mc_dir)));
    FolderInspection { reachable: true, validation, shape, write }
}

/// 逾時或連不到時回給前端的結果（狀態卡：連不到這個資料夾，重新檢查）。
pub fn unreachable_inspection(path: &Path) -> FolderInspection {
    let network = is_network_path(path);
    FolderInspection {
        reachable: false,
        validation: InstanceValidation::unreachable(network),
        shape: FolderShape { kind: "invalid".into(), parent: None, candidates: Vec::new(), has_options: false },
        write: None,
    }
}

/// 在另一條執行緒跑，超過 `limit` 就放棄等待（回 None）；那條執行緒自己結束，不影響畫面。
pub fn run_with_timeout<T: Send + 'static>(limit: Duration, job: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(job());
    });
    rx.recv_timeout(limit).ok()
}

/// 瀏覽視窗沒有上次路徑時的起始位置：第一個存在的常見啟動器資料夾（只查本機使用者資料夾）。
pub fn common_launcher_dir() -> Option<PathBuf> {
    first_launcher_dir(dirs::home_dir().as_deref(), dirs::data_dir().as_deref())
}

pub fn first_launcher_dir(home: Option<&Path>, appdata: Option<&Path>) -> Option<PathBuf> {
    let mut list: Vec<PathBuf> = Vec::new();
    if let Some(home) = home {
        list.push(home.join("curseforge").join("minecraft").join("Instances"));
        list.push(home.join("Documents").join("Curseforge").join("Minecraft").join("Instances"));
    }
    if let Some(appdata) = appdata {
        list.push(appdata.join("PrismLauncher").join("instances"));
        list.push(appdata.join("ModrinthApp").join("profiles"));
        list.push(appdata.join(".minecraft"));
    }
    list.into_iter().find(|p| p.is_dir())
}
