mod apply_instance;
mod cancel;
mod convert;
mod deepseek;
mod diagnose;
mod discord_auth;
mod disk;
mod font_pack;
mod ftbquests;
mod glossary;
mod glossary_modpack;
mod hashutil;
mod jar_scan;
mod jar_docs;
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
mod share_pack;
mod share_upload;
mod security;
mod session;
mod shared_tm;
mod text_overlay;
mod tm;
mod turnstile;
mod updater;

pub use apply_instance::{
    apply_to_instance, delete_apply_backups_in, restore_last_apply_in, ApplyResult,
    DeleteBackupResult, RestoreResult,
};
pub use cancel::{
    check as check_cancelled, request as request_cancel, reset as reset_cancel, CANCEL_MESSAGE,
};
pub use convert::{convert_langmap_s2tw, converter_name};
pub use deepseek::fill_missing_with_ai;
pub use deepseek::managed_ai_available;
pub use diagnose::{classify as classify_diagnosis, diagnose as diagnose_launch, LaunchDiagnosis};
pub use discord_auth::{
    cancel_discord_login, check_discord_auth_status, login_discord_blocking, logout_discord,
    DiscordAuthStatus, DISCORD_INVITE_URL,
};
pub use disk::{ensure_space, MIN_FREE_BYTES};
pub use font_pack::{build_font_pack_str_with_options, FontPackOptions, FontPackResult};
pub use ftbquests::translate_ftbquests;
pub use glossary::{ensure_user_glossary_template, load_phrase_dict, user_glossary_path};
pub use jar_scan::{resolve_minecraft_dir, scan_instance, LangMap, ScanReport};
pub use jar_docs::{extract_jar_documentation, JarDocumentationReport};
pub use jar_translate::{rewrite_translated_jars, JarTranslationReport};
pub use merge_ref::{
    discover_default_reference, load_reference_zh_tw, merge_fill_missing, subtract_covered,
};
pub use minemenu::fix_minemenu_unicode_escapes;
pub use origins::translate_origins;
pub use out_layout::{
    ensure_result_layout, suggest_output_base, write_coverage_report, CoverageStats,
    RESULT_DIR_NAME,
};
pub use pack_version::{build_pack_name, PackVersionInfo};
pub use pack_out::{
    build_resource_pack, detect_minecraft_version, detect_pack_format, pack_format_for_version,
    BuildOptions,
};
pub use quests_books::translate_quests_books;
pub use secrets::{
    get_ai_mode, get_api_settings_public, get_minimize_on_close, save_api_settings, set_ai_mode,
    set_minimize_on_close, ApiSettingsPublic,
};
pub use security::{normalize_user_path, sanitize_folder_name, validate_open_url};
pub use share_pack::package_translation;
pub use share_upload::{upload_share_package, ShareUploadResult};
pub use session::{
    count_map, find_pack_near, find_session_file, has_session_file, load_pack_zh, load_session,
    remaining_pending, save_session, TranslateSession, SESSION_FILE,
};
pub use text_overlay::translate_text_overlays;
pub use turnstile::{
    cancel_turnstile_verification, clear_turnstile_proof, turnstile_status,
    verify_turnstile_blocking,
};
pub use updater::{check_update as check_update_engine, download_and_launch, UpdateCheck};
