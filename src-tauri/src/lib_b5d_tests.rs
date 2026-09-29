//! B5d：選資料夾就判定的後端最小改動（probe 回「有變動」、新唯讀查詢、背景執行＋逾時）。

use super::*;

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

#[test]
fn b5d_probe_says_pack_changed_instead_of_looking_untranslated() {
    let root = std::env::temp_dir().join(format!("mcpl-b5d-probe-changed-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let instance = root.join("instance");
    let work = root.join("翻譯結果");
    fs::create_dir_all(instance.join("mods")).unwrap();
    fs::create_dir_all(work.join("resourcepacks")).unwrap();
    fs::write(instance.join("mods/jei.jar"), vec![0u8; 100]).unwrap();
    fs::write(work.join("resourcepacks/模組包翻譯工具+0902+R1.zip"), b"pack").unwrap();
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
    };
    save_session(&work, &session).unwrap();
    assert_eq!(probe_cache_at(&instance, &work).unwrap().status, "ready", "沒變動時照舊");

    // 加一個模組（已翻好的包更新了）
    fs::write(instance.join("mods/new-mod.jar"), vec![0u8; 200]).unwrap();
    let probe = probe_cache_at(&instance, &work).expect("不能回 None：畫面會像沒翻過");
    assert_eq!(probe.status, "changed");
    assert!(probe.mods_changed);
    assert!(!probe.shareable && !probe.applyable && !probe.matched, "有變動不是可用的結果：{probe:?}");
    assert_eq!(probe.pack_name.as_deref(), Some("ATM10"));
    assert!(probe.message.contains("有變動"), "{}", probe.message);
    let json = serde_json::to_value(&probe).unwrap();
    assert_eq!(json["modsChanged"], true);
    let _ = fs::remove_dir_all(root);
}

// 候選排序（changed 不蓋掉 ready／partial，partial 可取代 changed）改由行為測試
// b5d_fix4_partial_result_replaces_an_earlier_changed_one 守（審查 4：原本的字串比對沒抓到）。

#[test]
fn b5d_folder_queries_run_off_the_main_thread_with_a_deadline() {
    let lib = lib_src();
    for sig in ["async fn inspect_folder_cmd(", "async fn inspect_instance_identity_cmd(", "async fn validate_instance_cmd("] {
        let body = body_of(&lib, sig);
        assert!(body.contains("spawn_blocking") && body.contains("run_with_timeout"), "{sig} 要背景執行並設上限");
    }
    // 選資料夾時會碰遊戲資料夾（可能是網路磁碟）的其他查詢：不在主執行緒跑
    for name in [
        "fn detect_mc_version(",
        "fn detect_pack_translation_name(",
        "fn has_apply_backups_cmd(",
        "fn check_install_target(",
        "fn probe_local_pack_cache_cmd(",
        "fn check_write_access_cmd(",
        "fn common_launcher_dir_cmd(",
    ] {
        let at = lib.find(name).unwrap_or_else(|| panic!("找不到 {name}"));
        let head = &lib[..at];
        let head = &head[head.rfind("
#[").unwrap_or(0)..];
        assert!(head.contains("#[tauri::command(async)]"), "{name} 要 #[tauri::command(async)]");
    }
    let handler = &lib[lib.find("generate_handler![").unwrap()..];
    for cmd in ["inspect_folder_cmd,", "inspect_instance_identity_cmd,", "common_launcher_dir_cmd,"] {
        assert!(handler.contains(cmd), "{cmd} 要註冊");
    }
}

#[test]
fn ro_b5d_inspect_commands_write_nothing() {
    let root = std::env::temp_dir().join(format!("mcpl-b5d-ro-cmd-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let pack = root.join("ATM10");
    fs::create_dir_all(pack.join("mods")).unwrap();
    fs::create_dir_all(pack.join("config")).unwrap();
    fs::write(pack.join("mods/a.jar"), b"pk").unwrap();
    let list = |r: &Path| -> Vec<(PathBuf, Option<std::time::SystemTime>)> {
        walkdir::WalkDir::new(r)
            .into_iter()
            .filter_map(Result::ok)
            .map(|e| (e.path().to_path_buf(), fs::metadata(e.path()).ok().and_then(|m| m.modified().ok())))
            .collect()
    };
    let before = list(&root);
    let path = pack.display().to_string();
    let inspection = tauri::async_runtime::block_on(inspect_folder_cmd(path.clone())).unwrap();
    assert!(inspection.reachable && inspection.validation.ok);
    assert!(!inspection.shape.has_options, "沒 options.txt → 前端出 N-04");
    let identity = tauri::async_runtime::block_on(inspect_instance_identity_cmd(path.clone())).unwrap();
    assert_eq!(identity.state, "new");
    let _ = tauri::async_runtime::block_on(validate_instance_cmd(path.clone())).unwrap();
    let _ = probe_local_pack_cache_cmd(path.clone(), None, None);
    let _ = has_apply_backups_cmd(path.clone(), None);
    let _ = check_write_access_cmd(path);
    assert_eq!(before, list(&root), "選資料夾時的查詢不可寫入遊戲資料夾（不建 .mcpl、不留測試檔）");
    let _ = fs::remove_dir_all(root);
}

// ─── B5d 審查修正 ───────────────────────────────────────

/// 審查 1：「翻譯結果放旁邊」模式選資料夾時只算路徑，不在遊戲資料夾建「繁中翻譯輸出」（G1.36／G5d.1）。
#[test]
fn ro_b5d_fix1_beside_mode_suggestion_writes_nothing() {
    let root = std::env::temp_dir().join(format!("mcpl-b5d-fix1-beside-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let pack = root.join("ATM10");
    fs::create_dir_all(pack.join("mods")).unwrap();
    fs::write(pack.join("mods/a.jar"), b"pk").unwrap();
    let list = |r: &Path| -> Vec<(PathBuf, Option<std::time::SystemTime>)> {
        walkdir::WalkDir::new(r)
            .into_iter()
            .filter_map(Result::ok)
            .map(|e| (e.path().to_path_buf(), fs::metadata(e.path()).ok().and_then(|m| m.modified().ok())))
            .collect()
    };
    let before = list(&root);
    let path = pack.display().to_string();
    let beside = tauri::async_runtime::block_on(suggest_output_dir(path.clone())).unwrap();
    assert!(beside.ends_with("繁中翻譯輸出"), "{beside}");
    let legacy = tauri::async_runtime::block_on(suggest_resourcepacks_dir(path)).unwrap();
    assert_eq!(beside, legacy);
    assert_eq!(before, list(&root), "只算路徑，等開始翻譯時才由 ensure_result_layout 建");
    assert!(!Path::new(&beside).exists());
    let _ = fs::remove_dir_all(root);
}

/// 審查 4：候選位置裡「有變動」排在前面時，後面可接續的 partial 要能取代它（行為測試，走真的 command）。
#[test]
fn b5d_fix4_partial_result_replaces_an_earlier_changed_one() {
    let root = std::env::temp_dir().join(format!("mcpl-b5d-fix4-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let instance = root.join("ATM10");
    fs::create_dir_all(instance.join("mods")).unwrap();
    fs::write(instance.join("mods/jei.jar"), vec![0u8; 100]).unwrap();
    let session = |fingerprint: u64, pending: usize| {
        let mut ns: std::collections::HashMap<String, String> = Default::default();
        for i in 0..pending {
            ns.insert(format!("item.b5d{i}"), format!("Untranslated item {i}"));
        }
        let mut pending_en: engine::LangMap = Default::default();
        pending_en.insert("test".into(), ns);
        TranslateSession {
            version: 1,
            review_pass: 0,
            instance_path: instance.display().to_string(),
            output_dir: String::new(),
            pack_name: "ATM10".into(),
            pack_path: String::new(),
            pending_en,
            pending_count: pending,
            quality_deferred: Default::default(),
            keys_zh: 100,
            keys_hk_hint: 0,
            note: String::new(),
            target_version: None,
            translation_mode: "append".into(),
            translation_quality: "balanced".into(),
            coverage_tier: "max".into(),
            mods_fingerprint: fingerprint,
            run_preferences: engine::RunPreferences::default(),
            last_run_outcome: engine::RunOutcome::Completed,
        }
    };
    let live = engine::mods_fingerprint(&instance);
    // 第一個候選（本包另指定的位置）：舊的、mods 已變
    let hint = root.join("old-result");
    fs::create_dir_all(&hint).unwrap();
    save_session(&hint, &session(live ^ 0xdead, 5)).unwrap();
    assert_eq!(probe_cache_at(&instance, &hint).unwrap().status, "changed");
    // 後面的候選（自訂根目錄）：這次 mods 的、還可以接續
    let base = root.join("custom-base");
    let custom = PathBuf::from(managed_output_for_instance_at(&instance, Some(&base)));
    fs::create_dir_all(&custom).unwrap();
    save_session(&custom, &session(live, 5)).unwrap();
    assert_eq!(probe_cache_at(&instance, &custom).unwrap().status, "partial");

    let best = probe_local_pack_cache_cmd(
        instance.display().to_string(),
        Some(hint.display().to_string()),
        Some(base.display().to_string()),
    )
    .unwrap();
    assert_eq!(best.status, "partial", "可接續的結果不能被「有變動」蓋掉：{best:?}");
    assert!(!best.mods_changed);
    let _ = fs::remove_dir_all(root);
}
