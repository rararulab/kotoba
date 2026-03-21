//! `kotoba voice` — manage TTS voice selection.

use serde::Serialize;
use snafu::ResultExt;

use crate::error::{self, Result};

/// A voice entry for display.
#[derive(Debug, Serialize)]
pub struct VoiceInfo {
    /// Voice display name.
    pub name:    String,
    /// Backend identifier.
    pub backend: String,
    /// Whether this voice is currently active.
    pub active:  bool,
}

/// List available voices.
pub fn list() -> Result<()> {
    let current = crate::app_config::load().voice.active.clone();

    let mut voices: Vec<VoiceInfo> = Vec::new();

    // VOICEVOX built-in speakers (subset of most popular ones)
    let voicevox_speakers = [
        ("1", "四国めたん (normal)"),
        ("2", "四国めたん (あまあま)"),
        ("3", "ずんだもん (normal)"),
        ("4", "ずんだもん (あまあま)"),
        ("8", "春日部つむぎ"),
        ("10", "雨晴はう"),
        ("13", "青山龍星"),
        ("14", "冥鳴ひまり"),
        ("20", "もち子さん"),
        ("23", "WhiteCUL"),
    ];

    for (id, name) in &voicevox_speakers {
        let key = format!("voicevox:{id}");
        voices.push(VoiceInfo {
            name:    format!("{name} [{key}]"),
            backend: "voicevox".to_string(),
            active:  current == key,
        });
    }

    // Downloaded HuggingFace models
    let models_path = crate::paths::models_dir();
    std::fs::create_dir_all(&models_path).context(error::IoSnafu)?;
    if let Ok(entries) = std::fs::read_dir(&models_path) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let dir_name = entry.file_name().to_string_lossy().to_string();
                let key = format!("vits:{dir_name}");
                voices.push(VoiceInfo {
                    name:    format!("{dir_name} [{key}]"),
                    backend: "vits".to_string(),
                    active:  current == key,
                });
            }
        }
    }

    let output = serde_json::to_string_pretty(&voices).context(error::JsonSnafu)?;
    println!("{output}");
    Ok(())
}

/// Set the active voice via config file.
pub fn set(name: &str) -> Result<()> {
    let mut cfg = crate::app_config::load().clone();
    cfg.voice.active = name.to_string();
    crate::app_config::save(&cfg).context(error::IoSnafu)?;
    eprintln!("voice set to: {name}");
    Ok(())
}
