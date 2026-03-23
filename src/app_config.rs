//! Application configuration backed by TOML file.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

static APP_CONFIG: OnceLock<AppConfig> = OnceLock::new();

/// Application configuration.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Voice backend configuration.
    pub voice:     VoiceConfig,
    /// VOICEVOX-specific configuration.
    pub voicevox:  VoicevoxConfig,
    /// CosyVoice-specific configuration.
    pub cosyvoice: CosyvoiceConfig,
    /// RVC voice conversion configuration.
    pub rvc:       RvcConfig,
}

/// Voice backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    /// Active voice identifier (e.g. "voicevox:1", "vits:model-name")
    pub active: String,
    /// Default speech speed multiplier for TTS backends.
    pub speed:  f64,
}

/// VOICEVOX-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoicevoxConfig {
    /// Engine version to download.
    pub version: String,
    /// API base URL.
    pub url:     String,
    /// Default speaker ID.
    pub speaker: String,
}

/// CosyVoice-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CosyvoiceConfig {
    /// API base URL.
    pub url:           String,
    /// Inference mode (`sft`, `zero_shot`, `cross_lingual`, `instruct`).
    pub mode:          String,
    /// Prompt text for `zero_shot` mode.
    pub prompt_text:   String,
    /// Prompt wav path for `zero_shot` / `cross_lingual` mode.
    pub prompt_wav:    String,
    /// Instruction text for instruct mode.
    pub instruct_text: String,
}

/// RVC voice conversion configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RvcConfig {
    /// RVC model name (directory under `~/.kotoba/models/rvc/`).
    /// When non-empty, RVC voice conversion is applied after TTS synthesis.
    pub model:           String,
    /// Path to Python executable with RVC dependencies installed.
    pub python:          String,
    /// Pitch shift in semitones for RVC conversion.
    pub pitch:           i32,
    /// Pitch extraction algorithm used by RVC (e.g. rmvpe, rmvpe+, pm).
    pub pitch_algo:      String,
    /// Influence of the index file on timbre (0.0 to 1.0).
    pub index_influence: f64,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            active: "voicevox:1".to_string(),
            speed:  1.0,
        }
    }
}

impl Default for VoicevoxConfig {
    fn default() -> Self {
        Self {
            version: "0.22.2".to_string(),
            url:     "http://localhost:50021".to_string(),
            speaker: "1".to_string(),
        }
    }
}

impl Default for CosyvoiceConfig {
    fn default() -> Self {
        Self {
            url:           "http://127.0.0.1:50000".to_string(),
            mode:          "sft".to_string(),
            prompt_text:   String::new(),
            prompt_wav:    String::new(),
            instruct_text: String::new(),
        }
    }
}

impl Default for RvcConfig {
    fn default() -> Self {
        Self {
            model:           String::new(),
            python:          String::new(),
            pitch:           0,
            pitch_algo:      "rmvpe".to_string(),
            index_influence: 0.66,
        }
    }
}

/// Load config from TOML file, falling back to defaults.
pub fn load() -> &'static AppConfig {
    APP_CONFIG.get_or_init(|| {
        let path = crate::paths::config_file();
        if path.exists() {
            let settings = config::Config::builder()
                .add_source(config::File::from(path.as_ref()))
                .build()
                .unwrap_or_default();
            settings.try_deserialize().unwrap_or_default()
        } else {
            AppConfig::default()
        }
    })
}

/// Save config to TOML file.
pub fn save(cfg: &AppConfig) -> std::io::Result<()> {
    let path = crate::paths::config_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(cfg).expect("config serialization should not fail");
    std::fs::write(path, content)
}
