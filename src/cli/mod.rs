//! CLI command definitions and subcommand modules.

pub mod doctor;
pub mod export;
pub mod play;
pub mod setup;
pub mod voice;

use clap::{Parser, Subcommand};

/// Immersive Japanese language learning CLI.
#[derive(Parser)]
#[command(name = "kotoba")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Available subcommands.
#[derive(Subcommand)]
pub enum Command {
    /// Download VOICEVOX Engine, initialize DB, and configure environment
    Setup,
    /// Check all dependencies (DB, VOICEVOX, models, disk space)
    Doctor,
    /// Initialize the database (without full setup)
    Init,
    /// Show current learning status (level, vocab count, due reviews)
    Status,
    /// Add a new vocabulary word
    Add {
        /// The word (kanji or kana)
        word: String,
        /// Kana reading
        reading: String,
        /// Chinese meaning
        meaning: String,
        /// JLPT level
        #[arg(long, default_value = "N5")]
        level: String,
    },
    /// Record that a word was seen/reviewed
    Seen {
        /// The word
        word: String,
        /// Quality: 1 (forgot), 3 (recognized), 5 (instant recall)
        quality: u8,
    },
    /// Show vocabulary or grammar due for review
    Review {
        /// Review grammar instead of vocabulary
        #[arg(long)]
        grammar: bool,
    },
    /// Show learning progress statistics
    Progress {
        /// Show weekly stats only
        #[arg(long)]
        weekly: bool,
    },
    /// Play pronunciation via TTS
    Play {
        /// The word to pronounce
        word: String,
        /// TTS backend (voicevox or vits)
        #[arg(long, default_value = "voicevox")]
        backend: String,
    },
    /// Manage voice models
    Voice {
        #[command(subcommand)]
        action: VoiceAction,
    },
    /// Manage config values
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Export vocabulary data
    Export {
        /// Format: json, csv, anki
        format: String,
    },
}

/// Config management subcommands.
#[derive(Subcommand)]
pub enum ConfigAction {
    /// Set a config value
    Set {
        /// Config key (e.g. blending-intensity)
        key: String,
        /// Config value
        value: String,
    },
    /// Get a config value
    Get {
        /// Config key to look up
        key: String,
    },
    /// List all config values
    List,
}

/// Voice management subcommands.
#[derive(Subcommand)]
pub enum VoiceAction {
    /// List available voices (VOICEVOX built-in + downloaded HF models)
    List,
    /// Set the current voice
    Set {
        /// Voice name or ID
        name: String,
    },
    /// Download a voice model from `HuggingFace`
    Add {
        /// `HuggingFace` repo ID (e.g. username/model-name)
        repo_id: String,
    },
}
