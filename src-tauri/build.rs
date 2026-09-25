use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    emit_provenance();
    tauri_build::build()
}

/// 把建置來源身分編進 binary。P0-10：沒有這些值就無法判斷一顆 EXE 由哪個 commit 來、
/// 是否含未提交變更。取不到一律填明確 sentinel，**不得靜默留空**。
fn emit_provenance() {
    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());

    // 失效安全方向：git 查不到就當 dirty。寧可擋掉 stable 發布，
    // 也不要把來源不明的建置誤認成乾淨建置（見 release_manifest 的 B2-D 規則）。
    let dirty = match git(&["status", "--porcelain"]) {
        Some(out) => !out.trim().is_empty(),
        None => true,
    };

    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // 通道由建置環境決定，預設 test。stable 必須是明確指定的動作。
    let channel = std::env::var("MCPL_CHANNEL").unwrap_or_else(|_| "test".to_string());

    println!("cargo:rustc-env=MCPL_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=MCPL_GIT_DIRTY={dirty}");
    println!("cargo:rustc-env=MCPL_BUILD_EPOCH={epoch}");
    println!("cargo:rustc-env=MCPL_CHANNEL={channel}");

    emit_rerun_triggers();
    println!("cargo:rerun-if-env-changed=MCPL_CHANNEL");
}

/// 告訴 cargo 什麼時候要重跑這個 build script。
///
/// **不可以用 `../.git`**：在 git worktree 裡 `.git` 是一個檔案，內容是
/// `gitdir: …`，跨 commit 永遠不變。盯著它的話 build script 一次都不會重跑，
/// 於是 EXE 會一直帶著第一次建置時的 commit——**provenance 會說謊**，
/// 而那正是這整套機制要防的事。（實際踩過：v30 建置出來帶的是 Phase 0 的 commit。）
///
/// 用 `git rev-parse --git-path` 取真正的位置，一般 repo 與 worktree 都正確。
fn emit_rerun_triggers() {
    // HEAD：換 commit／換分支會變
    // index：git add／commit 會變，涵蓋 dirty 狀態的變化
    for name in ["HEAD", "index"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            if !path.is_empty() {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
    // HEAD 指向的 ref 檔本身（在同一分支上 commit 時，變的是這個檔）
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(&["rev-parse", "--git-path", &reference]) {
            if !path.is_empty() {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    Some(text.trim().to_string())
}
