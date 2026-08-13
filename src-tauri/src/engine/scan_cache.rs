//! 語言檔檔案級快取：只保存已解析的 locale map，不保存 AI 譯文。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CACHE_FILES: usize = 20_000;
const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedLangFile {
    pub size: u64,
    pub modified_nanos: u128,
    pub namespace: String,
    pub locale: String,
    pub entries: HashMap<String, String>,
    #[serde(default)]
    pub last_seen_unix: u64,
}

#[derive(Debug, Default)]
pub struct ScanCache {
    files: HashMap<String, CachedLangFile>,
    changed: bool,
    hits: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    files: HashMap<String, CachedLangFile>,
}

impl ScanCache {
    pub fn load() -> Self {
        let entries = cache_path()
            .ok()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<CacheFile>(&text).ok())
            .map(|file| file.files)
            .unwrap_or_default();
        let before = entries.len();
        let files = entries
            .into_iter()
            .filter(|(path, _)| Path::new(path).is_file())
            .collect::<HashMap<_, _>>();
        Self {
            changed: files.len() != before,
            files,
            hits: 0,
        }
    }

    pub fn get(&mut self, path: &Path) -> Option<CachedLangFile> {
        let key = cache_key(path);
        let current = fingerprint(path)?;
        let cached = self.files.get(&key)?.clone();
        if cached.size != current.0 || cached.modified_nanos != current.1 {
            return None;
        }
        if let Some(entry) = self.files.get_mut(&key) {
            entry.last_seen_unix = now_unix();
            self.changed = true;
        }
        self.hits += 1;
        self.files.get(&key).cloned()
    }

    pub fn put(
        &mut self,
        path: &Path,
        namespace: String,
        locale: String,
        entries: HashMap<String, String>,
    ) {
        let Some((size, modified_nanos)) = fingerprint(path) else {
            return;
        };
        if self.files.len() >= MAX_CACHE_FILES && !self.files.contains_key(&cache_key(path)) {
            return;
        }
        self.files.insert(
            cache_key(path),
            CachedLangFile {
                size,
                modified_nanos,
                namespace,
                locale,
                entries,
                last_seen_unix: now_unix(),
            },
        );
        self.changed = true;
    }

    pub fn hits(&self) -> usize {
        self.hits
    }

    pub fn save(&mut self) -> Result<(), String> {
        if !self.changed {
            return Ok(());
        }
        let path = cache_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut files = self.files.clone();
        let mut body = serialize_cache(&files)?;
        if files.len() > MAX_CACHE_FILES || body.len() > MAX_CACHE_BYTES {
            let mut ranked = files.into_iter().collect::<Vec<_>>();
            ranked.sort_by_key(|(_, entry)| std::cmp::Reverse(entry.last_seen_unix));
            ranked.truncate(MAX_CACHE_FILES);
            files = ranked.into_iter().collect();
            body = serialize_cache(&files)?;
        }
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, body).map_err(|e| e.to_string())?;
        if path.exists() {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
        self.changed = false;
        Ok(())
    }
}

fn serialize_cache(files: &HashMap<String, CachedLangFile>) -> Result<String, String> {
    serde_json::to_string(&CacheFile {
        version: 2,
        files: files.clone(),
    })
    .map_err(|e| e.to_string())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn cache_path() -> Result<PathBuf, String> {
    Ok(dirs::data_dir()
        .ok_or_else(|| "找不到使用者資料目錄，略過掃描快取".to_string())?
        .join("modpack-i18n-tool")
        .join("scan-cache.json"))
}

fn cache_key(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn fingerprint(path: &Path) -> Option<(u64, u128)> {
    let metadata = fs::metadata(path).ok()?;
    let modified_nanos = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((metadata.len(), modified_nanos))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn cache_hit_requires_same_file_fingerprint() {
        let root = std::env::temp_dir().join(format!("scan_cache_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("en_us.json");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(b"{}").unwrap();
        let mut cache = ScanCache::default();
        cache.put(&path, "demo".into(), "en_us".into(), HashMap::new());
        assert!(cache.get(&path).is_some());
        fs::write(&path, b"{\"a\":\"changed\"}").unwrap();
        assert!(cache.get(&path).is_none());
        let _ = fs::remove_dir_all(root);
    }
}
