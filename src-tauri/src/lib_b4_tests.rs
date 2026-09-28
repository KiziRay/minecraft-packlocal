//! B4：一鍵翻譯／補翻在 AI 失敗、停止時不丟已翻部分（lib.rs 需要 AppHandle，無法直接執行；
//! 這裡釘住流程上的關鍵點，防止之後改回「return Err 丟掉整輪」）。

fn body_of<'a>(src: &'a str, name: &str) -> &'a str {
    let body = &src[src.find(name).unwrap_or_else(|| panic!("找不到 {name}"))..];
    &body[..body.find("\n}\n").unwrap_or(body.len())]
}

fn lib_src() -> String {
    std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("lib.rs"))
        .unwrap()
        .replace("\r\n", "\n")
}

#[test]
fn b4_one_click_keeps_partial_results_on_ai_failure_and_stop() {
    let lib = lib_src();
    let body = body_of(&lib, "fn run_one_click(");
    for gone in [
        "dev_progress::finish(\"cancel:ai_fill\")",
        "dev_progress::finish(\"error:ai_fill\")",
        "return Err(format!(\"AI 翻譯失敗：{CANCEL_MESSAGE}\"))",
        "AUTO_RETRY_CAP: usize = 400",
    ] {
        assert!(!body.contains(gone), "run_one_click 不可再有：{gone}");
    }
    assert!(body.contains("begin_user_stop_finalize("), "停止要改成寫出已翻部分並套用");
    assert!(body.contains("RunOutcome::Aborted"), "停止要存成可接續的工作階段");
    assert!(body.contains("proportional_cap("), "自動重試上限要按比例");
    assert!(body.contains("run_interrupt::reset()"), "每輪開始要清掉上一輪的 AI 停止狀態");
    assert!(body.contains("interruption: engine::run_interrupt::view("), "結果要帶出中途停下與沒回應的資料");
}

#[test]
fn b4_supplement_keeps_partial_results_and_skips_extras_after_stop() {
    let lib = lib_src();
    let body = body_of(&lib, "fn run_supplement(");
    assert!(body.contains("run_interrupt::reset()"));
    assert!(body.contains("begin_user_stop_finalize("));
    assert!(body.contains("interruption: engine::run_interrupt::view("));
}

#[test]
fn b4_retrying_messages_show_the_retrying_state() {
    for msg in [
        "AI 連線中斷，自動重試中（第 2 次，等 1.6 秒）…",
        "AI 沒回應的 3 句：同一輪縮小批次重送（第 1 次，每批約原本的 1/2）…",
    ] {
        assert_eq!(super::infer_progress_hint(msg).state, Some(super::STATE_RETRYING), "{msg}");
    }
}
