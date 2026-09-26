mod app_settings;
pub mod migrate;
mod apply_instance;
mod apply_guard;
pub mod apply_identity;
mod apply_knowledge;
mod mcpl_marker;
pub mod apply_notice;
mod apply_plan;
pub mod apply_record;
mod apply_restore;
mod apply_pending;
mod archive_overlay;
mod codex_auth;
mod codex_chat;
pub mod cancel;
mod cjk;
mod convert;
mod coverage_ledger;
mod consistency_report;
mod coverage_tier;
mod deepseek;
pub mod dev_mode;
pub mod dev_progress;
mod diagnose;
mod diagnose_report;
mod discord_auth;
mod disk;
mod elevate;
pub mod eligibility;
mod failed_items;
/// 回歸語料守衛，僅測試期編譯。
#[cfg(test)]
mod fixtures_guard;
mod font_pack;
pub mod font_restore;
mod native_lang;
mod options_txt;
mod ftbquests;
mod gap_model;
pub mod game_process;
mod glossary;
mod glossary_modpack;
mod hashutil;
mod instance_validate;
mod issue_report;
mod jar_scan;
mod lang_provenance;
mod jar_docs;
mod jar_display;
mod jar_origins;
mod jar_patchouli;
mod jar_translate;
mod jar_sources;
mod lenient_json;
pub mod local_llm;
mod local_quality;
mod mech_tokens;
mod merge_ref;
mod minemenu;
pub(crate) mod nanazip_ensure;
pub(crate) mod origins;
pub(crate) mod win_process;
pub mod outcome_ledger;
mod out_layout;
pub mod paths;
mod pack_version;
mod pack_out;
mod placeholder;
mod placeholder_fix;
mod output_guard;
mod output_guard_file;
mod output_guard_sources;
mod text_component;
pub mod source_catalog;
pub mod provenance;
mod quests_books;
pub mod release_manifest;
mod resource_pack_guard;
pub mod pack_repair;
mod run_journal;
pub mod run_plan;
mod safe_text;
mod secrets;
mod sentence_split;
mod script_literals;
mod share_pack;
mod share_upload;
mod security;
mod scan_cache;
mod search_system;
mod session;
mod shared_identity;
mod shared_tm;
mod shared_contribute_queue;
mod shared_glossary;
mod text_overlay;
mod tm;
mod translation_quality;
mod translation_scope;
mod translation_helper;
mod translation_mode;
pub mod trust_keys;
mod turnstile;
mod updater;
mod usage_feedback;

pub use apply_instance::{
    apply_to_instance, delete_apply_backups_in, has_apply_backups_in, restore_last_apply_in, ApplyResult,
    ApplyStatus, DeleteBackupResult, RestoreResult,
};
pub use archive_overlay::translate_archive_overlays;
pub use codex_auth::{
    cancel_gpt_login, gpt_auth_status, gpt_login_blocking, gpt_logout, GptAuthStatus,
};
pub use cancel::{
    check as check_cancelled, is_cancelled, request as request_cancel, reset as reset_cancel,
    CANCEL_MESSAGE,
};
pub use convert::{
    apply_phrase_dict, convert_langmap_s2tw_selective, convert_langmap_s2tw_with_progress,
    converter_name, strip_of_suffix_zhi,
};
pub use consistency_report::{
    consistency_suggestions_path, consistency_suggestions_status, merge_consistency_suggestions,
    write_consistency_hints,
};
pub use coverage_tier::{map_stage_progress, CoverageSourceFlags};
pub use coverage_ledger::{CoverageLedger, StageEntry};
pub use deepseek::{
    contribute_shared_glossary_from_langmaps, fill_missing_with_mode, seed_tm_from_langmaps,
    verify_ai_assistance, verify_custom_api, AiFillReport,
};
pub use diagnose::{
    classify as classify_diagnosis, classify_input as classify_diagnosis_input,
    diagnose as diagnose_launch, diagnose_pack_dir, LaunchDiagnosis,
};
pub use diagnose_report::{submit_diagnose_report, DiagnoseReportRequest, DiagnoseReportResult};
pub use discord_auth::{
    cancel_discord_login, check_discord_auth_status, login_discord_blocking, logout_discord,
    DiscordAuthStatus, DISCORD_INVITE_URL,
};
pub use disk::{ensure_ready_to_write, ensure_space, probe_apply_targets, MIN_FREE_BYTES};
pub use elevate::{check_write_access, relaunch_as_admin};
pub use font_pack::{
    apply_font_pack_to_instance, build_font_pack_str_with_options, read_font_preview_base64,
    FontPackApplyResult, FontPackOptions, FontPackResult,
};
pub use ftbquests::translate_ftbquests;
pub use source_catalog::save as source_catalog_save;
pub use output_guard::{begin_run as begin_guard_run, snapshot_sources, take_run_report_with_fonts as take_run_report, DisplaySafety};
pub use gap_model::{count_gaps, describe as describe_gaps};
pub use glossary::{ensure_user_glossary_template, load_phrase_dict, user_glossary_path};
pub use instance_validate::{validate_instance_path, InstanceValidation};
pub use issue_report::{submit_issue_report, SubmitIssueReportResult};
pub use jar_scan::{resolve_minecraft_dir, scan_instance, LangMap, ScanReport};
pub use lang_provenance::{get_source as get_lang_source, LangSource, ProvenanceMap};
pub use jar_docs::{extract_jar_documentation, JarDocumentationReport};
pub use jar_display::translate_jar_display_texts;
pub use jar_patchouli::translate_jar_patchouli;
pub use jar_origins::translate_jar_origins;
pub use jar_translate::{rewrite_translated_jars, JarTranslationReport};
pub use merge_ref::{
    discover_default_reference, load_reference_zh_tw, merge_fill_missing, subtract_covered,
    try_download_cfpa_pack,
};
pub use minemenu::translate_minemenu;
pub use origins::translate_origins;
pub use app_settings::{
    patch_settings, read_settings, read_settings_report, settings_path, SettingsPatchOp,
};
pub use dev_mode::{
    eligible as dev_mode_eligible, enabled as dev_mode_enabled,
    log_path as dev_mode_log_path, set_enabled as dev_mode_set_enabled,
};
pub use run_journal::{list_runs, write_run_log};
pub use failed_items::{build_failed_items_csv, merge_imported, parse_import_text, write_failed_items_csv, ImportReport};
pub use pack_repair::repair_pack_list;
pub use resource_pack_guard::{check_pack_health, PackHealthReport};
pub use out_layout::{
    cleanup_transient_work, ensure_result_layout, prune_empty_result_dirs, suggest_output_base,
    write_coverage_report, write_gap_summary_file,
    CoverageStats, RESULT_DIR_NAME,
};
pub use pack_version::{
    build_pack_name, detect_pack_version, resolve_output_pack_name, PackVersionInfo,
};
pub use pack_out::{
    build_resource_pack, build_resource_pack_skipping_bundled, detect_minecraft_version, detect_pack_format,
    ensure_minecraft_version_for_translate, pack_format_for_version, BuildOptions,
};
pub use native_lang::collect_mod_zh_tw;
pub use quests_books::translate_quests_books;
pub use secrets::{
    cloud_topup_choice, get_ai_mode, get_api_settings_public, get_gpt_model, get_minimize_on_close,
    save_api_settings, save_api_settings_with_provider, set_ai_mode, set_gpt_model,
    set_remember_api_key, remember_api_key, clear_api_key,
    set_minimize_on_close, ApiSettingsPublic, CloudTopUpChoice,
};
pub use script_literals::translate_kubejs_literals;
pub use search_system::{run_search_pipeline, write_search_artifacts};
pub use security::{
    is_probably_network_path, normalize_user_path, validate_open_url,
};
pub use share_pack::{has_shareable_content, package_translation};
pub use share_upload::{upload_share_package, ShareUploadResult};
pub use session::{
    count_map, discover_prior_zh_sources, filter_local_untranslatable, find_pack_near,
    find_session_file, find_sibling_instances_with_same_mods, has_session_file,
    is_tool_resource_pack, load_pack_zh, load_session,
    merge_pending, mods_fingerprint, resolve_canonical_tool_zip,
    filter_quality_deferred, prune_quality_deferred, remaining_pending, rework_unusable_zh,
    save_session, RunOutcome, RunPreferences, TranslateSession, SESSION_FILE,
};
pub use shared_contribute_queue::flush_pending as flush_shared_contribute_queue;
pub use shared_tm::{
    contribute_lang_maps, contribute_lang_maps_limited, ContributeLangMapsOpts,
    reset_contribute_tracker, SkipSharedLookupGuard,
};
pub use text_overlay::translate_text_overlays;
pub use translation_mode::{
    mode_note, skip_complete_namespaces_with_provenance, TranslationMode, TranslationQuality,
};
pub use turnstile::{
    cancel_turnstile_verification, clear_turnstile_proof, verify_turnstile_blocking,
};
pub use translation_scope::TranslationScope;
pub use translation_helper::{
    cleanup_translation_helper, inspect_translation_helper, prepare_translation_helper,
    TranslationHelperStatus,
};
pub use updater::{
    check_update as check_update_engine, cleanup_update_residuals, download_and_launch, UpdateCheck,
};
pub use usage_feedback::{submit_usage_feedback_cmd, SubmitUsageFeedbackCmdResult};
