//! 共享翻譯資料使用的整合包識別。
//!
//! 只保留整合包名稱分類，不把實例路徑、檔案清單或帳號資訊送到伺服器。

use serde::{Deserialize, Serialize};
use std::path::Path;

use super::hashutil::sha256_hex;
use super::pack_version::detect_pack_version;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationScope {
    /// 由整合包名稱正規化後產生的穩定識別，不含版本，方便跨版本重用。
    pub pack_key: String,
    /// 僅用於 Cloudflare 共享資料的分類與管理，已去除換行與過長內容。
    pub pack_name: String,
}

impl TranslationScope {
    pub fn from_instance(instance_or_minecraft: &Path) -> Self {
        let info = detect_pack_version(instance_or_minecraft);
        Self::from_name(&info.modpack_name)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
