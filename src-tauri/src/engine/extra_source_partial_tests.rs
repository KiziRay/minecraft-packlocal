//! B4 #3：額外來源（任務、覆寫、任務書）AI 中途停下時，寫出已命中的部分；
//! 部分完成的產出者不 commit（不做退休判斷），改用 `confirm_partial` 讓已寫出的檔能裝進遊戲。

use std::fs;

use crate::engine::run_interrupt;
use crate::engine::shared_tm::{OfflineForTest, SkipSharedLookupGuard};
use crate::engine::text_sources::{self, begin, commit, confirm_partial, is_confirmed_output, record, retire_candidates};
use crate::engine::tm::Tm;
use crate::engine::tool_products::test_support::{temp_game, write};

/// 測試環境：不上網、不查共享庫、AI 視為這一輪已停（只剩免費資料層）。
struct Offline {
    _net: OfflineForTest,
    _lookup: SkipSharedLookupGuard,
}

fn offline_with_ai_halted() -> Offline {
    run_interrupt::reset();
    run_interrupt::halt_ai("測試：額度用完");
    Offline {
        _net: OfflineForTest::enter(),
        _lookup: SkipSharedLookupGuard::enter(true),
    }
}

impl Drop for Offline {
    fn drop(&mut self) {
        run_interrupt::reset();
    }
}

fn seed_tm(pairs: &[(&str, &str)]) {
    let mut tm = Tm::load();
    for (en, zh) in pairs {
        tm.insert(en, zh);
    }
    let _ = tm.save();
}

#[test]
fn partial_confirm_adds_outputs_but_never_retires_anything() {
    let mc = temp_game("b4partial");
    let work = temp_game("b4partial-work");
    write(&mc.join("a/en.json"), "x");
    write(&mc.join("b/en.json"), "x");
    write(&work.join("a/zh.json"), "y");
    write(&work.join("b/zh.json"), "y");
    begin(&work, "p");
    record(&work, &work.join("a/zh.json"), &mc, &mc.join("a/en.json"), &mc.join("a/en.json"), "p");
    record(&work, &work.join("b/zh.json"), &mc, &mc.join("b/en.json"), &mc.join("b/en.json"), "p");
    commit(&work, "p", &mc);

    // 第二輪中途停下：只重寫了 a，而且 b 的來源剛好被整合包移除
    fs::remove_file(mc.join("b/en.json")).unwrap();
    write(&work.join("a/zh.json"), "y2");
    begin(&work, "p");
    record(&work, &work.join("a/zh.json"), &mc, &mc.join("a/en.json"), &mc.join("a/en.json"), "p");
    confirm_partial(&work, "p", &mc);

    assert!(is_confirmed_output(&work, &work.join("a/zh.json")), "這一輪寫出的要能裝進遊戲");
    assert!(is_confirmed_output(&work, &work.join("b/zh.json")), "沒跑完：上一輪的條目照樣有效");
    assert!(retire_candidates(&work).is_empty(), "沒跑完不可以做退休判斷");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn quests_keep_what_the_free_layers_found_when_ai_stops() {
    let _env = offline_with_ai_halted();
    seed_tm(&[("Zq Partial Alpha", "測試部分甲")]);
    let mc = temp_game("b4ftb");
    let work = temp_game("b4ftb-work");
    write(
        &mc.join("config/ftbquests/quests/lang/en_us.snbt"),
        "{\n\tquest.AAAA.title: \"Zq Partial Alpha\"\n\tquest.BBBB.title: \"Zq Partial Beta\"\n}\n",
    );
    write(&mc.join("config/ftbquests/quests/chapters/a.snbt"), "{\n\tid: \"AAAA\"\n}\n");
    let result = crate::engine::ftbquests::translate_ftbquests(&mc, &work, true, None, |_, _| {})
        .expect("AI 停下不可以讓整個任務來源失敗、丟掉已命中的部分");
    let out = work.join("config/ftbquests/quests/lang/zh_tw.snbt");
    let zh = fs::read_to_string(&out).expect("已命中的部分要寫出");
    assert!(zh.contains("測試部分甲"), "{zh}");
    assert!(zh.contains("Zq Partial Beta"), "沒翻到的保留英文：{zh}");
    assert!(is_confirmed_output(&work, &out), "部分結果也要能裝進遊戲");
    assert!(result.note.contains("部分"), "{}", result.note);
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn overlay_batches_keep_going_and_write_what_was_found() {
    let _env = offline_with_ai_halted();
    seed_tm(&[("Zq Overlay Found", "測試覆寫甲")]);
    let mc = temp_game("b4overlay");
    let work = temp_game("b4overlay-work");
    write(
        &mc.join("kubejs/assets/zqx/lang/en_us.json"),
        r#"{"a":"Zq Overlay Found","b":"Zq Overlay Missing"}"#,
    );
    let result = crate::engine::text_overlay::translate_text_overlays(&mc, &work, true, None, |_, _| {})
        .expect("AI 停下只跳過沒翻到的，不可以整個來源失敗");
    let out = work.join("kubejs/assets/zqx/lang/zh_tw.json");
    let zh = fs::read_to_string(&out).expect("已命中的部分要寫出");
    assert!(zh.contains("測試覆寫甲"), "{zh}");
    assert!(is_confirmed_output(&work, &out));
    assert!(result.note.contains("部分"), "{}", result.note);
    let _ = text_sources::game_root(&work);
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}
