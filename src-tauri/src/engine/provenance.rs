//! 建置來源身分（P0-10）。
//!
//! 1.0.9 之前 `updater.rs` 把版本寫死成字串常數，於是一顆 EXE 由哪個 commit 建出來、
//! 是否含未提交變更、屬於哪個發布通道，全都無從判斷——`MCPL-1.0.9-test-v28.exe`
//! 就是這樣變成無法驗證的 binary。本模組把 `build.rs` 注入的值收成 typed 結構，
//! 讓 release manifest 與更新驗證有可比對的來源身分。

use serde::{Deserialize, Serialize};

/// 發布通道。client 的通道由建置決定，**不由 Worker 回應決定**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Beta,
    Canary,
    Test,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "beta",
            Channel::Canary => "canary",
            Channel::Test => "test",
        }
    }

    /// 不在集合內一律回 None；呼叫端必須拒絕，不得猜一個最接近的。
    pub fn parse(value: &str) -> Option<Channel> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stable" => Some(Channel::Stable),
            "beta" => Some(Channel::Beta),
            "canary" => Some(Channel::Canary),
            "test" => Some(Channel::Test),
            _ => None,
        }
    }
}

/// 40 位 hex 才算真正可追溯的 commit；`unknown` 是合法但不可信的 sentinel。
pub fn is_traceable_commit(commit: &str) -> bool {
    commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildProvenance {
    pub version: String,
    pub commit: String,
    pub dirty: bool,
    pub build_time: String,
    pub channel: Channel,
}

/// B2-D 的單一判準：dirty 或 commit 不可追溯 → 永遠不得作為 stable 發布候選。
///
/// client 自身的 `BuildProvenance` 與 manifest 帶來的 `ManifestProvenance` 是兩個型別，
/// 但判準必須完全一致——所以抽成同一個函式，避免兩邊各寫一份而慢慢分歧。
pub fn eligible_for_stable(commit: &str, dirty: bool) -> bool {
    !dirty && is_traceable_commit(commit)
}

/// 目前執行檔的來源身分。值由 `build.rs` 於編譯期注入。
pub fn current() -> BuildProvenance {
    BuildProvenance {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit: option_env!("MCPL_GIT_COMMIT").unwrap_or("unknown").to_string(),
        dirty: option_env!("MCPL_GIT_DIRTY") != Some("false"),
        build_time: iso8601_from_epoch(
            option_env!("MCPL_BUILD_EPOCH")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0),
        ),
        channel: option_env!("MCPL_CHANNEL")
            .and_then(Channel::parse)
            .unwrap_or(Channel::Test),
    }
}

/// build id＝版本－短 commit（－dirty 標記）。同版不同建置可區分，
/// 且一眼看得出是不是乾淨建置。
///
/// 分隔符刻意用 `-` 而非 `+`：build id 會進 `/api/desktop/latest?build=` 的 query string，
/// 而 `+` 在 query 會被解讀成空白，送到 Worker 就變成另一個字串。`-` 是 URL unreserved 字元。
pub fn current_build_id() -> String {
    let p = current();
    let short = if is_traceable_commit(&p.commit) {
        &p.commit[..12]
    } else {
        "unknown"
    };
    if p.dirty {
        format!("{}-{}-dirty", p.version, short)
    } else {
        format!("{}-{}", p.version, short)
    }
}

/// build id 只能含 URL unreserved 字元，否則進 query string 會被改寫。
pub fn is_url_safe_build_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~'))
}

/// Unix epoch 秒 → ISO 8601 UTC。不引入時間函式庫；純算術，可測。
pub fn iso8601_from_epoch(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant 的 civil_from_days：天數（1970-01-01 起算）→ 年月日。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_parse_rejects_unknown_values() {
        assert_eq!(Channel::parse("stable"), Some(Channel::Stable));
        assert_eq!(Channel::parse(" TEST "), Some(Channel::Test));
        assert_eq!(Channel::parse("beta"), Some(Channel::Beta));
        assert_eq!(Channel::parse("canary"), Some(Channel::Canary));
        // 不猜、不 fallback：不在集合內一律 None
        assert_eq!(Channel::parse("production"), None);
        assert_eq!(Channel::parse(""), None);
        assert_eq!(Channel::parse("stable-2"), None);
    }

    #[test]
    fn traceable_commit_requires_full_hex_sha() {
        assert!(is_traceable_commit("e66c7999b8432b2a4f370a62d78ef6abc211c8aa"));
        assert!(!is_traceable_commit("unknown"));
        assert!(!is_traceable_commit("e66c799")); // 短 hash 不算可追溯
        assert!(!is_traceable_commit(&"z".repeat(40))); // 非 hex
        assert!(!is_traceable_commit(""));
    }

    #[test]
    fn stable_eligibility_rejects_dirty_and_unknown() {
        const CLEAN: &str = "e66c7999b8432b2a4f370a62d78ef6abc211c8aa";
        assert!(eligible_for_stable(CLEAN, false));
        // 未提交變更 → 不合格
        assert!(!eligible_for_stable(CLEAN, true));
        // 來源不明 → 不合格（v28 就是這一類）
        assert!(!eligible_for_stable("unknown", false));
        assert!(!eligible_for_stable("unknown", true));
        // 短 hash 不算可追溯
        assert!(!eligible_for_stable("e66c799", false));
    }

    #[test]
    fn iso8601_matches_known_instants() {
        // 期望值以 `date -u -d @<epoch>` 核對過，不是靠推算。
        assert_eq!(iso8601_from_epoch(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_from_epoch(1), "1970-01-01T00:00:01Z");
        assert_eq!(iso8601_from_epoch(1_788_496_610), "2026-09-04T04:36:50Z");
        assert_eq!(iso8601_from_epoch(1_788_503_810), "2026-09-04T06:36:50Z");
        // 閏日
        assert_eq!(iso8601_from_epoch(1_709_164_800), "2024-02-29T00:00:00Z");
        // 2000-03-01（世紀閏年邊界之後）
        assert_eq!(iso8601_from_epoch(951_868_800), "2000-03-01T00:00:00Z");
    }

    #[test]
    fn build_id_stays_url_safe() {
        // `+` 在 query string 會被解讀成空白，build id 進 ?build= 就會變成別的字串。
        assert!(is_url_safe_build_id(&current_build_id()));
        assert!(is_url_safe_build_id("1.0.9-e66c7999b843"));
        assert!(is_url_safe_build_id("1.0.9-e66c7999b843-dirty"));
        assert!(!is_url_safe_build_id("1.0.9+e66c7999b843"));
        assert!(!is_url_safe_build_id("1.0.9 dirty"));
        assert!(!is_url_safe_build_id(""));
    }

    /// 編進來的 commit 必須就是**現在**這棵樹的 HEAD。
    ///
    /// 踩過的坑：build.rs 原本用 `cargo:rerun-if-changed=../.git`，但在 git worktree 裡
    /// `.git` 是一個內容永不改變的檔案，於是 build script 一次都沒重跑，
    /// EXE 一直帶著第一次建置時的 commit——provenance 會說謊，
    /// 而說謊的 provenance 比沒有 provenance 更糟（它會讓人相信一個錯的答案）。
    ///
    /// 這條測試把「重跑觸發器有沒有真的生效」變成機械可驗，不必靠人記得去比對。
    #[test]
    fn embedded_commit_matches_the_current_head() {
        let embedded = current().commit;
        if embedded == "unknown" {
            return; // 不在 git 環境（例如打包後的原始碼），無從比對
        }
        let Ok(out) = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
        else {
            return; // 沒有 git 可用
        };
        if !out.status.success() {
            return;
        }
        let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if head.is_empty() {
            return;
        }
        assert_eq!(
            embedded, head,
            "編進 binary 的 commit 與目前 HEAD 不符：build.rs 的 rerun 觸發器沒生效，\
             provenance 是舊的。請檢查 emit_rerun_triggers（worktree 的 .git 是檔案，\
             盯它不會觸發重跑）"
        );
    }

    #[test]
    fn current_provenance_is_well_formed() {
        let p = current();
        assert!(!p.version.is_empty());
        assert!(!p.commit.is_empty(), "commit 不得為空字串，取不到要填 unknown");
        assert!(p.build_time.ends_with('Z'));
        assert_eq!(p.build_time.len(), 20);
        // channel 必須落在允許集合內
        assert!(Channel::parse(p.channel.as_str()).is_some());
    }

    #[test]
    fn build_id_marks_dirty_and_unknown_builds() {
        let id = current_build_id();
        assert!(id.starts_with(env!("CARGO_PKG_VERSION")));
        // 版本後面一定接得上來源標記，不會只有一個裸版號
        assert!(id.len() > env!("CARGO_PKG_VERSION").len() + 1);
    }
}
