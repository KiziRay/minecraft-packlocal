//! 路徑／名稱／網址安全檢查，降低被惡意路徑或檔名利用的風險。

use std::path::{Component, Path, PathBuf};

const MAX_JAR_BYTES: u64 = 800 * 1024 * 1024; // 800MB，過大可能 zip bomb
const MAX_FONT_BYTES: u64 = 80 * 1024 * 1024; // 80MB 字體上限
pub const MAX_ZIP_ENTRY_BYTES: u64 = 12 * 1024 * 1024; // 單一語言檔 12MB
const MAX_API_KEY_LEN: usize = 512;

/// 資源包／資料夾顯示名：禁止路徑穿越與危險字元
pub fn sanitize_folder_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Ok("繁體中文翻譯".into());
    }
    if name.chars().count() > 80 {
        return Err("名稱太長，請縮短到 80 字以內。".into());
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err("名稱不能包含路徑符號（例如 / \\ ..）。".into());
    }
    for c in name.chars() {
        if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
            return Err("名稱含有系統不允許的字元。".into());
        }
        if c.is_control() {
            return Err("名稱含有無效字元。".into());
        }
    }
    // 拒絕 Windows 保留名
    let upper = name.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "COM1" | "COM2" | "COM3" | "COM4" | "LPT1" | "LPT2" | "LPT3"
    ) {
        return Err("這個名稱系統不能用，請換一個。".into());
    }
    Ok(name.to_string())
}

/// Minecraft 資源包 namespace（assets 下資料夾名）
pub fn sanitize_namespace(ns: &str) -> Result<String, String> {
    let ns = ns.trim();
    if ns.is_empty() || ns.len() > 64 {
        return Err("模組命名空間無效。".into());
    }
    if ns.contains("..") || ns.contains('/') || ns.contains('\\') || ns.contains('\0') {
        return Err("模組命名空間含危險路徑。".into());
    }
    // 僅允許常見合法字元，擋奇怪路徑段
    if !ns
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err("模組命名空間含不允許字元。".into());
    }
    Ok(ns.to_string())
}

/// ZIP 內路徑：拒絕絕對路徑、..、過長
pub fn is_safe_zip_entry_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 512 {
        return false;
    }
    if name.starts_with('/') || name.starts_with('\\') {
        return false;
    }
    // Windows 磁碟機路徑
    if name.len() >= 2 && name.as_bytes()[1] == b':' {
        return false;
    }
    let n = name.replace('\\', "/");
    for part in n.split('/') {
        if part == ".." || part.contains('\0') {
            return false;
        }
    }
    true
}

/// 確認 resolved 路徑落在 base 之下（防 path traversal）
pub fn ensure_under_base(base: &Path, candidate: &Path) -> Result<PathBuf, String> {
    let base_c = dunce_canonicalize(base)?;
    // 若 candidate 尚不存在，用 parent 檢查
    let check = if candidate.exists() {
        dunce_canonicalize(candidate)?
    } else if let Some(parent) = candidate.parent() {
        let p = if parent.as_os_str().is_empty() {
            base_c.clone()
        } else if parent.exists() {
            dunce_canonicalize(parent)?
        } else {
            // 逐段確保沒有 ..
            validate_no_parent_dir(candidate)?;
            return Ok(candidate.to_path_buf());
        };
        if !p.starts_with(&base_c) && p != base_c {
            // parent under base is enough for new file
            if !p.starts_with(&base_c) {
                return Err("偵測到不安全的輸出路徑，已阻擋。".into());
            }
        }
        return Ok(candidate.to_path_buf());
    } else {
        base_c.clone()
    };
    if !check.starts_with(&base_c) {
        return Err("偵測到不安全的輸出路徑，已阻擋。".into());
    }
    Ok(check)
}

fn validate_no_parent_dir(p: &Path) -> Result<(), String> {
    for c in p.components() {
        if matches!(c, Component::ParentDir) {
            return Err("路徑不能包含「上一層目錄」。".into());
        }
    }
    Ok(())
}

fn dunce_canonicalize(p: &Path) -> Result<PathBuf, String> {
    // 不額外依賴 dunce：盡力 canonicalize
    std::fs::canonicalize(p).or_else(|_| {
        // 不存在時回傳絕對化
        if p.is_absolute() {
            Ok(p.to_path_buf())
        } else {
            std::env::current_dir()
                .map(|c| c.join(p))
                .map_err(|e| e.to_string())
        }
    })
}

/// 只允許 http(s) 公開網址，擋 file/javascript／內網等
pub fn validate_open_url(url: &str) -> Result<String, String> {
    let u = url.trim();
    if u.len() > 2048 {
        return Err("網址太長。".into());
    }
    if u.contains('\0') || u.contains('\r') || u.contains('\n') {
        return Err("網址含有無效字元。".into());
    }
    let lower = u.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err("只能開啟 http 或 https 網址。".into());
    }
    if lower.contains("javascript:") || lower.contains("data:") || lower.contains("file:") {
        return Err("不允許的網址類型。".into());
    }
    reject_private_or_local_host(&lower)?;
    Ok(u.to_string())
}

/// AI Base URL：僅 https，禁止內網／本機（防 SSRF 把金鑰打到惡意端）
pub fn validate_api_base_url(url: &str) -> Result<String, String> {
    let u = url.trim().trim_end_matches('/');
    if u.is_empty() {
        return Ok("https://api.deepseek.com".into());
    }
    if u.len() > 512 {
        return Err("Base URL 太長。".into());
    }
    if u.contains('\0') || u.contains('\r') || u.contains('\n') || u.contains(' ') {
        return Err("Base URL 含有無效字元。".into());
    }
    let lower = u.to_ascii_lowercase();
    if !lower.starts_with("https://") {
        return Err("API Base URL 必須使用 https://（較安全）。".into());
    }
    if lower.contains("javascript:") || lower.contains("data:") || lower.contains("file:") {
        return Err("不允許的 Base URL。".into());
    }
    reject_private_or_local_host(&lower)?;
    Ok(u.to_string())
}

/// 金鑰基本長度與字元檢查（不驗證是否真有效）
pub fn validate_api_key(key: &str) -> Result<(), String> {
    let k = key.trim();
    if k.is_empty() {
        return Ok(()); // 空白＝沿用，由呼叫端處理
    }
    if k.len() > MAX_API_KEY_LEN {
        return Err("API 金鑰太長，請檢查是否貼錯。".into());
    }
    if k.contains('\0') || k.contains('\r') || k.contains('\n') {
        return Err("API 金鑰含有無效字元。".into());
    }
    Ok(())
}

fn reject_private_or_local_host(lower_url: &str) -> Result<(), String> {
    // 取出 host 粗略字串（scheme 後到 / 或 : 或結尾）
    let rest = lower_url
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or(lower_url);
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .split('@')
        .next_back()
        .unwrap_or("");
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        return Err("網址缺少主機名稱。".into());
    }
    if host == "localhost"
        || host == "0.0.0.0"
        || host == "[::1]"
        || host == "::1"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
    {
        return Err("不允許本機或內網位址。".into());
    }
    // 常見私網／迴環
    if host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
        || host == "metadata.google.internal"
    {
        return Err("不允許本機或內網位址。".into());
    }
    // 172.16.0.0 – 172.31.255.255
    if let Some(rest) = host.strip_prefix("172.") {
        if let Some((a, _)) = rest.split_once('.') {
            if let Ok(n) = a.parse::<u8>() {
                if (16..=31).contains(&n) {
                    return Err("不允許本機或內網位址。".into());
                }
            }
        }
    }
    Ok(())
}

pub fn check_jar_size(path: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_JAR_BYTES {
        return Err("模組檔太大，已略過（安全上限）。".into());
    }
    Ok(())
}

pub fn check_font_file(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err("找不到字體檔案。".into());
    }
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
        return Err("請使用 .ttf、.otf 或 .ttc 字體檔。".into());
    }
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() == 0 {
        return Err("字體檔是空的。".into());
    }
    if meta.len() > MAX_FONT_BYTES {
        return Err("字體檔太大（上限約 80MB）。".into());
    }
    Ok(())
}

/// 正規化使用者輸入路徑：去引號，禁止 null
pub fn normalize_user_path(s: &str) -> Result<PathBuf, String> {
    let t = s.trim().trim_matches('"').trim_matches('\'');
    if t.is_empty() {
        return Err("路徑是空的。".into());
    }
    if t.contains('\0') {
        return Err("路徑含有無效字元。".into());
    }
    let p = PathBuf::from(t);
    validate_no_parent_dir(&p)?;
    Ok(p)
}
