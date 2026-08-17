//! Public stub: only non-secret Worker base URL used by the updater.
//! Full AI key storage / preference I/O stays in the private engine.

/// Cloudflare Worker base URL (not a secret). Upstream AI keys live only in Worker secrets.
pub const MANAGED_BASE_URL: &str = "https://modpack-i18n.jolin34563.workers.dev";
