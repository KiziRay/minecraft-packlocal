//! Public source tree: UI + security modules only.
//! The full translation engine is private; this crate does **not** build a playable product binary.

mod engine;

pub use engine::{check_update, download_and_launch, is_newer, UpdateCheck};

/// Placeholder entry used by `main.rs` in this public tree.
pub fn run() {
    eprintln!(
        "This public repository ships the UI and reviewable security modules only.\n\
         The scan / translate engine is private. Download the official MCPL exe from the Worker update channel."
    );
}
