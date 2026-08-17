//! Public review surface for safety-critical modules.
//! Scan / translate / one-click pipeline code is **not** published.

pub mod hashutil;
pub mod placeholder;
pub mod secrets;
pub mod security;
pub mod updater;

pub use updater::{check_update, download_and_launch, is_newer, UpdateCheck};
