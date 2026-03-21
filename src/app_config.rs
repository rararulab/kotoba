//! Application configuration backed by TOML file.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

static APP_CONFIG: OnceLock<AppConfig> = OnceLock::new();

/// Application configuration.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Voice backend configuration.
    pub voice:    VoiceConfig,
    /// VOICEVOX-specific configuration.
    pub voicevox: VoicevoxConfig,
}

/// Voice backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    /// Active voice identifier (e.g. "voicevox:1", "vits:model-name")
    pub active: String,
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

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            active: "voicevox:1".to_string(),
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
