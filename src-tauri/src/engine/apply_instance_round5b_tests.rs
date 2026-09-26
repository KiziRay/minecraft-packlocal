//! 第五輪（續）：字體包與翻譯分開移除；語言標記重建連回批次；備份刪除後的說明。

use super::*;
use super::tests::{Stage, BACKUP};

fn font_pack(stage: &Stage) -> PathBuf {
    let font = stage.root.join("字體結果").join("resourcepacks").join("繁體中文遊戲字體");
    fs::create_dir_all(font.join("assets/minecraft/font")).unwrap();
    fs::write(font.join("assets/minecraft/font/default.json"), "{}").unwrap();
    font
}

fn apply_font(stage: &Stage) {
    crate::engine::font_pack::apply_font_pack_with_game_state(&stage.mc, &font_pack(stage), GameRunning::No).unwrap();
}

const FONT_ENTRY: &str = "file/繁體中文遊戲字體";
const FONT_FILE: &str = "resourcepacks/繁體中文遊戲字體/assets/minecraft/font/default.json";

// ─── 9. 字體包與翻譯分開 ───

#[test]
fn r9_removing_translation_keeps_font_pack() {
    let stage = Stage::new("r9-keep-font").with_zip();
    stage.apply(BACKUP);
    apply_font(&stage);
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original", "翻譯要移除");
    assert!(stage.mc.join(FONT_FILE).is_file(), "移除翻譯不能拿掉字體包");
    assert!(stage.options().contains(FONT_ENTRY), "移除翻譯不能把字體包從清單拿掉：{}", stage.options());
}

#[test]
fn r9_removing_font_pack_keeps_translation() {
    let stage = Stage::new("r9-keep-translation").with_zip();
    stage.apply(BACKUP);
    apply_font(&stage);
    let summary = crate::engine::font_restore::remove_font_pack_in(&stage.mc).unwrap();
    assert!(summary.contains("字體"), "{summary}");
    assert!(!stage.mc.join(FONT_FILE).exists(), "字體包要拿掉");
    assert!(!stage.options().contains(FONT_ENTRY), "{}", stage.options());
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated", "移除字體包不能動翻譯");
    assert!(stage.options().contains("file/繁體中文翻譯.zip"), "{}", stage.options());
    assert!(stage.options().contains("lang:zh_tw"), "{}", stage.options());
    // 之後移除翻譯仍正常
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

// ─── 11. 小項 ───

#[test]
fn r11_rebuilt_lang_marker_links_to_its_batch() {
    let stage = Stage::new("r11-lang");
    stage.apply(BACKUP);
    fs::remove_file(stage.mc.join(".mcpl/options.json")).unwrap();
    // 第一次：補回語言標記（這次不動）；第二次：語言改回英文
    let _ = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    let second = restore_last_apply_in(&stage.mc, Some(&stage.work));
    let options = stage.options();
    assert!(options.contains("lang:en_us"), "補回的語言標記要連回它那次套用，第二次才能改回：{options} {second:?}");
}

#[test]
fn r11_removal_after_deleting_backups_says_backups_deleted() {
    let stage = Stage::new("r11-bak-deleted");
    stage.apply(BACKUP);
    delete_apply_backups_in(&stage.mc, Some(&stage.work)).unwrap();
    let restored = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert!(
        restored.player_summary.contains("備份已刪除，無法還原被覆蓋的檔案"),
        "{}",
        restored.player_summary
    );
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated");
}
