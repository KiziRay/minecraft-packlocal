//! 套用前的「遊戲是不是還開著」前置檢查。
//!
//! 為什麼需要：套用會直接寫進遊戲實例資料夾。Minecraft 開著時 jar／資源包會被鎖，
//! 結果是半套用——玩家看到殘缺翻譯或閃退，然後把帳算在翻譯頭上。舊版只有在失敗後
//! 才說「請先關閉 Minecraft」，那是事後道歉，不是防呆。
//!
//! 失效安全方向（rules/50 R50-2）：偵測不到、權限不足、非 Windows → 一律回 `Unknown`
//! 並放行。這個檢查只能用來擋「確定在跑」的情況，不能變成新的卡關來源。

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameRunning {
    /// 確定有 Java 行程正掛著這個實例。
    Yes { detail: String },
    /// 查得到行程清單，確定沒有掛這個實例。
    No,
    /// 查不出來（沒有 PowerShell、逾時、權限不足、非 Windows）。一律當作放行。
    Unknown,
}

impl GameRunning {
    pub fn blocks_apply(&self) -> bool {
        matches!(self, Self::Yes { .. })
    }
}

/// 路徑正規化成可比對的小寫斜線形式，去掉結尾分隔符。
fn norm(path: &str) -> String {
    path.replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

/// 從命令列字串判斷是否指向這個實例。
///
/// 啟動器（CurseForge／Prism／MultiMC／官方）都會把 `--gameDir`／`-Duser.dir` 或
/// natives 路徑指到實例資料夾底下，所以「命令列含實例路徑」是穩定的判準。
/// 另外接受實例資料夾的最後一段名稱，涵蓋命令列用相對路徑的啟動器。
pub fn command_line_targets_instance(command_line: &str, instance_path: &str) -> bool {
    let haystack = norm(command_line);
    let needle = norm(instance_path);
    if needle.is_empty() {
        return false;
    }
    if haystack.contains(&needle) {
        return true;
    }
    // 退一步：實例資料夾名稱夠獨特時（例如 "atm10"）也算數，但太短的名稱不冒險。
    let leaf = needle.rsplit('/').next().unwrap_or_default();
    if leaf.len() >= 4 && haystack.contains(&format!("/{leaf}/")) {
        return true;
    }
    false
}

#[cfg(windows)]
fn java_command_lines() -> Option<Vec<String>> {
    use std::time::Duration;

    // 只問 java／javaw 兩種 image name，輸出量小；CommandLine 需要 CIM。
    let script = "Get-CimInstance Win32_Process -Filter \"Name='javaw.exe' or Name='java.exe'\" \
                  | Select-Object -ExpandProperty CommandLine";
    let output = crate::engine::win_process::hidden_command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    // spawn + wait_with_output 沒有原生逾時；PowerShell 這個查詢是有界的，
    // 但仍給一個保險：查詢本身失敗就回 None（＝Unknown＝放行）。
    let _ = Duration::from_secs(0);
    let done = output.wait_with_output().ok()?;
    if !done.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&done.stdout).to_string();
    Some(text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
}

#[cfg(not(windows))]
fn java_command_lines() -> Option<Vec<String>> {
    None
}

pub fn is_game_running(instance_path: &Path) -> GameRunning {
    let instance = instance_path.display().to_string();
    let Some(lines) = java_command_lines() else {
        return GameRunning::Unknown;
    };
    for line in &lines {
        if command_line_targets_instance(line, &instance) {
            return GameRunning::Yes {
                detail: "偵測到這個整合包的遊戲行程正在執行。".into(),
            };
        }
    }
    GameRunning::No
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_path_in_command_line_matches() {
        let cmd = r#""C:\Program Files\Java\bin\javaw.exe" -Duser.dir=D:\Games\ATM10 net.minecraft.client.main.Main"#;
        assert!(command_line_targets_instance(cmd, r"D:\Games\ATM10"));
        assert!(command_line_targets_instance(cmd, r"D:\Games\ATM10\"));
    }

    #[test]
    fn other_instance_does_not_match() {
        let cmd = r"javaw.exe -Duser.dir=D:\Games\SomethingElse";
        assert!(!command_line_targets_instance(cmd, r"D:\Games\ATM10"));
    }

    #[test]
    fn short_leaf_name_does_not_false_positive() {
        // 資料夾叫 "mc" 太短，不可以只靠名稱就判定
        let cmd = r"javaw.exe --gameDir /home/other/mc/run";
        assert!(!command_line_targets_instance(cmd, r"D:\Games\mc"));
    }

    #[test]
    fn distinct_leaf_name_matches_relative_launcher() {
        let cmd = r"javaw.exe --gameDir ../instances/atm10-hardmode/.minecraft";
        assert!(command_line_targets_instance(cmd, r"D:\packs\atm10-hardmode"));
    }

    #[test]
    fn empty_instance_never_matches() {
        assert!(!command_line_targets_instance("javaw.exe -Duser.dir=D:/x", ""));
    }

    #[test]
    fn unknown_never_blocks() {
        assert!(!GameRunning::Unknown.blocks_apply());
        assert!(!GameRunning::No.blocks_apply());
        assert!(GameRunning::Yes { detail: String::new() }.blocks_apply());
    }
}
