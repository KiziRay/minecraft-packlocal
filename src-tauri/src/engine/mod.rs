mod apply_instance;
mod archive_overlay;
mod cancel;
mod convert;
mod coverage_tier;
mod deepseek;
mod diagnose;
mod discord_auth;
mod disk;
mod font_pack;
mod ftbquests;
mod glossary;
mod glossary_modpack;
mod hashutil;
mod instance_validate;
mod jar_scan;
mod jar_docs;
mod jar_display;
mod jar_patchouli;
mod jar_translate;
mod lenient_json;
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
mod session;
mod shared_tm;
mod shared_glossary;
mod text_overlay;
mod tm;
mod translation_scope;
mod translation_helper;
mod translation_mode;
mod turnstile;
mod updater;

pub use apply_instance::{
    apply_to_instance, delete_apply_backups_in, has_apply_backups_in, restore_last_apply_in, ApplyResult,
    DeleteBackupResult, RestoreResult,
};
pub use archive_overlay::translate_archive_overlays;
pub use cancel::{
    check as check_cancelled, request as request_cancel, reset as reset_cancel, CANCEL_MESSAGE,
};
pub use convert::{convert_langmap_s2tw, converter_name};
pub use coverage_tier::{map_stage_progress, CoverageSourceFlags, CoverageTier};
pub use deepseek::fill_missing_with_mode;
pub use deepseek::managed_ai_available;
pub use diagnose::{classify as classify_diagnosis, diagnose as diagnose_launch, LaunchDiagnosis};
pub use discord_auth::{
    cancel_discord_login, check_discord_auth_status, login_discord_blocking, logout_discord,
    DiscordAuthStatus, DISCORD_INVITE_URL,
};
pub use disk::{ensure_space, MIN_FREE_BYTES};
pub use font_pack::{
    apply_font_pack_to_instance, build_font_pack_str_with_options, FontPackApplyResult,
    FontPackOptions, FontPackResult,
};
pub use ftbquests::translate_ftbquests;
pub use glossary::{ensure_user_glossary_template, load_phrase_dict, user_glossary_path};
pub use instance_validate::{validate_instance_path, InstanceValidation};
pub use jar_scan::{resolve_minecraft_dir, scan_instance, LangMap, ScanReport};
pub use jar_docs::{extract_jar_documentation, JarDocumentationReport};
pub use jar_display::translate_jar_display_texts;
pub use jar_patchouli::translate_jar_patchouli;
pub use jar_translate::{rewrite_translated_jars, JarTranslationReport};
pub use merge_ref::{
    discover_default_reference, load_reference_zh_tw, merge_fill_missing, subtract_covered,
    try_download_cfpa_pack,
};
pub use minemenu::fix_minemenu_unicode_escapes;
pub use origins::translate_origins;
pub use out_layout::{
    cleanup_transient_work, ensure_result_layout, suggest_output_base, write_coverage_report,
    write_gap_summary_file,
    CoverageStats, RESULT_DIR_NAME,
};
pub use pack_version::{build_pack_name, PackVersionInfo};
pub use pack_out::{
    build_resource_pack, detect_minecraft_version, detect_pack_format, pack_format_for_version,
    BuildOptions,
};
pub use quests_books::translate_quests_books;
pub use secrets::{
    get_ai_mode, get_api_settings_public, get_minimize_on_close, save_api_settings,
    save_api_settings_with_provider, set_ai_mode, set_minimize_on_close, ApiSettingsPublic,
};
pub use script_literals::translate_kubejs_literals;
pub use security::{
    is_probably_network_path, normalize_user_path, validate_open_url,
};
pub use share_pack::package_translation;
pub use share_upload::{upload_share_package, ShareUploadResult};
pub use session::{
    count_map, find_pack_near, find_session_file, has_session_file, load_pack_zh, load_session,
    remaining_pending, save_session, TranslateSession, SESSION_FILE,
};
pub use text_overlay::translate_text_overlays;
pub use translation_mode::{
    mode_note, skip_complete_namespaces, TranslationMode, TranslationQuality,
};
pub use turnstile::{
    cancel_turnstile_verification, clear_turnstile_proof, managed_turnstile_required,
    turnstile_status,
    verify_turnstile_blocking,
};
pub use translation_scope::TranslationScope;
pub use translation_helper::{
    cleanup_translation_helper, inspect_translation_helper, prepare_translation_helper,
    TranslationHelperStatus,
};
pub use updater::{check_update as check_update_engine, download_and_launch, UpdateCheck};
