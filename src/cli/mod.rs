//! CLI command definitions and subcommand modules.

pub mod dictionary;
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

/// Performance style for TTS delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum PlayStyle {
    /// Minimal expression changes.
    Neutral,
    /// Character-like expressive conversational style.
    Character,
    /// More dramatic performance.
    Dramatic,
    /// Softer and gentler delivery.
    Soft,
    /// Brighter and faster delivery.
    Energetic,
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
        /// Kana reading (optional; auto-filled when both reading and meaning
        /// are omitted)
        reading: Option<String>,
        /// Meaning (optional; auto-filled when both reading and meaning are
        /// omitted)
        meaning: Option<String>,
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
        word:   String,
        /// If set, immediately play the generated audio via local output device
        #[arg(long)]
        enable: bool,
        /// TTS performance style
        #[arg(long, value_enum, default_value = "character")]
        style:  PlayStyle,
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
    /// Download VOICEVOX, initialize DB, and set default Kokoro+RVC voice
    Setup,
    /// Check all dependencies (DB, VOICEVOX, models, disk space)
    Doctor {
        /// Output as JSON for scripting
        #[arg(long)]
        json: bool,
    },
    /// Initialize the database (without full setup)
    Init,
    /// Start an OpenAI-compatible TTS API server
    Serve {
        /// Host address to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Port to listen on
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },
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
    /// Manage voice tone presets (prosody and RVC tuning)
    Tone {
        #[command(subcommand)]
        action: VoiceToneAction,
    },
    /// Manage RVC voice conversion models
    Rvc {
        #[command(subcommand)]
        action: VoiceRvcAction,
    },
}

/// RVC model management subcommands.
#[derive(Subcommand)]
pub enum VoiceRvcAction {
    /// List downloaded RVC models
    List,
    /// Set the active RVC model (supports fuzzy matching)
    Set {
        /// Model name or substring (e.g. "miku")
        name: String,
    },
    /// Disable RVC voice conversion
    Off,
}

/// Voice tone preset subcommands.
#[derive(Subcommand)]
pub enum VoiceToneAction {
    /// List available tone presets
    List,
    /// Apply a tone preset
    Set {
        /// Preset name (e.g. balanced, genki, kawaii, miku)
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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn add_command_accepts_word_only() {
        let cli = Cli::try_parse_from(["kotoba", "add", "成功"]).expect("parse should succeed");
        let Command::Add {
            reading, meaning, ..
        } = cli.command
        else {
            panic!("expected add command");
        };

        assert!(reading.is_none());
        assert!(meaning.is_none());
    }

    #[test]
    fn add_command_accepts_manual_fields() {
        let cli = Cli::try_parse_from(["kotoba", "add", "成功", "せいこう", "success"])
            .expect("parse should succeed");
        let Command::Add {
            reading, meaning, ..
        } = cli.command
        else {
            panic!("expected add command");
        };

        assert_eq!(reading.as_deref(), Some("せいこう"));
        assert_eq!(meaning.as_deref(), Some("success"));
    }

    #[test]
    fn play_command_enable_defaults_to_false() {
        let cli = Cli::try_parse_from(["kotoba", "play", "成功"]).expect("parse should succeed");
        let Command::Play {
            word,
            enable,
            style,
        } = cli.command
        else {
            panic!("expected play command");
        };

        assert_eq!(word, "成功");
        assert!(!enable);
        assert_eq!(style, PlayStyle::Character);
    }

    #[test]
    fn play_command_supports_enable_flag() {
        let cli = Cli::try_parse_from(["kotoba", "play", "成功", "--enable"])
            .expect("parse should succeed");
        let Command::Play {
            word,
            enable,
            style,
        } = cli.command
        else {
            panic!("expected play command");
        };

        assert_eq!(word, "成功");
        assert!(enable);
        assert_eq!(style, PlayStyle::Character);
    }

    #[test]
    fn play_command_style_defaults_to_character() {
        let cli = Cli::try_parse_from(["kotoba", "play", "成功"]).expect("parse should succeed");
        let Command::Play { style, .. } = cli.command else {
            panic!("expected play command");
        };
        assert_eq!(style, PlayStyle::Character);
    }

    #[test]
    fn play_command_accepts_dramatic_style() {
        let cli = Cli::try_parse_from(["kotoba", "play", "成功", "--style", "dramatic"])
            .expect("parse should succeed");
        let Command::Play { style, .. } = cli.command else {
            panic!("expected play command");
        };
        assert_eq!(style, PlayStyle::Dramatic);
    }

    #[test]
    fn voice_tone_set_command_parses() {
        let cli = Cli::try_parse_from(["kotoba", "voice", "tone", "set", "miku"])
            .expect("parse should succeed");

        let Command::Voice { action } = cli.command else {
            panic!("expected voice command");
        };
        let VoiceAction::Tone { action } = action else {
            panic!("expected voice tone command");
        };
        let VoiceToneAction::Set { name } = action else {
            panic!("expected voice tone set command");
        };

        assert_eq!(name, "miku");
    }

    #[test]
    fn voice_rvc_list_command_parses() {
        let cli =
            Cli::try_parse_from(["kotoba", "voice", "rvc", "list"]).expect("parse should succeed");
        let Command::Voice { action } = cli.command else {
            panic!("expected voice command");
        };
        let VoiceAction::Rvc { action } = action else {
            panic!("expected voice rvc command");
        };
        assert!(matches!(action, VoiceRvcAction::List));
    }

    #[test]
    fn voice_rvc_set_command_parses() {
        let cli = Cli::try_parse_from(["kotoba", "voice", "rvc", "set", "miku"])
            .expect("parse should succeed");
        let Command::Voice { action } = cli.command else {
            panic!("expected voice command");
        };
        let VoiceAction::Rvc { action } = action else {
            panic!("expected voice rvc command");
        };
        let VoiceRvcAction::Set { name } = action else {
            panic!("expected set action");
        };
        assert_eq!(name, "miku");
    }

    #[test]
    fn voice_rvc_off_command_parses() {
        let cli =
            Cli::try_parse_from(["kotoba", "voice", "rvc", "off"]).expect("parse should succeed");
        let Command::Voice { action } = cli.command else {
            panic!("expected voice command");
        };
        let VoiceAction::Rvc { action } = action else {
            panic!("expected voice rvc command");
        };
        assert!(matches!(action, VoiceRvcAction::Off));
    }
}
