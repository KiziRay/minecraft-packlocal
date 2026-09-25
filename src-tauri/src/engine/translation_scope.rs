//! 共享翻譯資料使用的整合包識別。
//!
//! 主鍵優先用 mods 檔名指紋（跨設備／換資料夾名仍穩定）；讀不到 mods 時才退回名稱 hash。
//! 不上傳實例路徑或帳號資訊。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::hashutil::sha256_hex;
use super::jar_scan::resolve_minecraft_dir;
use super::pack_version::detect_pack_version;

const PACK_KEY_HISTORY_LIMIT: usize = 8;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationScope {
    /// 穩定識別：優先 mods 指紋；否則由整合包名稱產生。不含帳號／路徑。
    pub pack_key: String,
    /// 僅用於 Cloudflare 共享資料的分類與管理，已去除換行與過長內容。
    pub pack_name: String,
}

impl TranslationScope {
    pub fn from_instance(instance_or_minecraft: &Path) -> Self {
        let info = detect_pack_version(instance_or_minecraft);
        let pack_name = normalize_pack_name(&info.modpack_name);
        if let Some(key) = pack_key_from_mods(instance_or_minecraft) {
            remember_pack_key(&key);
            return Self {
                pack_key: key,
                pack_name: if pack_name.is_empty() {
                    "unknown-modpack".into()
                } else {
                    pack_name
                },
            };
        }
        let scope = Self::from_name(&info.modpack_name);
        remember_pack_key(&scope.pack_key);
        scope
    }

    pub fn from_name(name: &str) -> Self {
        let pack_name = normalize_pack_name(name);
        let seed = if pack_name.is_empty() {
            "unknown-modpack"
        } else {
            pack_name.as_str()
        };
        let digest = sha256_hex(seed.as_bytes());
        Self {
            pack_key: digest[..24].to_string(),
            pack_name,
        }
    }

    pub fn is_known(&self) -> bool {
        !self.pack_key.is_empty() && !self.pack_name.is_empty()
    }

    pub fn lookup_pack_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        push_pack_key(&mut keys, &self.pack_key);
        for key in load_pack_key_history().keys {
            push_pack_key(&mut keys, &key);
            if keys.len() >= PACK_KEY_HISTORY_LIMIT {
                break;
            }
        }
        keys
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PackKeyHistory {
    #[serde(default)]
    keys: Vec<String>,
}

fn pack_keys_path() -> PathBuf {
    super::paths::resolve_file(Path::new("pack_keys.json"))
}

fn load_pack_key_history() -> PackKeyHistory {
    fs::read_to_string(pack_keys_path())
        .ok()
        .and_then(|text| serde_json::from_str::<PackKeyHistory>(&text).ok())
        .unwrap_or_default()
}

#[cfg(not(test))]
fn save_pack_key_history(history: &PackKeyHistory) {
    let path = pack_keys_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(history) {
        let _ = fs::write(path, text + "\n");
    }
}

fn push_pack_key(keys: &mut Vec<String>, key: &str) {
    let key = key.trim();
    if key.is_empty() || keys.iter().any(|existing| existing == key) {
        return;
    }
    keys.push(key.to_string());
}

#[cfg(not(test))]
fn remember_pack_key(key: &str) {
    let key = key.trim();
    if key.is_empty() {
        return;
    }
    let mut keys = Vec::new();
    push_pack_key(&mut keys, key);
    for old in load_pack_key_history().keys {
        push_pack_key(&mut keys, &old);
        if keys.len() >= PACK_KEY_HISTORY_LIMIT {
            break;
        }
    }
    save_pack_key_history(&PackKeyHistory { keys });
}

#[cfg(test)]
fn remember_pack_key(_key: &str) {}

// ══ 模組身分（namespace → 這個包裡是哪一個 mod 檔）════════════════════════
//
// # 這是拿來做什麼的
//
// 共享翻譯庫的信任模型原本只有「整合包」這一層：同一個 `pack_key` 投的票
// 一票就採用，不同 `pack_key` 要兩票。問題是兩個整合包只要有一個模組不同，
// `pack_key` 就完全不同——但整合包之間**大量共用同樣的模組**。
// `create:item.wrench` 這條，在任何裝了同一版 Create 的包裡都是同一個字串、
// 同一個意思，卻要等兩個不同的包各投一票才敢用。
//
// 有了「這個 namespace 來自哪一個 mod 檔」，雲端就能判斷
// 「不同整合包，但同一個模組的同一個版本」→ 等同同一個包，一票就夠。
//
// # 為什麼放在行程內記憶體而不是設定檔
//
// 這份對照表只在「掃描完到上傳完」這段期間有意義，換一個整合包就作廢。
// 存檔反而會讓下一次執行讀到上一包的資料。
static MOD_IDENTITY: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn mod_identity() -> &'static Mutex<HashMap<String, String>> {
    MOD_IDENTITY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 掃描完成後登記這一輪的「namespace → mod 檔識別」。
///
/// 傳進來的 value 應該是 mod 檔名（含版本，不含副檔名），例如
/// `create-1.20.1-0.5.1.f`。**版本必須留著**——這裡要分辨的正是版本，
/// 和 `pack_key` 用的 `normalize_mod_file_name`（刻意去掉版本）相反。
pub fn remember_mod_identities(map: HashMap<String, String>) {
    let Ok(mut guard) = mod_identity().lock() else {
        return;
    };
    *guard = map;
}

/// 這個 namespace 來自哪一個 mod 檔（沒登記過就回 `None`）。
pub fn mod_identity_of(ns: &str) -> Option<String> {
    let ns = ns.trim();
    if ns.is_empty() {
        return None;
    }
    mod_identity().lock().ok()?.get(ns).cloned()
}

fn normalize_pack_name(name: &str) -> String {
    name.lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .chars()
        .take(120)
        .collect()
}

/// 對 mods/*.jar|zip 檔名做溫和正規化後排序，取 sha256 前 24 hex。
fn pack_key_from_mods(instance_or_minecraft: &Path) -> Option<String> {
    let mc = resolve_minecraft_dir(instance_or_minecraft).ok()?;
    let mods = mc.join("mods");
    let entries = fs::read_dir(&mods).ok()?;
    let mut names: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())?;
        if ext != "jar" && ext != "zip" {
            continue;
        }
        let raw = path.file_name()?.to_string_lossy();
        names.push(normalize_mod_file_name(&raw));
    }
    if names.is_empty() {
        return None;
    }
    names.sort();
    names.dedup();
    let seed = names.join("\n");
    let digest = sha256_hex(seed.as_bytes());
    Some(digest[..24].to_string())
}

fn normalize_mod_file_name(name: &str) -> String {
    let mut s = name.trim().to_ascii_lowercase();
    for ext in [".jar", ".zip"] {
        if let Some(stripped) = s.strip_suffix(ext) {
            s = stripped.to_string();
        }
    }
    // 去掉尾端常見 loader、MC／模組版本片段（-fabric、+mc1.20.1、_1.2.3）。
    loop {
        let before = s.clone();
        if let Some(cut) = strip_trailing_loader_token(&s)
            .or_else(|| strip_trailing_version_token(&s))
        {
            s = cut;
        }
        if s == before {
            break;
        }
    }
    s
}

fn strip_trailing_version_token(s: &str) -> Option<String> {
    // 全程走**字元**索引。舊版用 `bytes[i - 1] as char` 配位元組位移，
    // 中文檔名的模組（`工業模組1.2.3.jar`）往回掃完數字之後，
    // `&s[i - 2..i]` 會切在中文字中間直接 panic——與 v21 讓整包翻譯
    // 崩潰的是同一類 bug（見 `engine/safe_text.rs`）。
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    if chars.is_empty() {
        return None;
    }
    let byte_at = |j: usize| -> usize { chars.get(j).map(|(p, _)| *p).unwrap_or(s.len()) };

    let mut i = chars.len(); // 字元索引
    let mut saw_digit = false;
    while i > 0 {
        let (_, c) = chars[i - 1];
        if c.is_ascii_digit() {
            saw_digit = true;
            i -= 1;
            continue;
        }
        if saw_digit && c == '.' {
            i -= 1;
            saw_digit = false;
            continue;
        }
        break;
    }
    if !saw_digit && i == chars.len() {
        return None;
    }
    // 版本體必須以數字或 mc+數字開頭（往回掃後 i 指到分隔符或起點）
    if i >= chars.len() {
        return None;
    }
    let mut token_start = i;
    if i >= 2 && chars[i - 2].1 == 'm' && chars[i - 1].1 == 'c' {
        token_start = i - 2;
    }
    let version_start = chars[token_start].1;
    if !(version_start.is_ascii_digit() || s[byte_at(token_start)..].starts_with("mc")) {
        return None;
    }
    if token_start == 0 {
        return None;
    }
    let sep = chars[token_start - 1].1;
    if sep == '-' || sep == '+' || sep == '_' {
        Some(s[..byte_at(token_start - 1)].to_string())
    } else {
        None
    }
}

fn strip_trailing_loader_token(s: &str) -> Option<String> {
    for loader in ["neoforge", "fabric", "forge", "quilt"] {
        if s == loader {
            return None;
        }
        for sep in ['-', '+', '_'] {
            let suffix = format!("{sep}{loader}");
            if let Some(stripped) = s.strip_suffix(&suffix) {
                return Some(stripped.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn non_ascii_mod_filenames_do_not_panic() {
        // 中文（或任何非 ASCII）檔名 + 尾端版本號，舊版會在 `&s[i-2..i]`
        // 切到中文字中間直接 panic。這是 v21 那次崩潰的同一類 bug。
        for name in [
            "工業模組1.2.3",
            "魔法擴充-1.20.1",
            "Créatures-2.0.0",
            "Färbt_1.0",
            "🔥火焰模組+3.4",
            "中文",
            "",
            "1.2.3",
        ] {
            let _ = strip_trailing_version_token(name); // 不 panic 就算過
        }
        // 功能本身沒壞：ASCII 的正常案例仍然要正確
        assert_eq!(
            strip_trailing_version_token("create-1.20.1").as_deref(),
            Some("create")
        );
        assert_eq!(
            strip_trailing_version_token("jei_mc1.20.1").as_deref(),
            Some("jei")
        );
        assert_eq!(strip_trailing_version_token("create").as_deref(), None);
        // 中文名 + 分隔符 + 版本也要正確切掉，不是只求不 panic
        assert_eq!(
            strip_trailing_version_token("魔法擴充-1.20.1").as_deref(),
            Some("魔法擴充")
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "modpack-i18n-scope-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn same_pack_name_has_same_key() {
        let a = TranslationScope::from_name("  Example Pack\n");
        let b = TranslationScope::from_name("Example   Pack");
        assert_eq!(a.pack_key, b.pack_key);
        assert_eq!(a.pack_name, "Example Pack");
    }

    #[test]
    fn version_is_not_part_of_pack_classification() {
        assert_eq!(
            TranslationScope::from_name("Example Pack 1.0").pack_key,
            TranslationScope::from_name("Example Pack 1.0").pack_key
        );
    }

    #[test]
    fn same_mods_same_key_despite_folder_name() {
        let a_root = temp_dir("pack-a");
        let b_root = temp_dir("pack-b-renamed");
        for root in [&a_root, &b_root] {
            fs::create_dir_all(root.join("mods")).unwrap();
            fs::write(root.join("mods/create-1.20.1-0.5.1.jar"), b"x").unwrap();
            fs::write(root.join("mods/jei-1.20.1-fabric.jar"), b"y").unwrap();
            fs::write(root.join("mmc-pack.json"), r#"{"name":"A"}"#).unwrap();
        }
        fs::write(a_root.join("mmc-pack.json"), r#"{"name":"All the Mods"}"#).unwrap();
        fs::write(b_root.join("mmc-pack.json"), r#"{"name":"ATM Copy"}"#).unwrap();

        let a = TranslationScope::from_instance(&a_root);
        let b = TranslationScope::from_instance(&b_root);
        assert_eq!(a.pack_key, b.pack_key);
        assert_ne!(a.pack_name, b.pack_name);

        let _ = fs::remove_dir_all(a_root);
        let _ = fs::remove_dir_all(b_root);
    }

    #[test]
    fn adding_mod_changes_key() {
        let root = temp_dir("pack-add");
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("mods/create.jar"), b"x").unwrap();
        let before = TranslationScope::from_instance(&root);
        fs::write(root.join("mods/jei.jar"), b"y").unwrap();
        let after = TranslationScope::from_instance(&root);
        assert_ne!(before.pack_key, after.pack_key);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn normalize_strips_loader_and_version_noise() {
        assert_eq!(normalize_mod_file_name("JEI-1.20.1-fabric.jar"), "jei");
        assert_eq!(normalize_mod_file_name("jei-1.21-forge.jar"), "jei");
        assert_eq!(
            normalize_mod_file_name("create-1.20.1-0.5.1.jar"),
            "create"
        );
    }

    #[test]
    fn normalize_strips_plus_mc_and_embedded_loader_noise() {
        assert_eq!(normalize_mod_file_name("SomeMod+mc1.20.1.jar"), "somemod");
        assert_eq!(normalize_mod_file_name("SomeMod+1.20.1.jar"), "somemod");
        assert_eq!(
            normalize_mod_file_name("SomeMod-fabric-1.20.1.jar"),
            "somemod"
        );
        assert_eq!(
            normalize_mod_file_name("SomeMod+neoforge+mc1.21.1.zip"),
            "somemod"
        );
    }
}
