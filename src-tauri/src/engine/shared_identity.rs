//! 共享庫身分：廢 `__shared_text`；JAR 走模組鍵，實例 FTB／kubejs 走 pack.*。

use std::collections::HashMap;
use std::path::Path;

use super::translation_scope::TranslationScope;

const PACK_PREFIX: &str = "pack.";
const UNKNOWN_PACK_NS: &str = "pack.unknown";

/// Worker `tmValidNs`：`^[a-z0-9_.-]{1,64}$`
pub fn sanitize_share_ns(raw: &str, scope: Option<&TranslationScope>) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with("__") || trimmed == "shared_text" {
        return pack_namespace(scope);
    }
    let mut out = String::with_capacity(trimmed.len().min(64));
    for c in trimmed.chars() {
        if out.len() >= 64 {
            break;
        }
        let mapped = if c.is_ascii_uppercase() {
            c.to_ascii_lowercase()
        } else {
            c
        };
        if mapped.is_ascii_lowercase() || mapped.is_ascii_digit() || mapped == '_' || mapped == '.' || mapped == '-'
        {
            out.push(mapped);
        } else if mapped == '/' || mapped == '\\' {
            out.push('.');
        }
    }
    while out.starts_with('.') {
        out.remove(0);
    }
    if out.is_empty() || out.starts_with("__") {
        return pack_namespace(scope);
    }
    out
}

pub fn pack_namespace(scope: Option<&TranslationScope>) -> String {
    match scope {
        Some(s) if !s.pack_key.trim().is_empty() => {
            format!("{PACK_PREFIX}{}", s.pack_key.trim())
        }
        _ => UNKNOWN_PACK_NS.into(),
    }
}

pub fn is_pack_instance_path(path: &Path) -> bool {
    let lower = path_unix_lower(path);
    lower.contains("/config/ftbquests")
        || lower.contains("/ftbquests/")
        || lower.contains("/kubejs/")
        || lower.contains("/minemenu/")
        || lower.contains("/defaultconfigs/")
}

fn path_unix_lower(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

/// `assets/<ns>/` 或 `data/<ns>/` 的下一段。
pub fn mod_namespace_from_path(path: &Path) -> Option<String> {
    let mut prev = String::new();
    for comp in path.components() {
        let name = comp.as_os_str().to_string_lossy().to_ascii_lowercase();
        if prev == "assets" || prev == "data" {
            if name == "minecraft" {
                prev = name;
                continue;
            }
            let ns = sanitize_share_ns(&name, None);
            if ns != UNKNOWN_PACK_NS && !ns.starts_with(PACK_PREFIX) {
                return Some(ns);
            }
        }
        prev = name;
    }
    None
}

pub fn share_ns_for_path(path: &Path, scope: Option<&TranslationScope>) -> String {
    if is_pack_instance_path(path) {
        return pack_namespace(scope);
    }
    mod_namespace_from_path(path).unwrap_or_else(|| pack_namespace(scope))
}

pub fn remember_ns(
    ns_by_src: &mut HashMap<String, String>,
    source: &str,
    path: &Path,
    scope: Option<&TranslationScope>,
) {
    ns_by_src
        .entry(source.to_string())
        .or_insert_with(|| share_ns_for_path(path, scope));
}

pub fn aligned_namespaces(
    texts: &[String],
    ns_by_src: &HashMap<String, String>,
    scope: Option<&TranslationScope>,
) -> Vec<String> {
    texts
        .iter()
        .map(|s| {
            ns_by_src
                .get(s)
                .cloned()
                .unwrap_or_else(|| pack_namespace(scope))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn rejects_shared_text_fake_namespace() {
        let scope = TranslationScope::from_name("Example Pack");
        assert_ne!(sanitize_share_ns("__shared_text", Some(&scope)), "__shared_text");
        assert!(sanitize_share_ns("__shared_text", Some(&scope)).starts_with("pack."));
        assert_eq!(pack_namespace(Some(&scope)), format!("pack.{}", scope.pack_key));
    }

    #[test]
    fn jar_assets_and_data_become_module_ns() {
        let p = PathBuf::from("resourcepacks/abc/assets/patchouli/books/foo.json");
        assert_eq!(mod_namespace_from_path(&p).as_deref(), Some("patchouli"));
        let d = PathBuf::from(r"data\origins\powers\foo.json");
        assert_eq!(mod_namespace_from_path(&d).as_deref(), Some("origins"));
    }

    #[test]
    fn instance_ftb_kubejs_use_pack_ns() {
        let scope = TranslationScope::from_name("Example Pack");
        let ftb = PathBuf::from("config/ftbquests/chapters/a.snbt");
        assert_eq!(share_ns_for_path(&ftb, Some(&scope)), pack_namespace(Some(&scope)));
        let kjs = PathBuf::from("kubejs/server_scripts/a.js");
        assert_eq!(share_ns_for_path(&kjs, Some(&scope)), pack_namespace(Some(&scope)));
        let book = PathBuf::from("resourcepacks/x/assets/modonomicon/books/a.json");
        assert_eq!(share_ns_for_path(&book, Some(&scope)), "modonomicon");
    }
}
