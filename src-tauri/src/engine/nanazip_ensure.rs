//! 背景確保分享自解檔所需壓縮工具。玩家可見錯誤不得出現內部工具名。

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use walkdir::WalkDir;

use crate::engine::win_process::hidden_command;

const PLAYER_SHARE_UNAVAILABLE: &str = "暫時無法建立分享檔，請稍後再試。";
const WINGET_ID: &str = "M2Team.NanaZip";
const STORE_ID: &str = "9N8G7TSCL17S";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Running,
    Ready,
    Failed,
}

fn phase() -> &'static Mutex<Phase> {
    static P: OnceLock<Mutex<Phase>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(Phase::Idle))
}

pub fn tools_ready() -> bool {
    find_cli().is_ok() && find_sfx().is_ok()
}

pub fn should_skip_install(cli_ok: bool, sfx_ok: bool) -> bool {
    cli_ok && sfx_ok
}

pub fn player_unavailable() -> String {
    PLAYER_SHARE_UNAVAILABLE.into()
}

pub fn find_cli() -> Result<PathBuf, String> {
    let candidates = [
        "NanaZipC",
        "NanaZipC.exe",
        "nanazipc",
        "7z",
        "7z.exe",
    ];
    for name in candidates {
        let mut cmd = hidden_command("where");
        if let Ok(output) = cmd.arg(name).output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = text.lines().next() {
                    let p = PathBuf::from(line.trim());
                    if p.is_file() {
                        return Ok(p);
                    }
                }
            }
        }
    }
    let prog = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    for rel in ["NanaZip/NanaZipC.exe", "7-Zip/7z.exe"] {
        let p = PathBuf::from(&prog).join(rel);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(PLAYER_SHARE_UNAVAILABLE.into())
}

pub fn find_sfx() -> Result<PathBuf, String> {
    let mut roots = Vec::new();
    if let Ok(pf) = std::env::var("ProgramFiles") {
        roots.push(PathBuf::from(&pf).join("NanaZip"));
        roots.push(PathBuf::from(&pf).join("7-Zip"));
    }
    roots.push(PathBuf::from(r"C:\Program Files\NanaZip"));
    roots.push(PathBuf::from(r"C:\Program Files\7-Zip"));
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join(r"Programs\NanaZip"));
    }
    let apps = PathBuf::from(r"C:\Program Files\WindowsApps");
    if apps.is_dir() {
        if let Ok(rd) = fs::read_dir(&apps) {
            for entry in rd.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.contains("NanaZip") {
                    roots.push(entry.path());
                }
            }
        }
    }
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&root).max_depth(4).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.eq_ignore_ascii_case("NanaZip.Core.Windows.sfx")
                || name.eq_ignore_ascii_case("7z.sfx")
                || name.eq_ignore_ascii_case("7zS.sfx")
            {
                if path.is_file() {
                    return Ok(path.to_path_buf());
                }
            }
        }
    }
    Err(PLAYER_SHARE_UNAVAILABLE.into())
}

pub fn ensure_in_background() {
    std::thread::Builder::new()
        .name("mcpl-share-tool".into())
        .spawn(|| {
            let _ = run_ensure();
        })
        .ok();
}

fn set_phase(next: Phase) {
    if let Ok(mut g) = phase().lock() {
        *g = next;
    }
}

fn current_phase() -> Phase {
    phase().lock().map(|g| *g).unwrap_or(Phase::Idle)
}

fn run_ensure() -> Result<(), String> {
    if should_skip_install(find_cli().is_ok(), find_sfx().is_ok()) {
        set_phase(Phase::Ready);
        return Ok(());
    }
    {
        let mut g = phase().lock().map_err(|_| PLAYER_SHARE_UNAVAILABLE.to_string())?;
        if matches!(*g, Phase::Ready | Phase::Running) {
            return Ok(());
        }
        *g = Phase::Running;
    }
    if tools_ready() {
        set_phase(Phase::Ready);
        return Ok(());
    }
    let _ = try_winget(WINGET_ID, false);
    if !tools_ready() {
        let _ = try_winget(STORE_ID, true);
    }
    if tools_ready() {
        set_phase(Phase::Ready);
        Ok(())
    } else {
        set_phase(Phase::Failed);
        Err(PLAYER_SHARE_UNAVAILABLE.into())
    }
}

fn try_winget(id: &str, msstore: bool) -> bool {
    let mut cmd = hidden_command("winget");
    cmd.args([
        "install",
        "--id",
        id,
        "--accept-package-agreements",
        "--accept-source-agreements",
        "--disable-interactivity",
        "--silent",
    ]);
    if msstore {
        cmd.args(["--source", "msstore"]);
    } else {
        cmd.arg("-e");
    }
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn wait_until_ready(limit: Duration) -> Result<(), String> {
    if tools_ready() {
        set_phase(Phase::Ready);
        return Ok(());
    }
    match current_phase() {
        Phase::Idle | Phase::Failed => {
            let _ = run_ensure();
        }
        Phase::Running | Phase::Ready => {}
    }
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if tools_ready() {
            set_phase(Phase::Ready);
            return Ok(());
        }
        if matches!(current_phase(), Phase::Failed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    if tools_ready() {
        set_phase(Phase::Ready);
        Ok(())
    } else {
        Err(PLAYER_SHARE_UNAVAILABLE.into())
    }
}

pub fn require_tools() -> Result<(PathBuf, PathBuf), String> {
    wait_until_ready(Duration::from_secs(45))?;
    Ok((find_cli()?, find_sfx()?))
}

#[cfg(test)]
mod tests {
    use super::{player_unavailable, should_skip_install};

    #[test]
    fn skip_install_when_both_tools_present() {
        assert!(should_skip_install(true, true));
        assert!(!should_skip_install(true, false));
        assert!(!should_skip_install(false, true));
    }

    #[test]
    fn player_error_hides_internal_tool_name() {
        let msg = player_unavailable();
        assert!(!msg.to_ascii_lowercase().contains("nanazip"));
        assert!(!msg.to_ascii_lowercase().contains("7-zip"));
        assert!(msg.contains("暫時無法建立分享檔"));
    }
}
