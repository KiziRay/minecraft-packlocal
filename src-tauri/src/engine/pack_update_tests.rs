//! B6a-1：整合包更新偵測（唯讀）與「翻譯更新的部分」的待補計算。

use super::*;
use crate::engine::session::mods_fingerprint;
use crate::engine::jar_scan::scan_instance;
use crate::engine::session::{save_session, RunOutcome, RunPreferences};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

fn temp(name: &str) -> PathBuf {
    let root = crate::engine::paths::test_data_base().join(format!("b6a1-{name}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn jar_bytes(ns: &str, en: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        let body: serde_json::Map<String, serde_json::Value> =
            en.iter().map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string()))).collect();
        zip.start_file(format!("assets/{ns}/lang/en_us.json"), opts).unwrap();
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

fn has(map: &LangMap, ns: &str, k: &str) -> bool {
    map.get(ns).is_some_and(|m| m.contains_key(k))
}

fn session_for(instance: &Path, basis: UpdateBasis, pending: LangMap) -> TranslateSession {
    TranslateSession {
        version: 1,
        review_pass: 0,
        instance_path: instance.display().to_string(),
        output_dir: String::new(),
        pack_name: "ATM10".into(),
        pack_path: String::new(),
        pending_count: 0,
        pending_en: pending,
        quality_deferred: Default::default(),
        keys_zh: 10,
        keys_hk_hint: 0,
        note: String::new(),
        target_version: None,
        translation_mode: "append".into(),
        translation_quality: "balanced".into(),
        coverage_tier: "max".into(),
        mods_fingerprint: mods_fingerprint(instance),
        run_preferences: RunPreferences::default(),
        last_run_outcome: RunOutcome::Completed,
        update_basis: basis,
    }
}

/// 翻過一次的整合包：模組 alpha（兩句）＋MC 1.20.1。回（遊戲資料夾, 翻譯結果, 工作階段）。
fn translated_pack(name: &str) -> (PathBuf, PathBuf, TranslateSession) {
    let root = temp(name);
    let mc = root.join("game");
    fs::create_dir_all(mc.join("mods")).unwrap();
    fs::write(mc.join("minecraftinstance.json"), r#"{"gameVersion":"1.20.1"}"#).unwrap();
    fs::write(mc.join("mods/alpha.jar"), jar_bytes("alpha", &[("item.alpha.apple", "Apple"), ("item.alpha.pear", "Pear")]))
        .unwrap();
    let work = root.join("翻譯結果");
    fs::create_dir_all(&work).unwrap();
    let (_, _, _, _) = scan_instance(&mc, &HashMap::new(), false, false, |_, _| {}).unwrap();
    let basis = basis_for(&mc, None, &crate::engine::output_guard::snapshot_sources());
    assert!(!basis.source_hashes.is_empty() && !basis.mod_files.is_empty());
    assert_eq!(basis.mc_version.as_deref(), Some("1.20.1"));
    let session = session_for(&mc, basis, LangMap::new());
    save_session(&work, &session).unwrap();
    (mc, work, session)
}

fn snapshot(root: &Path) -> Vec<(PathBuf, u64, Option<std::time::SystemTime>)> {
    let mut out: Vec<_> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .map(|e| {
            let meta = fs::metadata(e.path()).ok();
            (e.path().to_path_buf(), meta.as_ref().map(|m| m.len()).unwrap_or(0), meta.and_then(|m| m.modified().ok()))
        })
        .collect();
    out.sort();
    out
}

#[test]
fn b6a1_session_stores_english_hashes_mc_version_and_mod_list_and_old_sessions_still_load() {
    let (mc, work, _) = translated_pack("session-json");
    let text = fs::read_to_string(work.join(crate::engine::session::SESSION_FILE)).unwrap();
    let raw: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(raw["sourceHashes"]["alpha"]["item.alpha.apple"].is_string(), "{raw}");
    assert_eq!(raw["mcVersion"], "1.20.1");
    assert!(raw["modFiles"]["alpha.jar"].is_u64());
    assert_eq!(raw["nsJars"]["alpha"][0], "alpha.jar");
    assert!(!crate::engine::migrate::session_needs_rescan(&raw), "有英文雜湊就不需重掃");
    // 舊工作階段（沒有這些欄位）照常讀、欄位是空的，並標記需重掃
    let mut old = raw.clone();
    for k in ["sourceHashes", "mcVersion", "modFiles", "nsJars"] {
        old.as_object_mut().unwrap().remove(k);
    }
    let (loaded, needs) = crate::engine::migrate::read_session_text(&old.to_string()).unwrap();
    assert!(needs);
    assert_eq!(loaded.update_basis, UpdateBasis::default());
    assert_eq!(refresh_reason(&loaded, &mc, None), Some(RefreshReason::NoBaseline), "[B0] 需重掃旗標：補翻時會重掃一次");
}

#[test]
fn b6a1_detection_counts_new_and_updated_mods_without_writing_anything() {
    let (mc, work, session) = translated_pack("detect");
    let untouched = inspect(&session, &mc, &work);
    assert!(!untouched.updated(), "沒更新不能說已更新：{untouched:?}");
    assert_eq!(refresh_reason(&session, &mc, None), None);
    // 整合包更新：alpha 多一句、改一句；新增模組 beta（三句）
    fs::write(
        mc.join("mods/alpha.jar"),
        jar_bytes("alpha", &[("item.alpha.apple", "Apple"), ("item.alpha.pear", "Pear (ripe)"), ("item.alpha.plum", "Plum")]),
    )
    .unwrap();
    fs::write(mc.join("mods/beta.jar"), jar_bytes("beta", &[("a", "One"), ("b", "Two"), ("c", "Three")])).unwrap();
    let root = mc.parent().unwrap().to_path_buf();
    let before = snapshot(&root);
    let view = inspect(&session, &mc, &work);
    assert_eq!(before, snapshot(&root), "更新偵測零寫入（G1.36）");
    assert!(view.mods_changed && view.updated() && view.counts_known, "{view:?}");
    assert_eq!(view.new_mods, 1);
    assert_eq!(view.updated_mods, 1);
    assert_eq!(view.sentences, 5, "新模組 3 句＋alpha 新增 1 句＋改過 1 句；沒變的那句不算");
    assert!(!view.mc_changed);
    assert_eq!(refresh_reason(&session, &mc, None), Some(RefreshReason::ModsChanged));
    let json = serde_json::to_value(&view).unwrap();
    for k in ["modsChanged", "countsKnown", "newMods", "updatedMods", "sentences", "textsChanged", "mcChanged"] {
        assert!(json.get(k).is_some(), "前端讀 {k}：{json}");
    }
}

#[test]
fn b6a1_minecraft_version_change_is_reported_only_when_both_sides_are_known() {
    let (mc, work, session) = translated_pack("mc-version");
    fs::write(mc.join("minecraftinstance.json"), r#"{"gameVersion":"1.21.1"}"#).unwrap();
    let view = inspect(&session, &mc, &work);
    assert!(view.mc_changed && view.updated(), "{view:?}");
    assert_eq!(view.mc_before.as_deref(), Some("1.20.1"));
    assert_eq!(view.mc_now.as_deref(), Some("1.21.1"));
    // 舊版翻過的包（工作階段沒記版本、沒記指紋）：不判版本變了、不判已更新
    let mut old = session.clone();
    old.update_basis = UpdateBasis::default();
    old.mods_fingerprint = 0;
    let view = inspect(&old, &mc, &work);
    assert!(!view.mc_changed && !view.updated() && !view.counts_known, "舊工作階段不能被誤判：{view:?}");
    // 有記指紋、沒記雜湊與模組清單：看得出有變動，但算不出句數（前端不寫數字）
    fs::write(mc.join("mods/beta.jar"), jar_bytes("beta", &[("a", "One")])).unwrap();
    let mut legacy = session.clone();
    legacy.update_basis = UpdateBasis::default();
    let view = inspect(&legacy, &mc, &work);
    assert!(view.mods_changed && !view.counts_known && view.sentences == 0, "{view:?}");
}

#[test]
fn b6a1_changed_quest_text_is_counted_read_only() {
    let (mc, work, session) = translated_pack("texts");
    let quest = mc.join("config/ftbquests/quests/chapters/intro.snbt");
    fs::create_dir_all(quest.parent().unwrap()).unwrap();
    fs::write(&quest, "{ title: \"Hello\" }").unwrap();
    let out = work.join("config/ftbquests/quests/chapters/intro.snbt");
    fs::create_dir_all(out.parent().unwrap()).unwrap();
    fs::write(&out, "{ title: \"你好\" }").unwrap();
    crate::engine::text_sources::record(&work, &out, &mc, &quest, &quest, "quests");
    assert_eq!(inspect(&session, &mc, &work).texts_changed, 0);
    fs::write(&quest, "{ title: \"Hello there\" }").unwrap();
    let root = mc.parent().unwrap().to_path_buf();
    let before = snapshot(&root);
    let view = inspect(&session, &mc, &work);
    assert_eq!(before, snapshot(&root), "零寫入");
    assert_eq!(view.texts_changed, 1);
    assert!(view.updated() && !view.mods_changed, "任務文字改了也算已更新：{view:?}");
}

/// 「翻譯更新的部分」：新增模組的句子、英文改過的句子、上次的缺口要翻；沒變的舊譯文照用；拿掉的模組清掉。
#[test]
fn b6a1_refresh_translates_new_mods_and_changed_sentences_only() {
    let (mc, _work, session) = translated_pack("refresh");
    // 上次的翻譯：alpha 兩句都翻了，另有 gone 模組（整合包之後拿掉）
    let old_zh = lm(&[("alpha", "item.alpha.apple", "蘋果"), ("alpha", "item.alpha.pear", "梨子"), ("gone", "x", "舊的")]);
    let mut basis = session.update_basis.clone();
    basis.source_hashes.entry("gone".into()).or_default().insert("x".into(), text_hash("Old"));
    basis.ns_jars.insert("gone".into(), vec!["gone.jar".into()]);
    // 整合包更新
    fs::write(
        mc.join("mods/alpha.jar"),
        jar_bytes("alpha", &[("item.alpha.apple", "Apple"), ("item.alpha.pear", "Pear (ripe)")]),
    )
    .unwrap();
    fs::write(mc.join("mods/beta.jar"), jar_bytes("beta", &[("item.beta.one", "Copper Gear"), ("item.beta.two", "Iron Gear")]))
        .unwrap();
    let (zh_scan, en_only, _, _) = scan_instance(&mc, &HashMap::new(), false, false, |_, _| {}).unwrap();
    let en_full = crate::engine::output_guard::snapshot_sources();
    let r = plan_refresh(RefreshInput {
        old_hashes: &basis.source_hashes,
        old_pending: &LangMap::new(),
        en_full: &en_full,
        en_only: &en_only,
        zh_scan: &zh_scan,
        old_zh: &old_zh,
        removal: RemovalCheck { scan_clean: true, old_ns_jars: &basis.ns_jars, mods_now: &mods_now(&mc) },
    });
    assert!(has(&r.pending, "beta", "item.beta.one") && has(&r.pending, "beta", "item.beta.two"), "新模組要翻：{:?}", r.pending);
    assert!(has(&r.pending, "alpha", "item.alpha.pear"), "英文改過的句子要重翻");
    assert!(!has(&r.pending, "alpha", "item.alpha.apple"), "沒變的句子不再送");
    assert_eq!(r.zh["alpha"].get("item.alpha.apple").map(String::as_str), Some("蘋果"), "沒變的舊譯文照用");
    assert!(!has(&r.zh, "alpha", "item.alpha.pear"), "英文改過的舊譯文不沿用");
    assert!(has(&r.stale, "alpha", "item.alpha.pear"));
    assert!(!r.zh.contains_key("gone"), "拿掉的模組舊翻譯清掉");
    assert_eq!(r.removed_namespaces, vec!["gone".to_string()]);
    assert_eq!((r.new_sentences, r.changed_sentences), (2, 1));
    assert_eq!(r.hashes["alpha"]["item.alpha.pear"], text_hash("Pear (ripe)"), "新的雜湊要存回工作階段");
    let summary = UpdateSummary::from_refresh(RefreshReason::ModsChanged, &r);
    let json = serde_json::to_value(&summary).unwrap();
    assert_eq!(json["removedMods"], 1);
    assert_eq!(json["reason"], "modsChanged");
    assert!(summary.log_line().contains("拿掉的 1 個模組，舊翻譯已清掉"));
}

#[test]
fn b6a1_refresh_keeps_old_gaps_and_does_not_resend_deliberately_skipped_sentences() {
    let hashes = hashes_of(&lm(&[("a", "k1", "One"), ("a", "k2", "Two"), ("a", "k3", "Three")]));
    let en_only = lm(&[("a", "k1", "One"), ("a", "k2", "Two"), ("a", "k3", "Three")]);
    let old_pending = lm(&[("a", "k2", "Two")]);
    let r = plan_refresh(RefreshInput {
        old_hashes: &hashes,
        old_pending: &old_pending,
        en_full: &en_only,
        en_only: &en_only,
        zh_scan: &LangMap::new(),
        old_zh: &lm(&[("a", "k1", "一")]),
        removal: RemovalCheck { scan_clean: true, old_ns_jars: &Default::default(), mods_now: &Default::default() },
    });
    assert!(has(&r.pending, "a", "k2"), "上次的缺口順便補完（S15 與 S14 同時成立）");
    assert!(!has(&r.pending, "a", "k3"), "上次刻意沒列入待補、英文也沒變的不送（例：略過已完成命名空間）");
    assert!(!has(&r.pending, "a", "k1"));
}

#[test]
fn b6a1_old_session_without_hashes_does_not_drop_anything() {
    let en = lm(&[("a", "k1", "One"), ("a", "k2", "Two")]);
    let old_zh = lm(&[("a", "k1", "一"), ("other", "x", "別的")]);
    let r = plan_refresh(RefreshInput {
        old_hashes: &SourceHashes::new(),
        old_pending: &LangMap::new(),
        en_full: &en,
        en_only: &en,
        zh_scan: &LangMap::new(),
        old_zh: &old_zh,
        removal: RemovalCheck { scan_clean: true, old_ns_jars: &Default::default(), mods_now: &Default::default() },
    });
    assert_eq!(r.zh, old_zh, "無法確認哪些改了：舊譯文一條都不丟");
    assert!(r.removed_namespaces.is_empty());
    assert!(has(&r.pending, "a", "k2") && !has(&r.pending, "a", "k1"));
    assert_eq!((r.new_sentences, r.changed_sentences), (0, 0));
    assert!(!r.hashes.is_empty(), "重掃後補上雜湊");
}

#[test]
fn b6a1_game_chinese_fills_what_the_old_translation_lacks() {
    let en = lm(&[("a", "k1", "One"), ("a", "k2", "Two")]);
    let r = plan_refresh(RefreshInput {
        old_hashes: &hashes_of(&en),
        old_pending: &LangMap::new(),
        en_full: &en,
        en_only: &lm(&[("a", "k1", "One")]),
        zh_scan: &lm(&[("a", "k2", "模組自帶")]),
        old_zh: &lm(&[("a", "k1", "一")]),
        removal: RemovalCheck { scan_clean: true, old_ns_jars: &Default::default(), mods_now: &Default::default() },
    });
    assert!(has(&r.scan_fill, "a", "k2"));
    assert!(r.pending.is_empty());
}

#[test]
fn b6a1_full_retranslation_does_not_reuse_translations_whose_english_changed() {
    let old = hashes_of(&lm(&[("a", "k1", "One"), ("a", "k2", "Two"), ("gone", "x", "X")]));
    let new = hashes_of(&lm(&[("a", "k1", "One"), ("a", "k2", "Two!"), ("a", "k3", "Three")]));
    let mut prior = lm(&[("a", "k1", "一"), ("a", "k2", "二"), ("gone", "x", "叉"), ("unknown", "y", "未知")]);
    let jars: BTreeMap<String, Vec<String>> = [("gone".to_string(), vec!["gone.jar".to_string()])].into();
    let now: std::collections::BTreeSet<String> = ["other.jar".to_string()].into();
    let removal = RemovalCheck { scan_clean: true, old_ns_jars: &jars, mods_now: &now };
    let (stale, removed) = drop_stale(&mut prior, &old, &new, &LangMap::new(), &removal);
    assert_eq!((stale, removed), (1, 1));
    assert!(has(&prior, "a", "k1") && !has(&prior, "a", "k2") && !prior.contains_key("gone"));
    assert!(has(&prior, "unknown", "y"), "上次沒記錄的命名空間無法確認，不丟");
    // 舊工作階段沒雜湊：什麼都不丟
    let mut prior2 = lm(&[("a", "k2", "二")]);
    assert_eq!(drop_stale(&mut prior2, &SourceHashes::new(), &new, &LangMap::new(), &removal), (0, 0));
    assert!(has(&prior2, "a", "k2"));
}

#[test]
fn b6a1_forget_keys_gives_changed_sentences_another_chance() {
    let mut deferred = lm(&[("a", "k1", "One"), ("a", "k2", "Two")]);
    forget_keys(&mut deferred, &lm(&[("a", "k1", "One!")]));
    assert!(!has(&deferred, "a", "k1") && has(&deferred, "a", "k2"));
}

#[test]
fn b6a1_text_hash_is_stable() {
    assert_eq!(text_hash(""), "cbf29ce484222325");
    assert_eq!(text_hash("Apple"), text_hash("Apple"));
    assert_ne!(text_hash("Apple"), text_hash("Apple "));
}
