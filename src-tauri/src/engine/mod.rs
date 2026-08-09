mod apply_instance;
mod cancel;
mod convert;
mod deepseek;
mod disk;
mod font_pack;
mod ftbquests;
mod glossary;
mod jar_scan;
mod lenient_json;
mod merge_ref;
mod minemenu;
mod origins;
mod out_layout;
mod pack_out;
mod placeholder;
mod secrets;
mod security;
mod session;
mod text_overlay;
mod tm;
mod updater;

pub use apply_instance::{apply_to_instance, ApplyResult};
pub use cancel::{
    check as check_cancelled, request as request_cancel, reset as reset_cancel, CANCEL_MESSAGE,
};
pub use convert::{convert_langmap_s2tw, converter_name};
pub use disk::{ensure_space, MIN_FREE_BYTES};
pub use deepseek::fill_missing_with_ai;
pub use font_pack::{build_font_pack_str_with_options, FontPackOptions, FontPackResult};
pub use ftbquests::translate_ftbquests;
pub use glossary::{
    ensure_user_glossary_template, load_phrase_dict, user_glossary_path,
};
pub use jar_scan::{resolve_minecraft_dir, scan_instance, LangMap, ScanReport};
pub use merge_ref::{
    discover_default_reference, load_reference_zh_tw, merge_fill_missing, subtract_covered,
};
pub use minemenu::fix_minemenu_unicode_escapes;
pub use origins::translate_origins;
pub use out_layout::{
    ensure_result_layout, suggest_output_base, write_coverage_report, CoverageStats, RESULT_DIR_NAME,
};
pub use pack_out::{build_resource_pack, detect_pack_format, BuildOptions};
pub use secrets::{
    get_api_settings_public, get_minimize_on_close, load_deepseek_key, save_api_settings,
    set_minimize_on_close, ApiSettingsPublic,
};
pub use deepseek::managed_ai_available;
pub use security::{normalize_user_path, sanitize_folder_name, validate_open_url};
pub use session::{
    count_map, find_pack_near, find_session_file, has_session_file, load_pack_zh, load_session,
    remaining_pending, save_session, TranslateSession, SESSION_FILE,
};
pub use text_overlay::translate_text_overlays;
pub use updater::{check_update as check_update_engine, download_and_launch, UpdateCheck};
