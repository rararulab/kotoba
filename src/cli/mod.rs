//! CLI command definitions and subcommand modules.

pub mod doctor;
pub mod export;
pub mod huggingface;
pub mod play;
pub mod setup;
pub mod voice;

use clap::{Parser, Subcommand, ValueEnum};

/// JLPT proficiency level.
#[derive(Clone, Debug, ValueEnum)]
pub enum JlptLevel {
    N5,
    N4,
    N3,
    N2,
    N1,
}

impl std::fmt::Display for JlptLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::N5 => write!(f, "N5"),
            Self::N4 => write!(f, "N4"),
            Self::N3 => write!(f, "N3"),
            Self::N2 => write!(f, "N2"),
            Self::N1 => write!(f, "N1"),
        }
    }
}

/// Export output format.
#[derive(Clone, Debug, ValueEnum)]
pub enum ExportFormat {
    Json,
    Csv,
    Anki,
}

/// SRS review quality rating.
#[derive(Clone, Debug, ValueEnum)]
pub enum ReviewQuality {
    /// User misuses word or asks meaning again
    Forgot = 1,
    /// User understands word in context
    Recognized = 3,
    /// Instant recall, uses word correctly unprompted
    Recalled = 5,
}

impl ReviewQuality {
    /// Convert to the u8 value used by the SRS algorithm.
    pub const fn as_u8(&self) -> u8 {
        match self {
            Self::Forgot => 1,
            Self::Recognized => 3,
            Self::Recalled => 5,
        }
    }
}

/// Immersive Japanese language learning CLI.
#[derive(Parser)]
#[command(name = "kotoba", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Available subcommands.
#[derive(Subcommand)]
pub enum Command {
    // ── Learning ─────────────────────────────────────────────
    /// Add a new vocabulary word
    Add {
        /// The word (kanji or kana)
        word:    String,
        /// Kana reading
        reading: String,
        /// Meaning
        meaning: String,
        /// JLPT level
        #[arg(long, short = 'l', default_value = "n5")]
        level:   JlptLevel,
    },
    /// Manage grammar patterns
    Grammar {
        #[command(subcommand)]
        action: GrammarAction,
    },
    /// Record that a word or grammar pattern was seen/reviewed
    Seen {
        /// The word
        word:    String,
        /// Quality: forgot, recognized, or recalled
        quality: ReviewQuality,
        /// Record review for a grammar pattern instead of vocabulary
        #[arg(long, short = 'g')]
        grammar: bool,
    },
    /// Show vocabulary or grammar due for review
    Review {
        /// Review grammar instead of vocabulary
        #[arg(long, short = 'g')]
        grammar: bool,
    },
    /// List vocabulary or grammar entries
    List {
        /// List grammar instead of vocabulary
        #[arg(long, short = 'g')]
        grammar: bool,
        /// Filter by JLPT level
        #[arg(long, short = 'l')]
        level:   Option<JlptLevel>,
    },

    // ── Progress ─────────────────────────────────────────────
    /// Show current learning status (level, vocab count, due reviews)
    Status,
    /// Show learning progress statistics
    Progress {
        /// Show weekly stats only
        #[arg(long, short = 'w')]
        weekly: bool,
    },
    /// Export vocabulary or grammar data
    Export {
        /// Output format
        format:  ExportFormat,
        /// Export grammar instead of vocabulary
        #[arg(long, short = 'g')]
        grammar: bool,
    },

    // ── Voice ────────────────────────────────────────────────
    /// Play pronunciation via TTS
    Play {
        /// The word to pronounce
        word: String,
    },
    /// Manage TTS voice selection
    Voice {
        #[command(subcommand)]
        action: VoiceAction,
    },
    /// Download and manage ONNX models from HuggingFace
    #[allow(clippy::doc_markdown)] // doc comment doubles as CLI help text
    Huggingface {
        #[command(subcommand)]
        action: HuggingFaceAction,
    },

    // ── System ───────────────────────────────────────────────
    /// Download VOICEVOX Engine, initialize DB, and configure environment
    Setup,
    /// Check all dependencies (DB, VOICEVOX, models, disk space)
    Doctor {
        /// Output as JSON for scripting
        #[arg(long)]
        json: bool,
    },
    /// Initialize the database (without full setup)
    Init,
    /// Manage config values
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

/// Grammar management subcommands.
#[derive(Subcommand)]
pub enum GrammarAction {
    /// Add a new grammar pattern
    Add {
        /// Grammar pattern (e.g. "〜ている")
        pattern: String,
        /// Meaning description
        meaning: String,
        /// JLPT level
        #[arg(long, short = 'l', default_value = "n5")]
        level:   JlptLevel,
        /// Example sentence
        #[arg(long)]
        example: Option<String>,
    },
    /// List all grammar patterns
    List {
        /// Filter by JLPT level
        #[arg(long, short = 'l')]
        level: Option<JlptLevel>,
    },
}

/// Config management subcommands.
#[derive(Subcommand)]
pub enum ConfigAction {
    /// Set a config value
    Set {
        /// Config key (e.g. blending-intensity)
        key:   String,
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
}

/// HuggingFace model management subcommands.
#[allow(clippy::doc_markdown)] // doc comments double as CLI help text
#[derive(Subcommand)]
pub enum HuggingFaceAction {
    /// Download an ONNX model from HuggingFace
    Add {
        /// HuggingFace repo ID (e.g. username/model-name)
        repo_id: String,
    },
    /// List downloaded HuggingFace models
    List,
}
