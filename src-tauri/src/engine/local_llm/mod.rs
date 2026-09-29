mod download;
mod hardware;
mod install;
mod gguf_meta;
mod manifest;
pub mod release;
pub mod timeouts;
mod select;
mod server;
pub mod sizing;

pub use install::{
    delete_local_model, ensure_ready_for_translate, install_and_start, probe_install, ProbeView,
};
pub use server::{
    chat_base_url, files_ready, health_ok, load_state, stop_own_server, LOCAL_LLM_API_KEY,
    PORT_BASE,
};
/// 本地伺服器狀態（啟動時的顯示卡層數、上下文、同時處理數）。
pub use server::ServerState as ServerStateView;
/// 只有 deepseek.rs 的測試會引用：釘住「輸出上限 ≤ 上下文 1/4」這條不變式。
#[allow(unused_imports)]
pub use server::context_size_for;
pub use server::recommended_parallel_slots;
pub use server::{own_server_liveness, take_start_note, OwnServerLiveness};

use crate::engine::security::validate_local_llm_base_url;
use install::default_install_dir;
use std::path::Path;

pub fn active_chat_base_url() -> Result<String, String> {
    let state = load_state();
    if state.port < PORT_BASE || !health_ok(state.port) {
        return Err("本地模型尚未就緒。請先完成安裝並等到健康檢查通過。".into());
    }
    validate_local_llm_base_url(&chat_base_url(state.port))
}

pub fn is_installed() -> bool {
    let state = load_state();
    if !state.install_dir.trim().is_empty() && files_ready(Path::new(&state.install_dir)) {
        return true;
    }
    files_ready(&default_install_dir())
}

/// 狀態檔記過的安裝資料夾本身不存在（搬走、外接硬碟沒接）。沒記過、或資料夾還在（不管有沒有模型）都是 false。
pub(crate) fn install_dir_missing(recorded: &str) -> bool {
    let path = recorded.trim();
    !path.is_empty() && !Path::new(path).exists()
}

pub fn status_view() -> serde_json::Value {
    let state = load_state();
    let dir = if state.install_dir.trim().is_empty() {
        default_install_dir()
    } else {
        std::path::PathBuf::from(&state.install_dir)
    };
    let installed = files_ready(&dir);
    // 防禦性紀錄：「資料夾已設定但判定未安裝」是使用者回報過的嚴重症狀（見
    // docs/MCPL-FIX-PLAN-8.md 項目 8）。讀碼追完這條鏈路本輪沒找到新根因，判斷
    // 很可能是第九輪 state_path() 修好前的舊測試 exe 測到的。留這行以防萬一還會
    // 再發生——下次出現就有現場證據可看，不必再靠讀碼猜第三個假說。
    if !state.install_dir.trim().is_empty() && !installed {
        log_install_dir_mismatch(&dir);
    }
    let ready = state.port >= PORT_BASE && health_ok(state.port);
    // B5a-2：設定→資料與備份顯示「佔用 GB」（只讀；只算刪除時會刪的 models／runtime 兩個子目錄）
    let size_bytes = if installed {
        crate::engine::paths::dir_size_bytes(&dir.join("models"))
            + crate::engine::paths::dir_size_bytes(&dir.join("runtime"))
    } else {
        0
    };
    serde_json::json!({
        "installed": installed,
        // B5b 審查 3b：裝過（狀態檔記了安裝位置）但那個資料夾本身不在了——畫面照實說「找不到安裝位置」；
        // 資料夾在但沒有模型照「還沒下載」（第二輪審查）
        "installDirMissing": !installed && install_dir_missing(&state.install_dir),
        "sizeBytes": size_bytes,
        "ready": ready,
        "port": state.port,
        "pid": state.pid,
        "installDir": dir.display().to_string(),
        "message": if ready {
            "本地模型可以使用，可以按開始翻譯。"
        } else if installed {
            "檔案已在，但服務尚未就緒。"
        } else {
            "尚未安裝本地模型。"
        }
    })
}

/// 附加一行到可攜式根的 `install-dir-mismatch.log`：記錄的 install_dir、
/// 資料夾本身在不在、`llama-server.exe` 找不找得到。寫失敗（例如唯讀環境）
/// 就靜默略過，這只是輔助排查，不能反過來變成新的失敗點。
fn log_install_dir_mismatch(dir: &std::path::Path) {
    use std::io::Write;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!(
        "unix:{secs} install_dir={} dir_exists={} exe_found={}\n",
        dir.display(),
        dir.is_dir(),
        server::find_llama_exe(dir).is_ok()
    );
    let log_path = super::paths::resolve_file(std::path::Path::new("install-dir-mismatch.log"));
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = f.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod b5b_install_dir_tests {
    use super::install_dir_missing;

    /// B5b 第二輪審查 3b：只有「記住的安裝資料夾本身不存在」才算找不到位置；
    /// 資料夾在但沒有模型（偵測後還沒下載、或模型檔被刪）照「還沒下載」。
    #[test]
    fn b5b_install_dir_missing_only_when_the_recorded_folder_is_gone() {
        let root = std::env::temp_dir().join(format!("mcpl-b5b-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("models")).unwrap();
        let recorded = root.display().to_string();
        // 偵測後還沒下載：資料夾在、沒有模型
        assert!(!install_dir_missing(&recorded));
        // 刪除模型檔後：資料夾還在
        std::fs::remove_dir_all(root.join("models")).unwrap();
        assert!(!install_dir_missing(&recorded));
        // 整個資料夾不見（搬走、外接硬碟沒接）
        std::fs::remove_dir_all(&root).unwrap();
        assert!(install_dir_missing(&recorded));
        // 從沒記過位置
        assert!(!install_dir_missing(""));
        assert!(!install_dir_missing("   "));
    }
}
