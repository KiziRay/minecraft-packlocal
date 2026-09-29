//! B6a-1：更新偵測（probe 回更新差異）與「翻譯更新的部分」的接線。

use super::*;
use std::io::Write;

fn lib_src() -> String {
    std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("lib.rs"))
        .unwrap()
        .replace("\r\n", "\n")
}

fn body_of<'a>(lib: &'a str, sig: &str) -> &'a str {
    let at = lib.find(sig).unwrap_or_else(|| panic!("找不到 {sig}"));
    let body = &lib[at..];
    &body[..body.find("\n}\n").unwrap()]
}

fn pos(body: &str, needle: &str) -> usize {
    body.find(needle).unwrap_or_else(|| panic!("找不到 {needle}"))
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

/// 翻過一次、已有可用結果的包。回（根, 遊戲資料夾, 翻譯結果）。
fn translated(name: &str, with_basis: bool) -> (PathBuf, PathBuf, PathBuf) {
    let root = engine::paths::test_data_base().join(format!("lib-b6a1-{name}"));
    let _ = fs::remove_dir_all(&root);
    let instance = root.join("instance");
    fs::create_dir_all(instance.join("mods")).unwrap();
    fs::write(instance.join("minecraftinstance.json"), r#"{"gameVersion":"1.20.1"}"#).unwrap();
    fs::write(instance.join("mods/alpha.jar"), jar_bytes("alpha", &[("item.alpha.apple", "Apple")])).unwrap();
    let work = root.join("翻譯結果");
    fs::create_dir_all(work.join("resourcepacks")).unwrap();
    fs::write(work.join("resourcepacks/模組包翻譯工具+0902+R1.zip"), b"pack").unwrap();
    let _ = scan_instance(&instance, &HashMap::new(), false, false, |_, _| {}).unwrap();
    let update_basis = if with_basis {
        engine::pack_update::basis_for(&instance, None, &engine::snapshot_sources())
    } else {
        Default::default()
    };
    let session = TranslateSession {
        version: 1,
        review_pass: 0,
        instance_path: instance.display().to_string(),
        output_dir: String::new(),
        pack_name: "ATM10".into(),
        pack_path: String::new(),
        pending_en: Default::default(),
        pending_count: 0,
        quality_deferred: Default::default(),
        keys_zh: 100,
        keys_hk_hint: 0,
        note: String::new(),
        target_version: None,
        translation_mode: "append".into(),
        translation_quality: "balanced".into(),
        coverage_tier: "max".into(),
        mods_fingerprint: engine::mods_fingerprint(&instance),
        run_preferences: engine::RunPreferences::default(),
        last_run_outcome: engine::RunOutcome::Completed,
        update_basis,
    };
    save_session(&work, &session).unwrap();
    (root, instance, work)
}

#[test]
fn b6a1_probe_reports_how_much_changed_after_a_pack_update() {
    let (root, instance, work) = translated("probe", true);
    let same = probe_cache_at(&instance, &work).unwrap();
    assert_eq!(same.status, "ready", "沒更新照舊");
    assert!(same.pack_update.is_none());
    fs::write(instance.join("mods/beta.jar"), jar_bytes("beta", &[("a", "One"), ("b", "Two")])).unwrap();
    let probe = probe_cache_at(&instance, &work).unwrap();
    assert_eq!(probe.status, "changed");
    assert!(probe.mods_changed && !probe.applyable && !probe.shareable);
    let json = serde_json::to_value(&probe).unwrap();
    assert_eq!(json["packUpdate"]["countsKnown"], true, "{json}");
    assert_eq!(json["packUpdate"]["newMods"], 1);
    assert_eq!(json["packUpdate"]["sentences"], 2);
    assert!(probe.message.contains("翻譯更新的部分"), "{}", probe.message);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn b6a1_probe_says_minecraft_version_changed() {
    let (root, instance, work) = translated("mc", true);
    fs::write(instance.join("minecraftinstance.json"), r#"{"gameVersion":"1.21.1"}"#).unwrap();
    let probe = probe_cache_at(&instance, &work).unwrap();
    assert_eq!(probe.status, "changed");
    assert!(!probe.mods_changed, "模組沒變");
    let json = serde_json::to_value(&probe).unwrap();
    assert_eq!(json["packUpdate"]["mcChanged"], true);
    assert_eq!(json["packUpdate"]["mcBefore"], "1.20.1");
    assert_eq!(json["packUpdate"]["mcNow"], "1.21.1");
    assert!(probe.message.contains("Minecraft 版本變了"), "{}", probe.message);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn b6a1_pack_translated_by_an_older_version_is_not_misjudged() {
    let (root, instance, work) = translated("old", false);
    // 舊工作階段：沒有英文雜湊、MC 版本、模組清單；版本換了也不判「版本變了」
    fs::write(instance.join("minecraftinstance.json"), r#"{"gameVersion":"1.21.1"}"#).unwrap();
    let probe = probe_cache_at(&instance, &work).unwrap();
    assert_eq!(probe.status, "ready", "舊版翻過的包、模組沒變：照舊是可用的結果");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ro_b6a1_update_detection_writes_nothing() {
    let (root, instance, _work) = translated("ro", true);
    fs::write(instance.join("mods/beta.jar"), jar_bytes("beta", &[("a", "One")])).unwrap();
    let list = |r: &Path| -> Vec<(PathBuf, Option<std::time::SystemTime>)> {
        let mut v: Vec<_> = walkdir::WalkDir::new(r)
            .into_iter()
            .filter_map(Result::ok)
            .map(|e| (e.path().to_path_buf(), fs::metadata(e.path()).ok().and_then(|m| m.modified().ok())))
            .collect();
        v.sort();
        v
    };
    let before = list(&root);
    let probe = probe_local_pack_cache_cmd(instance.display().to_string(), Some(root.join("翻譯結果").display().to_string()), None)
        .unwrap();
    assert_eq!(probe.status, "changed");
    assert_eq!(before, list(&root), "選資料夾時的更新偵測不可寫入（遊戲資料夾與翻譯結果都一樣）");
    let _ = fs::remove_dir_all(root);
}

/// 「翻譯更新的部分」＝接續補完先重掃：要在算待補、載入英文原文表、改寫模組檔之前。
#[test]
fn b6a1_supplement_rescans_before_computing_what_to_translate() {
    let lib = lib_src();
    let sup = body_of(&lib, "fn run_supplement(");
    let refresh = pos(sup, "refresh_after_pack_update(app, &work, &mut session, &mut zh, &dict)?");
    assert!(pos(sup, "load_pack_zh(&pack_path)?") < refresh, "先讀舊翻譯，再重掃比對");
    assert!(refresh < pos(sup, "let mut pending = remaining_pending(&session.pending_en, &zh);"));
    assert!(refresh < pos(sup, "prepare_build_sources("), "英文原文表要先刷新再載入（G2.15）");
    assert!(refresh < pos(sup, "rewrite_jars_and_log("), "模組檔以重掃後的翻譯改寫");
    for built in ["pack_update: pack_update.clone(),"] {
        assert_eq!(sup.matches(built).count(), 2, "兩個出口都帶 packUpdate");
    }
    let body = body_of(&lib, "fn refresh_after_pack_update(");
    assert!(pos(body, "refresh_reason(") < pos(body, "scan_instance("), "有更新（或舊工作階段）才重掃");
    assert!(pos(body, "scan_instance(") < pos(body, "source_catalog_save(") , "重掃後刷新英文原文表");
    assert!(pos(body, "source_catalog_save(") < pos(body, "plan_refresh("));
    for field in ["session.pending_en =", "session.mods_fingerprint =", "session.update_basis =", "forget_keys(&mut session.quality_deferred"] {
        assert!(body.contains(field), "重掃後要更新 {field}");
    }
}

#[test]
fn b6a1_full_translation_records_the_basis_and_drops_stale_prior_translations() {
    let lib = lib_src();
    let one = body_of(&lib, "fn run_one_click(");
    let basis = pos(one, "engine::pack_update::basis_for(&instance, Some(&work), &engine::snapshot_sources())");
    assert!(pos(one, "scan_instance(&instance, &dict, true, true") < basis, "掃描完才有完整英文表");
    let drop = pos(one, "engine::pack_update::drop_stale(&mut prior");
    assert!(drop < pos(one, "let n = merge_fill_missing(&mut zh, &prior);"), "英文改過的舊譯文不併入");
    assert_eq!(one.matches("update_basis: update_basis.clone(),").count(), 1, "新依據只在整輪跑完的結尾寫");
    assert!(one.contains("pack_update: one_click_update,"));
}

/// 審查 F2：本地整理完的中途快照（Crashed）沿用上一份工作階段的指紋與依據，不寫新依據。
#[test]
fn b6a1_f2_interim_snapshot_does_not_record_the_new_basis() {
    let lib = lib_src();
    let one = body_of(&lib, "fn run_one_click(");
    let crashed = pos(one, "last_run_outcome: engine::RunOutcome::Crashed,");
    let snapshot = &one[one[..crashed].rfind("&TranslateSession {").unwrap()..crashed + 200];
    assert!(snapshot.contains("update_basis: interim_basis,"), "中途快照不能寫新依據");
    assert!(snapshot.contains("mods_fingerprint: interim_fingerprint,"), "中途快照不能寫新指紋");
    assert!(pos(one, "engine::pack_update::interim_basis(load_session(&work)") < crashed, "取上一份要在寫中途快照之前");
    let finished = pos(one, "engine::RunOutcome::Aborted");
    assert!(pos(&one[finished..], "update_basis: update_basis.clone(),") > 0, "結尾寫新依據");
}

/// 審查 F4／F1：整輪翻譯接續舊譯文用上一輪依據（另存新結果時也找得到），移除模組前先確認。
#[test]
fn b6a1_f4_full_translation_uses_the_prior_result_basis_and_the_removal_guard() {
    let lib = lib_src();
    let one = body_of(&lib, "fn run_one_click(");
    let prior = pos(one, "engine::pack_update::prior_session(&instance, &work)");
    assert!(prior < pos(one, "engine::pack_update::drop_stale(&mut prior"));
    assert!(one.contains("let old_hashes = &prior_basis.source_hashes;"));
    assert!(one.contains("old_ns_jars: &prior_basis.ns_jars"));
    assert!(one.contains("engine::pack_update::scan_is_clean(&report.errors)"));
    let sup = body_of(&lib, "fn refresh_after_pack_update(");
    assert!(sup.contains("scan_is_clean(&report.errors)") && sup.contains("old_ns_jars: &session.update_basis.ns_jars"));
    assert!(!sup.contains("_report"), "掃描錯誤不能被忽略");
}
