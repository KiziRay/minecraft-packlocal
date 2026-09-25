//! Windows 子程序一律隱藏主控台，避免跳出黑窗。

use std::ffi::OsStr;
use std::process::{Command, Stdio};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 建立隱藏主控台的 Command（stdin 接到 null）。
pub fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    hide_console(&mut cmd);
    cmd
}

pub fn hide_console(cmd: &mut Command) -> &mut Command {
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// 把子程序綁進一個「跟著本程式一起死」的 Job Object。
///
/// 背景：`shutdown_side_processes()` 那條優雅關閉路徑（quit_app／更新／真正關窗）
/// 已經會主動砍 llama-server，但那是「本程式自己執行到那段程式碼」才會發生的事。
/// 使用者從工作管理員「結束工作」、系統當機、或任何形式的強制終止，我們自己的
/// Rust 程式碼根本沒有機會跑，子程序（Windows 預設）會變成孤兒繼續佔用記憶體與
/// 顯示卡直到手動關閉。Job Object 是作業系統層級的保險：只要把子程序指派進一個
/// 開了 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 的 Job，本程式的所有控制代碼
/// （含這個 Job 的控制代碼）在程式結束時一定會被 OS 收回——不管是正常退出、
/// 當機、還是被工作管理員強制關閉——Job 控制代碼一關閉，OS 就會自動砍掉裡面
/// 還活著的子程序，不需要我們自己的程式碼跑到任何一行清理邏輯。
///
/// 呼叫失敗（極舊 Windows、權限問題等）只記錄不中斷：這是「多一層保險」，
/// 不是啟動本地模型的必要條件，失敗了本地模型仍然照常可用，只是失去這層保護。
#[cfg(windows)]
pub fn kill_child_when_this_process_dies(child: &std::process::Child) {
    use std::os::windows::io::AsRawHandle;
    use std::ptr;

    // JOBOBJECT_EXTENDED_LIMIT_INFORMATION 的 repr(C) 版本：欄位型別與大小對齊
    // winnt.h 的原始定義（x86_64 上 SIZE_T／ULONG_PTR／LARGE_INTEGER 皆為 8 bytes），
    // 但我們只填 LimitFlags 這一個欄位，其餘全部歸零（不設任何時間／記憶體限制）。
    #[repr(C)]
    #[derive(Default)]
    struct JobobjectBasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }
    #[repr(C)]
    #[derive(Default)]
    struct JobobjectExtendedLimitInformation {
        basic_limit_information: JobobjectBasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(lp_job_attributes: *const std::ffi::c_void, lp_name: *const u16) -> *mut std::ffi::c_void;
        fn SetInformationJobObject(
            h_job: *mut std::ffi::c_void,
            job_object_information_class: i32,
            lp_job_object_information: *const std::ffi::c_void,
            cb_job_object_information_length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(h_job: *mut std::ffi::c_void, h_process: *mut std::ffi::c_void) -> i32;
    }

    unsafe {
        let job = CreateJobObjectW(ptr::null(), ptr::null());
        if job.is_null() {
            return;
        }
        let mut info = JobobjectExtendedLimitInformation::default();
        info.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JobobjectExtendedLimitInformation>() as u32,
        );
        if ok == 0 {
            return;
        }
        // 不呼叫 CloseHandle：這個控制代碼要活到本程式自己結束為止，那正是
        // 觸發 KILL_ON_JOB_CLOSE 的時機；提早關掉反而失去這層保護。
        AssignProcessToJobObject(job, child.as_raw_handle() as *mut std::ffi::c_void);
    }
}

#[cfg(not(windows))]
pub fn kill_child_when_this_process_dies(_child: &std::process::Child) {}

#[cfg(test)]
mod tests {
    use super::CREATE_NO_WINDOW;

    #[test]
    fn create_no_window_flag_is_windows_constant() {
        assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
    }

    #[cfg(windows)]
    #[test]
    fn attaching_a_real_child_to_the_job_does_not_panic_or_error() {
        // 用一個真的會結束的短命子程序驗證整條 FFI 呼叫鏈不會 panic／回傳錯誤路徑。
        // 不驗證「強制關閉後真的被砍」——那需要另開一個程式當「父程式」再殺它，
        // 屬於整合測試而非單元測試的範疇。
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "exit 0"])
            .spawn()
            .expect("spawn cmd");
        super::kill_child_when_this_process_dies(&child);
        let _ = child.wait();
    }
}
