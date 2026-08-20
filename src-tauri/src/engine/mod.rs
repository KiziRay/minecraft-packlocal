mod apply_instance;
mod archive_overlay;
pub mod cancel;
mod convert;
mod consistency_report;
mod coverage_tier;
mod deepseek;
pub mod dev_progress;
mod diagnose;
mod diagnose_report;
mod discord_auth;
mod disk;
mod font_pack;
mod ftbquests;
mod glossary;
mod glossary_modpack;
mod hashutil;
mod instance_validate;
mod jar_scan;
mod lang_provenance;
mod jar_docs;
mod jar_display;
mod jar_patchouli;
mod jar_translate;
mod lenient_json;
mod mech_tokens;
mod merge_ref;
mod minemenu;
mod origins;
mod out_layout;
mod pack_version;
mod pack_out;
mod placeholder;
mod quests_books;
mod secrets;
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
mod turnstile;
mod updater;
mod usage_feedback;

pub use apply_instance::{
    apply_to_instance, delete_apply_backups_in, has_apply_backups_in, restore_last_apply_in, ApplyResult,
    DeleteBackupResult, RestoreResult,
};
pub use archive_overlay::translate_archive_overlays;
pub use cancel::{
    check as check_cancelled, is_cancelled, request as request_cancel, reset as reset_cancel,
    CANCEL_MESSAGE,
};
pub use convert::{
    apply_phrase_dict, convert_langmap_s2tw_selective, convert_langmap_s2tw_with_progress,
    converter_name, strip_of_suffix_zhi,
};
pub use consistency_report::write_consistency_hints;
pub use coverage_tier::{map_stage_progress, CoverageSourceFlags, CoverageTier};
pub use deepseek::{
    fill_missing_with_mode, managed_ai_available, seed_tm_from_langmaps, verify_custom_api,
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
pub use font_pack::{
    apply_font_pack_to_instance, build_font_pack_str_with_options, read_font_preview_base64,
    FontPackApplyResult, FontPackOptions, FontPackResult,
};
pub use ftbquests::translate_ftbquests;
pub use glossary::{ensure_user_glossary_template, load_phrase_dict, user_glossary_path};
pub use instance_validate::{validate_instance_path, InstanceValidation};
pub use jar_scan::{resolve_minecraft_dir, scan_instance, LangMap, ScanReport};
pub use lang_provenance::{LangSource, ProvenanceMap};
pub use jar_docs::{extract_jar_documentation, JarDocumentationReport};
pub use jar_display::translate_jar_display_texts;
pub use jar_patchouli::translate_jar_patchouli;
pub use jar_translate::{rewrite_translated_jars, JarTranslationReport};
pub use merge_ref::{
    discover_default_reference, load_reference_zh_tw, merge_fill_missing, subtract_covered,
    try_download_cfpa_pack,
};
pub use minemenu::translate_minemenu;
pub use origins::translate_origins;
pub use out_layout::{
    cleanup_transient_work, ensure_result_layout, suggest_output_base, write_coverage_report,
    write_gap_summary_file,
    CoverageStats, RESULT_DIR_NAME,
};
pub use pack_version::{build_pack_name, resolve_output_pack_name, PackVersionInfo};
pub use pack_out::{
    build_resource_pack, detect_minecraft_version, detect_pack_format,
    ensure_minecraft_version_for_translate, pack_format_for_version, BuildOptions,
};
pub use quests_books::translate_quests_books;
pub use secrets::{
    get_ai_mode, get_api_settings_public, get_minimize_on_close, save_api_settings,
    save_api_settings_with_provider, set_ai_mode, set_minimize_on_close, ApiSettingsPublic,
};
pub use script_literals::translate_kubejs_literals;
pub use search_system::{run_search_pipeline, write_search_artifacts};
pub use security::{
    is_probably_network_path, normalize_user_path, validate_open_url,
};
pub use share_pack::{has_shareable_content, package_translation};
pub use share_upload::{upload_share_package, ShareUploadResult};
pub use session::{
    count_map, filter_local_untranslatable, find_pack_near, find_session_file, has_session_file,
    load_pack_zh, load_session, merge_pending, remaining_pending, rework_unusable_zh, save_session,
    TranslateSession, SESSION_FILE,
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
pub use updater::{check_update as check_update_engine, download_and_launch, UpdateCheck};
pub use usage_feedback::{
    managed_ai_gp_reward_cmd, managed_ai_usage_cmd, submit_usage_feedback_cmd, ManagedAiGpRewardCmdResult,
    ManagedAiUsageCmdResult, SubmitUsageFeedbackCmdResult,
};
