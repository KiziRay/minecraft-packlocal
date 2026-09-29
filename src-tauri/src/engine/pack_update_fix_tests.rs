//! B6a-1 審查修正：F1 不誤刪有效譯文、F2 中途快照、F5 句數算不出不寫、文字來源刷新後 S15 消失。

use super::*;
use crate::engine::session::mods_fingerprint;
use crate::engine::jar_scan::scan_instance;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

fn temp(name: &str) -> PathBuf {
    let root = crate::engine::paths::test_data_base().join(format!("b6a1-fix-{name}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn jar_bytes(ns: &str, en: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let body: serde_json::Map<String, serde_json::Value> =
            en.iter().map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string()))).collect();
        zip.start_file(format!("assets/{ns}/lang/en_us.json"), zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(serde_json::Value::Object(body).to_string().as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn lm(entries: &[(&str, &str, &str)]) -> LangMap {
    let mut m = LangMap::new();
    for (ns, k, v) in entries {
        m.entry(ns.to_string()).or_default().insert(k.to_string(), v.to_string());
    }
    m
}

/// 翻過的包：alpha（人工補翻過）、beta 兩個模組。回（遊戲資料夾, 判斷依據, 舊翻譯）。
fn pack(name: &str) -> (PathBuf, UpdateBasis, LangMap) {
    let mc = temp(name).join("game");
    fs::create_dir_all(mc.join("mods")).unwrap();
    fs::write(mc.join("mods/Alpha-1.0.jar"), jar_bytes("alpha", &[("item.alpha.apple", "Apple")])).unwrap();
    fs::write(mc.join("mods/beta.jar"), jar_bytes("beta", &[("item.beta.gear", "Gear")])).unwrap();
    scan_instance(&mc, &HashMap::new(), false, false, |_, _| {}).unwrap();
    let basis = basis_for(&mc, None, &crate::engine::output_guard::snapshot_sources());
    (mc, basis, lm(&[("alpha", "item.alpha.apple", "人工補翻的蘋果"), ("beta", "item.beta.gear", "齒輪")]))
}

fn refresh_after_rescan(mc: &Path, basis: &UpdateBasis, old_zh: &LangMap) -> Refresh {
    let (zh_scan, en_only, _, report) = scan_instance(mc, &HashMap::new(), false, false, |_, _| {}).unwrap();
    let en_full = crate::engine::output_guard::snapshot_sources();
    let mods_now = mods_now(mc);
    plan_refresh(RefreshInput {
        old_hashes: &basis.source_hashes,
        old_pending: &LangMap::new(),
        en_full: &en_full,
        en_only: &en_only,
        zh_scan: &zh_scan,
        old_zh,
        removal: RemovalCheck { scan_clean: scan_is_clean(&report.errors), old_ns_jars: &basis.ns_jars, mods_now: &mods_now },
    })
}

#[test]
fn b6a1_f1_unreadable_jar_does_not_wipe_its_translations() {
    let (mc, basis, old_zh) = pack("f1-unreadable");
    // 模組檔讀不到（壞掉、網路磁碟抖動、被鎖）：還在 mods/ 裡
    fs::write(mc.join("mods/Alpha-1.0.jar"), b"not a zip").unwrap();
    let r = refresh_after_rescan(&mc, &basis, &old_zh);
    assert!(r.zh.contains_key("alpha"), "讀不到不等於被拿掉，人工補翻不能清掉：{:?}", r.removed_namespaces);
    assert!(r.removed_namespaces.is_empty());
    assert_eq!(r.unconfirmed_removed, vec!["alpha".to_string()], "列「無法確認」");
}

#[test]
fn b6a1_f1_disabled_mod_is_kept() {
    let (mc, basis, old_zh) = pack("f1-disabled");
    fs::rename(mc.join("mods/Alpha-1.0.jar"), mc.join("mods/Alpha-1.0.jar.disabled")).unwrap();
    let r = refresh_after_rescan(&mc, &basis, &old_zh);
    assert!(r.zh.contains_key("alpha"), ".jar.disabled 是暫時停用，不清");
    assert_eq!(r.unconfirmed_removed, vec!["alpha".to_string()]);
}

#[test]
fn b6a1_f1_scan_errors_block_any_removal() {
    let (mc, basis, old_zh) = pack("f1-scan-error");
    fs::remove_file(mc.join("mods/Alpha-1.0.jar")).unwrap();
    let (zh_scan, en_only, _, _) = scan_instance(&mc, &HashMap::new(), false, false, |_, _| {}).unwrap();
    let en_full = crate::engine::output_guard::snapshot_sources();
    let now = mods_now(&mc);
    let errors = vec!["beta.jar: 存取被拒（os error 5）".to_string()];
    let r = plan_refresh(RefreshInput {
        old_hashes: &basis.source_hashes,
        old_pending: &LangMap::new(),
        en_full: &en_full,
        en_only: &en_only,
        zh_scan: &zh_scan,
        old_zh: &old_zh,
        removal: RemovalCheck { scan_clean: scan_is_clean(&errors), old_ns_jars: &basis.ns_jars, mods_now: &now },
    });
    assert!(r.zh.contains_key("alpha"), "掃描有錯誤時一律不移除");
    assert_eq!(r.unconfirmed_removed, vec!["alpha".to_string()]);
    assert!(scan_is_clean(&["掃描快取未能保存（不影響本次翻譯）：x".to_string()]), "不影響翻譯的提醒不算錯誤");
}

#[test]
fn b6a1_f1_really_removed_mod_is_cleared_and_summary_explains_unconfirmed() {
    let (mc, basis, old_zh) = pack("f1-removed");
    assert_eq!(basis.ns_jars.get("alpha"), Some(&vec!["alpha-1.0.jar".to_string()]), "記下哪個模組檔提供這個命名空間");
    fs::remove_file(mc.join("mods/Alpha-1.0.jar")).unwrap();
    let r = refresh_after_rescan(&mc, &basis, &old_zh);
    assert!(!r.zh.contains_key("alpha"));
    assert_eq!(r.removed_namespaces, vec!["alpha".to_string()]);
    assert!(r.zh.contains_key("beta"));
    // 只有上一輪沒記模組檔的舊工作階段：無法確認，一律不移除
    let mut old = basis.clone();
    old.ns_jars.clear();
    let r = refresh_after_rescan(&mc, &old, &old_zh);
    assert!(r.zh.contains_key("alpha") && r.removed_namespaces.is_empty());
    let s = UpdateSummary::from_refresh(RefreshReason::ModsChanged, &r);
    assert_eq!(s.unconfirmed_removed, 1);
    assert!(s.log_line().contains("1 個模組讀不到或暫時停用，舊翻譯先保留"), "{}", s.log_line());
}

#[test]
fn b6a1_f1_full_retranslation_uses_the_same_guard() {
    let (mc, basis, _) = pack("f1-full");
    fs::rename(mc.join("mods/Alpha-1.0.jar"), mc.join("mods/Alpha-1.0.jar.disabled")).unwrap();
    let (zh_scan, _, _, report) = scan_instance(&mc, &HashMap::new(), false, false, |_, _| {}).unwrap();
    let new = hashes_of(&crate::engine::output_guard::snapshot_sources());
    let now = mods_now(&mc);
    let mut prior = lm(&[("alpha", "item.alpha.apple", "蘋果")]);
    let removal = RemovalCheck { scan_clean: scan_is_clean(&report.errors), old_ns_jars: &basis.ns_jars, mods_now: &now };
    assert_eq!(drop_stale(&mut prior, &basis.source_hashes, &new, &zh_scan, &removal), (0, 0));
    assert!(prior.contains_key("alpha"));
}

/// F2：整輪翻譯的中途快照沿用上一份工作階段的判斷依據；崩潰後接續補完仍會重掃、仍丟過期譯文。
#[test]
fn b6a1_f2_interim_snapshot_keeps_the_previous_basis_so_resume_still_rescans() {
    let (mc, basis, _) = pack("f2");
    let prev = TranslateSession {
        version: 1,
        review_pass: 0,
        instance_path: mc.display().to_string(),
        output_dir: String::new(),
        pack_name: "P".into(),
        pack_path: String::new(),
        pending_en: LangMap::new(),
        pending_count: 0,
        quality_deferred: LangMap::new(),
        keys_zh: 0,
        keys_hk_hint: 0,
        note: String::new(),
        target_version: None,
        translation_mode: "append".into(),
        translation_quality: "balanced".into(),
        coverage_tier: "max".into(),
        mods_fingerprint: mods_fingerprint(&mc),
        run_preferences: Default::default(),
        last_run_outcome: crate::engine::session::RunOutcome::Completed,
        update_basis: basis.clone(),
    };
    // 整合包更新（alpha 句子改了）後開始整輪翻譯，本地整理完就崩潰
    fs::write(mc.join("mods/Alpha-1.0.jar"), jar_bytes("alpha", &[("item.alpha.apple", "Green Apple")])).unwrap();
    let (fingerprint, interim) = interim_basis(Some(&prev));
    assert_eq!(interim, basis, "中途快照不寫新依據");
    let mut crashed = prev.clone();
    crashed.mods_fingerprint = fingerprint;
    crashed.update_basis = interim;
    assert_eq!(refresh_reason(&crashed, &mc, None), Some(RefreshReason::ModsChanged), "接續補完仍會重掃");
    let old_zh = lm(&[("alpha", "item.alpha.apple", "蘋果")]);
    let r = refresh_after_rescan(&mc, &crashed.update_basis, &old_zh);
    assert!(r.pending.get("alpha").is_some_and(|m| m.contains_key("item.alpha.apple")), "過期譯文照樣丟掉重翻");
    assert_eq!(interim_basis(None), (0, UpdateBasis::default()), "沒有上一份＝沒有依據（補翻會重掃）");
}

#[test]
fn b6a1_f5_unknown_new_sentence_count_is_not_written() {
    let s = UpdateSummary { changed_sentences: 3, removed_mods: 1, ..Default::default() };
    assert!(!s.log_line().contains("新增"), "整輪翻譯算不出新增幾句：{}", s.log_line());
    assert!(s.log_line().contains("英文改過 3 句"));
    let json = serde_json::to_value(&s).unwrap();
    assert!(json["newSentences"].is_null());
}

/// 審查 F4：另存新結果（新的翻譯結果沒有工作階段）時，讀同一個遊戲資料夾其他翻譯結果的依據（只讀）。
#[test]
fn b6a1_f4_new_result_copy_reads_the_basis_of_the_previous_result() {
    let root = temp("f4");
    let mc = root.join("game");
    fs::create_dir_all(mc.join("mods")).unwrap();
    fs::write(mc.join("mods/a.jar"), jar_bytes("alpha", &[("k", "Apple")])).unwrap();
    let old_work = root.join(crate::engine::out_layout::RESULT_DIR_NAME);
    let basis = UpdateBasis { source_hashes: hashes_of(&lm(&[("alpha", "k", "Apple")])), ..Default::default() };
    let mut s = crate::engine::migrate::read_session_text(&serde_json::json!({
        "version": 1, "instancePath": mc.display().to_string(), "outputDir": "", "packName": "P", "packPath": "",
        "pendingEn": {}, "pendingCount": 0, "keysZh": 0, "note": ""
    }).to_string()).unwrap().0;
    s.update_basis = basis.clone();
    crate::engine::session::save_session(&old_work, &s).unwrap();
    let new_work = root.join("另存").join(crate::engine::out_layout::RESULT_DIR_NAME);
    fs::create_dir_all(&new_work).unwrap();
    let before: Vec<_> = walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok).map(|e| e.path().to_path_buf()).collect();
    let found = prior_session(&mc, &new_work).expect("要找到同一個遊戲資料夾的上一個結果");
    assert_eq!(found.update_basis, basis);
    let after: Vec<_> = walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok).map(|e| e.path().to_path_buf()).collect();
    assert_eq!(before, after, "只讀");
    // 別的遊戲資料夾的結果不算
    let other = root.join("other");
    fs::create_dir_all(other.join("mods")).unwrap();
    assert!(prior_session(&other, &new_work).is_none());
}

/// 未驗證項：只改任務文字時，翻譯更新的部分（接續補完重跑任務）之後產出清單刷新、S15 消失；
/// 新版沒有可翻的字（這輪沒有產出）也一樣消失（產出者已完整處理過新版）。
#[test]
fn b6a1_quest_text_change_is_cleared_after_the_quest_producer_reruns() {
    let root = temp("quests");
    let mc = root.join("game");
    let work = root.join("work");
    let quest = mc.join("config/ftbquests/quests/chapters/intro.snbt");
    fs::create_dir_all(quest.parent().unwrap()).unwrap();
    fs::create_dir_all(&work).unwrap();
    let run = || crate::engine::ftbquests::translate_ftbquests(&mc, &work, false, None, |_, _| {}).unwrap();
    // 簡體任務文字：不用 AI 也會轉成台灣正體寫出
    fs::write(&quest, "{ title: \"苹果任务\" }").unwrap();
    run();
    assert_eq!(crate::engine::text_sources::count_changed_sources(&work, &mc), 0);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fs::write(&quest, "{ title: \"香蕉任务\" }").unwrap();
    assert_eq!(crate::engine::text_sources::count_changed_sources(&work, &mc), 1, "任務文字改了 → S15");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    run();
    assert_eq!(crate::engine::text_sources::count_changed_sources(&work, &mc), 0, "重跑後有新產出：S15 消失");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fs::write(&quest, "{ title: \"Banana quest\" }").unwrap();
    assert_eq!(crate::engine::text_sources::count_changed_sources(&work, &mc), 1);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    run();
    assert_eq!(crate::engine::text_sources::count_changed_sources(&work, &mc), 0, "新版沒有可翻的字（沒有產出）也不再說要翻");
}
