//! CLI command definitions and subcommand modules.

pub mod export;
pub mod play;

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
    /// Initialize the database
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
    /// Play pronunciation via VOICEVOX TTS
    Play {
        /// The word to pronounce
        word: String,
    },
    /// Set a config value
    Config {
        /// Config key (e.g. blending-intensity)
        key: String,
        /// Config value
        value: String,
    },
    /// Export vocabulary data
    Export {
        /// Format: json, csv, anki
        format: String,
    },
}
