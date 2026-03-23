//! Centralized path management for kotoba data directories.
//!
//! All paths derive from a single data root, resolved once via `OnceLock`.
//! Follows the rara-paths pattern for lazy, thread-safe path initialization.

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Root data directory: `~/.kotoba`
pub fn data_dir() -> &'static Path {
    DATA_DIR.get_or_init(|| {
        dirs::home_dir()
            .expect("home directory must be resolvable")
            .join(".kotoba")
    })
}

/// Database file: `<data>/kotoba.db`
pub fn db_path() -> PathBuf { data_dir().join("kotoba.db") }

/// VOICEVOX engine directory: `<data>/voicevox`
pub fn voicevox_dir() -> PathBuf { data_dir().join("voicevox") }

/// VOICEVOX engine executable: `<data>/voicevox/run`
pub fn voicevox_executable() -> PathBuf { voicevox_dir().join("run") }

/// Voice models directory: `<data>/models`
pub fn models_dir() -> PathBuf { data_dir().join("models") }

/// `CosyVoice` runtime directory: `<data>/cosyvoice`
pub fn cosyvoice_dir() -> PathBuf { data_dir().join("cosyvoice") }

/// `CosyVoice` runtime log file: `<data>/cosyvoice/runtime.log`
pub fn cosyvoice_log_file() -> PathBuf { cosyvoice_dir().join("runtime.log") }

/// Audio cache directory: `<data>/audio`
pub fn audio_cache_dir() -> PathBuf { data_dir().join("audio") }

/// Config file path: `<data>/config.toml`
pub fn config_file() -> PathBuf { data_dir().join("config.toml") }

/// RVC Python venv directory: `<data>/venvs/rvc`
pub fn rvc_venv_dir() -> PathBuf { data_dir().join("venvs").join("rvc") }
