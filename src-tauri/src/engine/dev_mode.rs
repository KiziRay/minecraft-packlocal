//! 開發人員測試模式：只有開發者本人看得到、也只有他打得開的詳細診斷紀錄。
//!
//! # 為什麼要有這個
//!
//! 一般使用者的紀錄要「看得懂、看得完」，所以刻意精簡：一個階段一兩行、
//! 只講結果不講過程。但維護時需要的正好相反——要知道**每一個決策點走了哪條分支、
//! 為什麼**。過去只能靠出貨一版「診斷日誌版」給使用者跑，收到證據要好幾天。
//!
//! 開發人員模式把那份詳細紀錄變成常駐能力：打開就有，關掉完全不影響一般使用者。
//!
//! # 兩道門，缺一不可
//!
//! 1. **身分**：登入的 Discord 帳號必須是 [`DEVELOPER_USER_ID`]。
//!    這不是安全邊界（本機檔案本來就改得動），而是**避免一般使用者誤開**——
//!    詳細紀錄會拖慢速度、產生大量檔案，對一般玩家有害無益。
//! 2. **開關**：帳號對了還要自己去設定裡打開。預設關閉。
//!
//! # 紀錄寫到哪
//!
//! 寫進資料根目錄的 `開發診斷紀錄.log`，**與使用者看的執行日誌分開**。
//! 兩者混在一起的話，使用者的日誌會被淹沒，開發者的紀錄也會被日誌輪替截斷。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// 開發人員的 Discord 帳號。只有這個帳號登入時，設定裡才會出現這個選項。
pub const DEVELOPER_USER_ID: &str = "305389581304463360";

/// 紀錄檔名。與使用者看的「執行日誌.txt」分開。
const DEV_LOG_FILE: &str = "開發診斷紀錄.log";

/// 單一檔案的大小上限。超過就輪替一次（保留前一份），避免長期跑滿磁碟。
const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;

/// 開關現況。0＝還沒判定、1＝關、2＝開。
static STATE: AtomicU8 = AtomicU8::new(0);
/// 這個帳號有沒有資格。與 `STATE` 分開存，因為登出後資格會消失但設定值還在。
static ELIGIBLE: AtomicBool = AtomicBool::new(false);

/// 這個 Discord user id 是不是開發人員本人。
pub fn is_developer_account(user_id: &str) -> bool {
    user_id.trim() == DEVELOPER_USER_ID
}

/// 更新「這台機器目前登入的帳號有沒有資格」。登入狀態變動時呼叫。
pub fn set_eligible(user_id: &str) {
    let ok = is_developer_account(user_id);
    ELIGIBLE.store(ok, Ordering::Relaxed);
    if !ok {
        // 換成別的帳號就立刻失效，不留著上一個帳號開過的狀態
        STATE.store(1, Ordering::Relaxed);
    } else {
        STATE.store(0, Ordering::Relaxed); // 下次讀取時重新從設定檔判定
    }
}

/// 目前登入的帳號有沒有資格看到這個選項。
pub fn eligible() -> bool {
    ELIGIBLE.load(Ordering::Relaxed)
}

/// 開發人員模式現在是不是開著。
///
/// **兩個條件都成立才算開**：帳號有資格，而且設定裡打開了。
/// 這個函式會在熱路徑上被呼叫很多次，所以結果快取在原子變數裡。
pub fn enabled() -> bool {
    if !ELIGIBLE.load(Ordering::Relaxed) {
        return false;
    }
    match STATE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = read_setting();
            STATE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// 設定開關。沒有資格時一律拒絕，避免一般使用者誤開拖慢自己的翻譯。
pub fn set_enabled(on: bool) -> Result<bool, String> {
    if !ELIGIBLE.load(Ordering::Relaxed) {
        return Err("這個選項只對開發人員帳號開放。".into());
    }
    // 依路徑合併寫入：只改 developer.testMode，不整份覆寫（設定視窗可能同時在寫別的欄位）
    super::app_settings::patch_settings(&[super::app_settings::SettingsPatchOp {
        path: "developer.testMode".into(),
        value: Some(serde_json::Value::Bool(on)),
        delete: false,
    }])?;
    STATE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
    if on {
        log_line("session", "開發人員測試模式已啟用");
    }
    Ok(on)
}

fn read_setting() -> bool {
    super::app_settings::read_settings()
        .get("developer")
        .and_then(|d| d.get("testMode"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub fn log_path() -> PathBuf {
    super::paths::resolve_file(Path::new(DEV_LOG_FILE))
}

/// 寫一行詳細紀錄。模式沒開時**立刻回傳**，不做任何字串處理。
///
/// 這一點很重要：呼叫點會散布在翻譯的熱迴圈裡，關閉時的成本必須接近零。
/// 所以呼叫端要用 [`dev_log!`] 巨集，讓格式化本身也被跳過。
pub fn log_line(scope: &str, message: &str) {
    if !enabled() {
        return;
    }
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    rotate_if_needed(&path);
    let line = format!("[{}] [{scope}] {message}\n", stamp());
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// 超過上限就把現有檔案改名成 `.1`（只保留一份前檔）。
fn rotate_if_needed(path: &Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() < MAX_LOG_BYTES {
        return;
    }
    let previous = path.with_extension("log.1");
    let _ = std::fs::remove_file(&previous);
    let _ = std::fs::rename(path, &previous);
}

fn stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 台灣時間，只求可讀，不引入額外相依
    let t = secs + 8 * 3600;
    let (h, m, s) = ((t / 3600) % 24, (t / 60) % 60, t % 60);
    format!("{h:02}:{m:02}:{s:02}")
}

/// 寫一行開發診斷紀錄。模式沒開時連格式化都不會執行。
///
/// ```ignore
/// dev_log!("scan", "jar={} entries={}", name, count);
/// ```
#[macro_export]
macro_rules! dev_log {
    ($scope:expr, $($arg:tt)*) => {{
        if $crate::engine::dev_mode::enabled() {
            $crate::engine::dev_mode::log_line($scope, &format!($($arg)*));
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 這些測試都在改同一組全域原子變數，平行跑會互相踩。
    /// 每個測試先拿這把鎖，確保一次只有一個在動狀態。
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn only_the_developer_account_is_eligible() {
        let _g = lock();
        assert!(is_developer_account(DEVELOPER_USER_ID));
        assert!(is_developer_account("  305389581304463360  "), "前後空白要容忍");
        assert!(!is_developer_account("111222333444555666"));
        assert!(!is_developer_account(""));
    }

    #[test]
    fn switching_away_from_the_developer_account_turns_it_off_immediately() {
        let _g = lock();
        // 換帳號後不該留著上一個帳號開過的狀態
        set_eligible(DEVELOPER_USER_ID);
        assert!(eligible());
        set_eligible("999888777666555444");
        assert!(!eligible());
        assert!(!enabled(), "沒資格就一定是關的，不管設定檔寫什麼");
    }

    #[test]
    fn ineligible_accounts_cannot_turn_it_on() {
        let _g = lock();
        set_eligible("999888777666555444");
        let err = set_enabled(true).unwrap_err();
        assert!(err.contains("開發人員"));
        assert!(!enabled());
    }

    #[test]
    fn hand_editing_the_settings_file_does_not_enable_it_for_other_accounts() {
        let _g = lock();
        // 設定檔是純文字，任何人都改得動。所以「開著」不能只看設定檔——
        // 一定要同時滿足「登入的帳號是開發者本人」。
        // STATE=2 代表設定檔那一關已經算過且是開的；ELIGIBLE 才是身分那一關。
        STATE.store(2, Ordering::Relaxed);
        ELIGIBLE.store(false, Ordering::Relaxed);
        assert!(!enabled(), "設定檔寫 true 也不能讓非開發者帳號生效");

        // 換成開發者本人，同樣的設定值就會生效
        ELIGIBLE.store(true, Ordering::Relaxed);
        STATE.store(2, Ordering::Relaxed);
        assert!(enabled());
    }

    #[test]
    fn the_toggle_round_trips_for_the_developer_account() {
        let _g = lock();
        set_eligible(DEVELOPER_USER_ID);
        assert!(eligible());
        assert_eq!(set_enabled(true).unwrap(), true);
        assert!(enabled(), "開了就要真的是開的");
        assert_eq!(set_enabled(false).unwrap(), false);
        assert!(!enabled(), "關了就要真的是關的");
    }

    #[test]
    fn logging_is_a_no_op_when_disabled() {
        let _g = lock();
        // 呼叫點散布在翻譯熱迴圈裡，關閉時必須接近零成本，而且不可產生檔案
        set_eligible("999888777666555444");
        assert!(!enabled());
        log_line("test", "這一行不該被寫出去");
        // enabled() 為 false 時 log_line 會在第一行就 return，不會建立任何檔案
    }
}
