//! B6a-2：略過的產出者不算已更新、舊包不混入、書本輸出每輪清空。

use std::fs;

use super::session::{discover_prior_zh_sources, mods_fingerprint};
use super::text_sources::{begin, commit, count_changed_sources, record, set_skipped_producers};
use super::tool_products::test_support::{temp_game, write};

#[test]
fn b6a2_producers_skipped_by_tier_do_not_count_as_a_pack_update() {
    let mc = temp_game("b6a2-skip");
    let work = temp_game("b6a2-skip-work");
    write(&mc.join("config/ftbquests/q.snbt"), "v1");
    write(&work.join("config/ftbquests/q.snbt"), "譯1");
    begin(&work, "ftbquests");
    let src = mc.join("config/ftbquests/q.snbt");
    record(&work, &work.join("config/ftbquests/q.snbt"), &mc, &src, &src, "ftbquests");
    commit(&work, "ftbquests", &mc);
    assert_eq!(count_changed_sources(&work, &mc), 0, "沒變");
    write(&src, "v2 整合包更新後");
    assert_eq!(count_changed_sources(&work, &mc), 1, "來源變了");
    set_skipped_producers(&work, &["ftbquests"]);
    assert_eq!(count_changed_sources(&work, &mc), 0, "這一輪因完整度略過的來源，不算「整合包已更新」");
    set_skipped_producers(&work, &[]);
    assert_eq!(count_changed_sources(&work, &mc), 1, "下一輪沒略過就照舊判斷");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

fn game_with_mods(tag: &str) -> std::path::PathBuf {
    let mc = temp_game(tag);
    write(&mc.join("mods/a.jar"), "jar-a");
    mc
}

#[test]
fn b6a2_prior_pack_in_the_game_must_belong_to_this_pack() {
    let mc = game_with_mods("b6a2-prior");
    let work = temp_game("b6a2-prior-work");
    let zip = mc.join("resourcepacks/繁體中文翻譯.zip");
    write(&zip.with_extension("zip"), "PK");
    let found = |mc: &std::path::Path| discover_prior_zh_sources(&work, "繁體中文翻譯", "1.0", mc);
    assert!(!found(&mc).iter().any(|p| p == &zip), "沒有標記檔＝無法確認，不接續");
    let fp = mods_fingerprint(&mc);
    assert_ne!(fp, 0);
    write(&mc.join("resourcepacks/繁體中文翻譯.meta.json"), &format!("{{\"modsFingerprint\":{}}}", fp + 1));
    assert!(!found(&mc).iter().any(|p| p == &zip), "別的整合包的舊包不得混入");
    write(&mc.join("resourcepacks/繁體中文翻譯.meta.json"), &format!("{{\"modsFingerprint\":{fp}}}"));
    assert!(found(&mc).iter().any(|p| p == &zip), "指紋相符才接續");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b6a2_prior_result_in_the_tool_folder_is_still_found_without_a_marker() {
    let mc = game_with_mods("b6a2-prior2");
    let work = temp_game("b6a2-prior2-work");
    let zip = work.join("resourcepacks/繁體中文翻譯.zip");
    write(&zip, "PK");
    assert!(discover_prior_zh_sources(&work, "繁體中文翻譯", "1.0", &mc).iter().any(|p| p == &zip));
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

const OLD_BOOK: &str = "assets/ns/patchouli_books/old/zh_tw/e.json";
const NEW_BOOK: &str = "assets/ns/patchouli_books/new/zh_tw/e.json";

#[test]
fn b6a2_fix4_failed_or_stopped_round_keeps_every_old_book() {
    let work = temp_game("b6a2-swap-fail");
    write(&work.join("pack-assets").join(OLD_BOOK), "舊頁");
    {
        let _swap = super::pack_assets::Swap::begin(&work).unwrap();
        assert!(!work.join("pack-assets").join(OLD_BOOK).exists(), "換新期間舊頁在旁邊");
        write(&work.join("pack-assets").join(NEW_BOOK), "新頁（部分完成）");
        // 提早離開（錯誤、停止、AI 不可用都一樣）：沒有呼叫 finish
    }
    assert_eq!(fs::read_to_string(work.join("pack-assets").join(OLD_BOOK)).unwrap(), "舊頁", "舊書頁併回");
    assert!(work.join("pack-assets").join(NEW_BOOK).is_file(), "新寫出的也留著");
    assert!(!work.join("pack-assets.old").exists());
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b6a2_fix4_new_page_wins_over_old_page_on_merge() {
    let work = temp_game("b6a2-swap-new-wins");
    write(&work.join("pack-assets").join(OLD_BOOK), "舊");
    let swap = super::pack_assets::Swap::begin(&work).unwrap();
    write(&work.join("pack-assets").join(OLD_BOOK), "新");
    swap.finish(false, &|_| true);
    assert_eq!(fs::read_to_string(work.join("pack-assets").join(OLD_BOOK)).unwrap(), "新");
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b6a2_fix4_completed_round_drops_only_books_that_are_no_longer_valid() {
    let work = temp_game("b6a2-swap-ok");
    write(&work.join("pack-assets/assets/ns/keep.json"), "仍有效（來源還在、這輪沒重做）");
    write(&work.join("pack-assets/assets/ns/gone.json"), "來源已被拿掉");
    let swap = super::pack_assets::Swap::begin(&work).unwrap();
    write(&work.join("pack-assets").join(NEW_BOOK), "新頁");
    swap.finish(true, &|rel| rel.ends_with("keep.json"));
    assert!(work.join("pack-assets/assets/ns/keep.json").is_file());
    assert!(!work.join("pack-assets/assets/ns/gone.json").exists(), "已失效的舊書頁才真的丟掉");
    assert!(work.join("pack-assets").join(NEW_BOOK).is_file());
    assert!(!work.join("pack-assets.old").exists());
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b6a2_fix4_a_crash_leftover_is_merged_back_on_the_next_round() {
    let work = temp_game("b6a2-swap-crash");
    write(&work.join("pack-assets.old").join(OLD_BOOK), "上次當機留下的舊頁");
    let swap = super::pack_assets::Swap::begin(&work).unwrap();
    swap.finish(false, &|_| true);
    assert!(work.join("pack-assets").join(OLD_BOOK).is_file());
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b6a2_fix4_pack_built_during_the_swap_still_contains_old_and_new_books() {
    let work = temp_game("b6a2-swap-build");
    let pack = temp_game("b6a2-swap-build-pack");
    write(&work.join("pack-assets").join(OLD_BOOK), r#"{"name":"書"}"#);
    let _swap = super::pack_assets::Swap::begin(&work).unwrap();
    write(&work.join("pack-assets").join(NEW_BOOK), r#"{"name":"書"}"#);
    super::pack_assets::copy_into_pack(&work, &pack).unwrap();
    assert!(pack.join(OLD_BOOK).is_file() && pack.join(NEW_BOOK).is_file(), "中途建出的資源包不能少書");
    let _ = fs::remove_dir_all(&work);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn b6a2_fix4_round_counts_as_complete_only_when_all_three_book_producers_committed() {
    let mc = game_with_mods("b6a2-rounds");
    let work = temp_game("b6a2-rounds-work");
    let all = ["jar_patchouli", "overlay", "archive"];
    commit(&work, "overlay", &mc); // 上一輪留下的
    super::text_sources::reset_rounds(&work, &all);
    assert!(!super::text_sources::committed_all(&work, &all));
    begin(&work, "overlay");
    commit(&work, "overlay", &mc);
    begin(&work, "archive");
    super::text_sources::confirm_partial(&work, "archive", &mc); // AI 中途停下：部分完成不算
    assert!(!super::text_sources::committed_all(&work, &all), "部分完成、沒跑的產出者都不算完整");
    // JAR 書本：沒有任何書也要算跑完（模組被拿掉後舊書頁才清得掉）
    super::jar_patchouli::translate_jar_patchouli(&mc, &work, false, None, |_, _| {}).unwrap();
    begin(&work, "archive");
    commit(&work, "archive", &mc);
    assert!(super::text_sources::committed_all(&work, &all));
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

fn write_zip(path: &std::path::Path, files: &[(&str, &str)]) {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, body) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn names(list: &[&str]) -> std::collections::HashSet<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn b6a2_jar_book_records_its_source_jar_and_goes_stale_when_the_jar_is_replaced() {
    let mc = temp_game("b6a2-jarbook");
    let work = temp_game("b6a2-jarbook-work");
    let jar = mc.join("mods/guide.jar");
    write_zip(&jar, &[("assets/example/patchouli_books/guide/en_us/categories/village.json", r#"{"name":"村庄模块"}"#)]);
    super::jar_patchouli::translate_jar_patchouli(&mc, &work, false, None, |_, _| {}).unwrap();
    let book = "assets/example/patchouli_books/guide/zh_tw/categories/village.json";
    assert!(work.join("pack-assets").join(book).is_file());
    let entries = super::text_sources::entries_for(&work).expect("產出清單");
    let entry = entries.get(&format!("pack-assets/{book}")).expect("書本要記來源指紋");
    assert_eq!(entry.source, "mods/guide.jar");
    let record = super::apply_record::ApplyRecord::default();
    let in_zip = names(&[book, "assets/example/lang/zh_tw.json"]);
    assert!(super::pack_books::check_entries(&work, &mc, &record, &in_zip).outdated.is_empty(), "來源沒變");
    write_zip(&jar, &[("assets/example/patchouli_books/guide/en_us/categories/village.json", r#"{"name":"新版本"}"#)]);
    assert_eq!(super::pack_books::check_entries(&work, &mc, &record, &in_zip).outdated, vec![book.to_string()], "JAR 換成新版");
    assert_eq!(count_changed_sources(&work, &mc), 0, "模組換版由 mods 指紋判斷，不重複算成文字來源");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b6a2_zip_book_records_its_source_zip_and_goes_stale_when_the_zip_is_replaced() {
    let mc = temp_game("b6a2-zipbook");
    let work = temp_game("b6a2-zipbook-work");
    let zip = mc.join("datapacks/pack.zip");
    write_zip(
        &zip,
        &[
            ("pack.mcmeta", r#"{"pack":{"pack_format":15,"description":"x"}}"#),
            ("assets/ns/patchouli_books/b/zh_cn/entries/e.json", r#"{"name":"简体书页","pages":[]}"#),
        ],
    );
    super::archive_overlay::translate_archive_overlays(&mc, &work, false, None, |_, _| {}).unwrap();
    let book = "assets/ns/patchouli_books/b/zh_tw/entries/e.json";
    assert!(work.join("pack-assets").join(book).is_file(), "ZIP 的書本放進主資源包");
    let entries = super::text_sources::entries_for(&work).expect("產出清單");
    assert_eq!(entries.get(&format!("pack-assets/{book}")).map(|e| e.source.as_str()), Some("datapacks/pack.zip"));
    let record = super::apply_record::ApplyRecord::default();
    let in_zip = names(&[book]);
    assert!(super::pack_books::check_entries(&work, &mc, &record, &in_zip).outdated.is_empty());
    write_zip(&zip, &[("pack.mcmeta", r#"{"pack":{"pack_format":15,"description":"y"}}"#)]);
    assert_eq!(super::pack_books::check_entries(&work, &mc, &record, &in_zip).outdated, vec![book.to_string()]);
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b6a2_fix3_folder_pack_name_with_a_dot_keeps_its_whole_name_for_the_marker() {
    let mc = game_with_mods("b6a2-dotname");
    let fp = mods_fingerprint(&mc);
    let folder = mc.join("resourcepacks/MyPack-1.0");
    fs::create_dir_all(&folder).unwrap();
    write(&mc.join("resourcepacks/MyPack-1.0.meta.json"), &format!("{{\"modsFingerprint\":{fp}}}"));
    assert!(super::session::existing_pack_matches_current_mods(&mc, &folder, &mc), "標記檔名是完整的 MyPack-1.0.meta.json");
    // 只去掉最後的 .zip
    write(&mc.join("resourcepacks/Other-2.1.zip"), "PK");
    write(&mc.join("resourcepacks/Other-2.1.meta.json"), &format!("{{\"modsFingerprint\":{fp}}}"));
    assert!(super::session::existing_pack_matches_current_mods(&mc, &mc.join("resourcepacks/Other-2.1.zip"), &mc));
    let _ = fs::remove_dir_all(&mc);
}

#[test]
fn b6a2_fix6_sources_changed_under_a_tier_skipped_producer_are_counted_separately() {
    let mc = temp_game("b6a2-skipcount");
    let work = temp_game("b6a2-skipcount-work");
    write(&mc.join("config/ftbquests/q.snbt"), "v1");
    write(&mc.join("kubejs/a.js"), "v1");
    for (producer, rel) in [("ftbquests", "config/ftbquests/q.snbt"), ("scripts", "kubejs/a.js")] {
        write(&work.join(rel), "譯");
        begin(&work, producer);
        record(&work, &work.join(rel), &mc, &mc.join(rel), &mc.join(rel), producer);
        commit(&work, producer, &mc);
    }
    write(&mc.join("config/ftbquests/q.snbt"), "v2");
    write(&mc.join("kubejs/a.js"), "v2");
    set_skipped_producers(&work, &["ftbquests"]);
    assert_eq!(count_changed_sources(&work, &mc), 1, "只有這輪有跑的產出者算「要翻」");
    assert_eq!(super::text_sources::count_skipped_changed_sources(&work, &mc), 1, "略過的另外數：來源已變、因完整度設定沒翻");
    set_skipped_producers(&work, &[]);
    assert_eq!(super::text_sources::count_skipped_changed_sources(&work, &mc), 0);
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b6a2_fix6_the_pack_update_view_carries_the_skipped_count() {
    let view = super::pack_update::UpdateView { skipped_changed: 2, ..Default::default() };
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["skippedChanged"], 2);
    assert!(!view.updated(), "只有略過來源有變，不會讓 S15 出現（那是 B6a-1 留下的問題）");
}
