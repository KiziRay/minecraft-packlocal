//! B6a-2：每一種整檔替換產物，在來源改變後不被套用（列入過期）；來源沒變時正常套用。

use super::tests::{Stage, BACKUP};
use super::*;
use crate::engine::tool_products::test_support::write;
use std::collections::HashSet;
use std::io::{Read, Write};

/// 真正的套用（不經測試用的「先把翻譯結果全部登記成本輪產出」包裝，保留測試自己記的來源指紋）。
fn real_apply(stage: &Stage) -> ApplyResult {
    super::apply_to_instance_with_game_state(&stage.mc, &stage.work, Some("繁體中文翻譯"), BACKUP, GameRunning::No).unwrap()
}

/// 各種整檔替換的產出（翻譯結果與遊戲裡同一個相對路徑）。
const REPLACED: &[&str] = &[
    "config/a.properties",
    "kubejs/client_scripts/a.js",
    "datapacks/p.zip",
    "defaultconfigs/d.toml",
    "global_packs/required_data/x/a.json",
    "paxi/datapacks/p.zip",
    "scripts/a.zs",
    "minemenu/menu.json",
    "data/ns/a.json",
    "assets/ns/b.json",
    "config/ftbquests/quests/chapters/c.snbt",
    "config/openloader/data/pk/pack.zip",
];

fn stage_with_replacements(tag: &str) -> Stage {
    let stage = Stage::new(tag);
    for rel in REPLACED {
        write(&stage.mc.join(rel), &format!("原文:{rel}"));
        write(&stage.work.join(rel), &format!("譯文:{rel}"));
    }
    crate::engine::text_sources::mark_all_produced_for_test(&stage.work, &stage.mc);
    stage
}

#[test]
fn b6a2_every_whole_file_replacement_is_applied_when_its_source_is_unchanged() {
    let stage = stage_with_replacements("b6a2-same");
    let result = real_apply(&stage);
    assert_eq!(result.status, ApplyStatus::Applied);
    assert!(result.outdated_texts.is_empty(), "{:?}", result.outdated_texts);
    for rel in REPLACED {
        assert_eq!(fs::read_to_string(stage.mc.join(rel)).unwrap(), format!("譯文:{rel}"), "{rel} 要放進遊戲");
    }
}

#[test]
fn b6a2_every_whole_file_replacement_is_listed_outdated_when_its_source_changed() {
    let stage = stage_with_replacements("b6a2-changed");
    // 整合包更新：每一個來源都換成新內容（翻譯之後）
    for rel in REPLACED {
        write(&stage.mc.join(rel), &format!("新版:{rel}"));
    }
    let result = real_apply(&stage);
    let listed: HashSet<&str> = result.outdated_texts.iter().map(String::as_str).collect();
    for rel in REPLACED {
        assert!(listed.contains(rel), "{rel} 來源變了，要列入過期：{:?}", result.outdated_texts);
        assert_eq!(fs::read_to_string(stage.mc.join(rel)).unwrap(), format!("新版:{rel}"), "{rel} 不能被舊譯文蓋掉");
    }
}

#[test]
fn b6a2_reapplying_over_the_tool_version_is_not_outdated() {
    let stage = stage_with_replacements("b6a2-reapply");
    real_apply(&stage);
    let again = real_apply(&stage);
    assert!(again.outdated_texts.is_empty(), "已套用後再套用，來源以原檔為準：{:?}", again.outdated_texts);
}

fn zip_bytes(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        for (name, body) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn zip_has(path: &Path, name: &str) -> bool {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let ok = zip.by_name(name).is_ok();
    ok
}

fn zip_read(path: &Path, name: &str) -> String {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut e = zip.by_name(name).unwrap();
    let mut s = String::new();
    e.read_to_string(&mut s).unwrap();
    s
}

const BOOK: &str = "assets/ns/patchouli_books/b/zh_tw/e.json";
const LANG: &str = "assets/ns/lang/zh_tw.json";

/// 主資源包裡有一本書（來源是 mods/book-a.jar，另有 mods/book-b.jar 也提供同命名空間）。
fn stage_with_book(tag: &str) -> Stage {
    let stage = Stage::new(tag).with_zip();
    fs::write(stage.work.join("resourcepacks/繁體中文翻譯.zip"), zip_bytes(&[(BOOK, "{\"name\":\"書\"}"), (LANG, "{\"a\":\"甲\"}")])).unwrap();
    write(&stage.work.join("pack-assets").join(BOOK), "{\"name\":\"書\"}");
    fs::write(stage.mc.join("mods/book-a.jar"), b"book-a-v1").unwrap();
    fs::write(stage.mc.join("mods/book-b.jar"), b"book-b-v1").unwrap();
    let (a, b) = (stage.mc.join("mods/book-a.jar"), stage.mc.join("mods/book-b.jar"));
    crate::engine::pack_books::record_pack_assets(&stage.work, &stage.work, &stage.mc, "jar_patchouli", &|_| {
        vec![(a.clone(), a.clone()), (b.clone(), b.clone())]
    });
    stage
}

#[test]
fn b6a2_book_in_the_main_pack_is_applied_when_its_sources_are_unchanged() {
    let stage = stage_with_book("b6a2-book-same");
    let result = real_apply(&stage);
    assert!(result.outdated_texts.is_empty(), "{:?}", result.outdated_texts);
    let placed = stage.mc.join("resourcepacks/繁體中文翻譯.zip");
    assert!(zip_has(&placed, BOOK) && zip_has(&placed, LANG));
}

#[test]
fn b6a2_book_is_left_out_when_any_source_jar_changed_but_other_entries_stay() {
    for changed in ["book-a.jar", "book-b.jar"] {
        let stage = stage_with_book(&format!("b6a2-book-{changed}"));
        fs::write(stage.mc.join("mods").join(changed), b"updated-by-the-pack").unwrap();
        let result = real_apply(&stage);
        assert!(
            result.outdated_texts.iter().any(|t| t.contains(BOOK)),
            "{changed} 變了，書要列入過期：{:?}",
            result.outdated_texts
        );
        let placed = stage.mc.join("resourcepacks/繁體中文翻譯.zip");
        assert!(!zip_has(&placed, BOOK), "舊書頁不能蓋掉新版");
        assert_eq!(zip_read(&placed, LANG), "{\"a\":\"甲\"}", "清單沒記的項目照放");
        assert!(zip_has(&stage.work.join("resourcepacks/繁體中文翻譯.zip"), BOOK), "翻譯結果裡的原 zip 不動");
    }
}

#[test]
fn b6a2_book_is_left_out_when_its_source_is_gone() {
    let stage = stage_with_book("b6a2-book-gone");
    fs::remove_file(stage.mc.join("mods/book-a.jar")).unwrap();
    let result = real_apply(&stage);
    assert!(result.source_removed_texts.iter().any(|t| t.contains(BOOK)), "來源已被移除要另外列：{:?}", result.source_removed_texts);
    assert!(!zip_has(&stage.mc.join("resourcepacks/繁體中文翻譯.zip"), BOOK));
}

fn tree_hashes(root: &Path) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            (
                e.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"),
                crate::engine::apply_record::file_sha256(e.path()),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
fn b6a2_fix1a_latest_applied_recognises_the_filtered_copy_in_the_game() {
    let stage = stage_with_book("b6a2-fix1a");
    fs::write(stage.mc.join("mods/book-a.jar"), b"updated-by-the-pack").unwrap();
    real_apply(&stage);
    let placed = stage.mc.join("resourcepacks/繁體中文翻譯.zip");
    assert!(!zip_has(&placed, BOOK), "前提：遊戲裡放的是過濾副本");
    assert_eq!(
        crate::engine::result_owner::latest_applied(&stage.mc, &stage.work),
        Some(true),
        "過濾後遊戲裡不是原 zip，B5c 仍要認得這一輪已套用"
    );
}

#[test]
fn b6a2_fix1a_unfiltered_apply_is_still_recognised() {
    let stage = stage_with_book("b6a2-fix1a-plain");
    real_apply(&stage);
    assert_eq!(crate::engine::result_owner::latest_applied(&stage.mc, &stage.work), Some(true));
}

#[test]
fn b6a2_fix1b_the_filtered_copy_is_removed_after_apply_on_success_and_on_failure() {
    let stage = stage_with_book("b6a2-fix1b");
    fs::write(stage.mc.join("mods/book-a.jar"), b"updated-by-the-pack").unwrap();
    real_apply(&stage);
    let dir = crate::engine::pack_books::last_temp_dir_for_test().expect("有產生過濾副本");
    assert!(!dir.exists(), "成功後暫存副本要清掉：{}", dir.display());

    // 失敗路徑：套用在過濾之後、寫入之前就停下（備份政策為先問）也要清掉
    let stage = stage_with_book("b6a2-fix1b-fail");
    fs::write(stage.mc.join("mods/book-a.jar"), b"updated-by-the-pack").unwrap();
    let r = super::apply_to_instance_with_game_state(&stage.mc, &stage.work, Some("繁體中文翻譯"), BackupPolicy::Ask, GameRunning::No).unwrap();
    assert_ne!(r.status, ApplyStatus::Applied);
    let dir = crate::engine::pack_books::last_temp_dir_for_test().expect("有產生過濾副本");
    assert!(!dir.exists(), "停下時暫存副本也要清掉：{}", dir.display());
}

#[test]
fn b6a2_fix1c_unreadable_manifest_keeps_the_games_existing_books_and_lists_them_as_unverifiable() {
    let stage = stage_with_book("b6a2-fix1c");
    real_apply(&stage);
    let placed = stage.mc.join("resourcepacks/繁體中文翻譯.zip");
    assert!(zip_has(&placed, BOOK), "前提：第一次套用已放進書");
    // 產出清單壞掉（網路磁碟讀不到）：無法確認，遊戲裡原有的書頁維持現狀，不被整個換掉
    write(&stage.work.join(".mcpl-text-sources.json"), "{ 壞掉");
    fs::write(stage.work.join("resourcepacks/繁體中文翻譯.zip"), zip_bytes(&[(BOOK, "{\"name\":\"新版書\"}"), (LANG, "{\"a\":\"乙\"}")])).unwrap();
    let r = real_apply(&stage);
    assert!(r.unverifiable_texts.iter().any(|t| t.contains(BOOK)), "{:?}", r.unverifiable_texts);
    assert_eq!(zip_read(&placed, BOOK), "{\"name\":\"書\"}", "維持遊戲現狀（舊書頁），不放新的、也不丟掉");
    assert_eq!(zip_read(&placed, LANG), "{\"a\":\"乙\"}", "語言檔照放");
}

#[test]
fn b6a2_fix1c_unreadable_manifest_first_time_places_no_unverified_book() {
    let stage = stage_with_book("b6a2-fix1c-first");
    write(&stage.work.join(".mcpl-text-sources.json"), "{ 壞掉");
    let r = real_apply(&stage);
    assert!(r.unverifiable_texts.iter().any(|t| t.contains(BOOK)));
    assert!(!zip_has(&stage.mc.join("resourcepacks/繁體中文翻譯.zip"), BOOK), "遊戲裡本來沒有，就不放無法確認的書頁");
}

#[test]
fn b6a2_fix2_changed_removed_and_unverifiable_books_are_listed_separately() {
    let stage = stage_with_book("b6a2-fix2");
    fs::write(stage.mc.join("mods/book-a.jar"), b"updated").unwrap();
    let changed = real_apply(&stage);
    assert!(changed.outdated_texts.iter().any(|t| t.contains(BOOK)) && changed.source_removed_texts.is_empty());
    let stage = stage_with_book("b6a2-fix2-gone");
    fs::remove_file(stage.mc.join("mods/book-a.jar")).unwrap();
    fs::remove_file(stage.mc.join("mods/book-b.jar")).unwrap();
    let gone = real_apply(&stage);
    assert!(gone.source_removed_texts.iter().any(|t| t.contains(BOOK)), "{:?}", gone);
    assert!(gone.outdated_texts.iter().all(|t| !t.contains(BOOK)));
}

#[test]
fn b6a2_fix7_filter_failure_writes_nothing_to_the_game_folder() {
    let stage = stage_with_book("b6a2-fix7");
    fs::write(stage.mc.join("mods/book-a.jar"), b"updated-by-the-pack").unwrap();
    let before = tree_hashes(&stage.mc);
    crate::engine::pack_books::force_filter_failure_for_test(true);
    let result = super::apply_to_instance_with_game_state(&stage.mc, &stage.work, Some("繁體中文翻譯"), BACKUP, GameRunning::No);
    crate::engine::pack_books::force_filter_failure_for_test(false);
    assert!(result.is_err(), "過濾失敗要停下");
    let after = tree_hashes(&stage.mc);
    assert_eq!(before, after, "遊戲資料夾一個檔都不能動（含 .mcpl 標記）");
}

#[test]
fn b6a2_fix2_retired_book_is_not_applied_even_after_a_failed_round_merged_it_back() {
    let stage = stage_with_book("b6a2-retired");
    // 模組被拿掉；jar_patchouli 這輪完整跑完（沒有書了）→ 條目退休。失敗輪把舊書頁併回 pack-assets。
    fs::remove_file(stage.mc.join("mods/book-a.jar")).unwrap();
    fs::remove_file(stage.mc.join("mods/book-b.jar")).unwrap();
    crate::engine::text_sources::begin(&stage.work, "jar_patchouli");
    crate::engine::text_sources::commit(&stage.work, "jar_patchouli", &stage.mc);
    assert!(stage.work.join("pack-assets").join(BOOK).is_file(), "前提：舊書頁還在翻譯結果裡（失敗輪併回）");
    let r = real_apply(&stage);
    assert!(!zip_has(&stage.mc.join("resourcepacks/繁體中文翻譯.zip"), BOOK), "已退休來源的書頁不能套用");
    assert!(r.source_removed_texts.iter().any(|t| t.contains(BOOK)), "{:?}", r.source_removed_texts);
}

#[test]
fn b6a2_fix2_unreadable_game_pack_stops_the_apply_instead_of_silently_dropping_its_books() {
    let stage = stage_with_book("b6a2-unreadable-game-zip");
    write(&stage.work.join(".mcpl-text-sources.json"), "{ 壞掉");
    fs::write(stage.mc.join("resourcepacks/繁體中文翻譯.zip"), b"not a zip").unwrap();
    let before = tree_hashes(&stage.mc);
    let result = super::apply_to_instance_with_game_state(&stage.mc, &stage.work, Some("繁體中文翻譯"), BACKUP, GameRunning::No);
    let err = result.expect_err("讀不到遊戲裡現有的資源包，無法維持現狀，要停下");
    assert!(err.contains("讀不到遊戲裡現有的資源包"), "{err}");
    assert_eq!(before, tree_hashes(&stage.mc), "零寫入");
}

#[test]
fn b6a2_fix2_book_page_with_no_record_in_the_list_is_not_placed() {
    // 記錄來源指紋失敗（或舊版結果）＝清單上沒有這頁的紀錄 → 無法確認，第一次套用不放；語言檔照放。
    let stage = Stage::new("b6a2-unrecorded-book").with_zip();
    fs::write(stage.work.join("resourcepacks/繁體中文翻譯.zip"), zip_bytes(&[(BOOK, "{\"name\":\"書\"}"), (LANG, "{\"a\":\"甲\"}")])).unwrap();
    write(&stage.work.join("pack-assets").join(BOOK), "{\"name\":\"書\"}");
    let r = real_apply(&stage);
    let placed = stage.mc.join("resourcepacks/繁體中文翻譯.zip");
    assert!(!zip_has(&placed, BOOK), "沒有紀錄的書頁不能放");
    assert!(zip_has(&placed, LANG), "語言檔照放");
    assert!(r.unverifiable_texts.iter().any(|t| t.contains(BOOK)), "{:?}", r.unverifiable_texts);
}
