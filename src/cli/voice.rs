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
                // Skip directories managed by other backends
                if dir_name == "kokoro" || dir_name == "rvc" {
                    continue;
                }
                let key = format!("vits:{dir_name}");
                voices.push(VoiceInfo {
                    name:    format!("{dir_name} [{key}]"),
                    backend: "vits".to_string(),
                    active:  current == key,
                });
            }
        }
    }

    // Kokoro ONNX voices (available only when the model is downloaded)
    // TODO: read available voices from voices-v1.0.bin metadata instead of
    // hardcoding
    let kokoro_model = models_path.join("kokoro").join("kokoro-v1.0.onnx");
    if kokoro_model.exists() {
        let kokoro_voices = [
            "af_heart",
            "af_bella",
            "af_nicole",
            "af_sarah",
            "af_sky",
            "am_adam",
            "am_michael",
            "jf_alpha",
            "jf_gongitsune",
            "jm_kumo",
        ];
        for voice_name in &kokoro_voices {
            let key = format!("kokoro:{voice_name}");
            voices.push(VoiceInfo {
                name:    format!("Kokoro {voice_name} [{key}]"),
                backend: "kokoro".to_string(),
                active:  current == key,
            });
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

/// An RVC model entry for display.
#[derive(Debug, Serialize)]
pub struct RvcModelInfo {
    /// Directory name of the model.
    pub name:      String,
    /// Whether this model is currently active.
    pub active:    bool,
    /// Whether model.pth exists in the directory.
    pub has_pth:   bool,
    /// Whether model.index exists in the directory.
    pub has_index: bool,
}

/// Scan the RVC models directory and return all valid models.
pub fn list_rvc_models() -> Vec<RvcModelInfo> {
    let rvc_dir = crate::paths::models_dir().join("rvc");
    let active_model = crate::app_config::load().rvc.model.clone();

    let Ok(entries) = std::fs::read_dir(&rvc_dir) else {
        return Vec::new();
    };

    let mut models: Vec<RvcModelInfo> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let dir = entry.path();
            RvcModelInfo {
                active: name == active_model,
                has_pth: dir.join("model.pth").exists(),
                has_index: dir.join("model.index").exists(),
                name,
            }
        })
        .collect();

    models.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    models
}

/// Find an RVC model by exact name or case-insensitive substring.
///
/// Returns `Ok(name)` if exactly one model matches, or an error describing
/// zero / ambiguous matches.
pub fn resolve_rvc_model(query: &str) -> Result<String> {
    let models = list_rvc_models();
    let valid: Vec<&RvcModelInfo> = models.iter().filter(|m| m.has_pth).collect();

    // Exact match first (case-insensitive)
    if let Some(exact) = valid.iter().find(|m| m.name.eq_ignore_ascii_case(query)) {
        return Ok(exact.name.clone());
    }

    // Substring match (case-insensitive)
    let query_lower = query.to_lowercase();
    let matches: Vec<&&RvcModelInfo> = valid
        .iter()
        .filter(|m| m.name.to_lowercase().contains(&query_lower))
        .collect();

    match matches.len() {
        0 => {
            let available = valid
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            error::RvcSnafu {
                message: format!("no RVC model matching '{query}' (available: {available})"),
            }
            .fail()
        }
        1 => Ok(matches[0].name.clone()),
        _ => {
            let ambiguous = matches
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            error::RvcSnafu {
                message: format!(
                    "'{query}' matches multiple RVC models: {ambiguous} — be more specific"
                ),
            }
            .fail()
        }
    }
}

/// List available RVC models as JSON.
pub fn list_rvc() -> Result<()> {
    let models = list_rvc_models();
    let output = serde_json::to_string_pretty(&models).context(error::JsonSnafu)?;
    println!("{output}");
    Ok(())
}

/// Set the active RVC model with fuzzy matching and validation.
pub fn set_rvc(query: &str) -> Result<String> {
    let resolved = resolve_rvc_model(query)?;
    let mut cfg = crate::app_config::load().clone();
    cfg.rvc.model.clone_from(&resolved);
    crate::app_config::save(&cfg).context(error::IoSnafu)?;
    eprintln!("rvc model set to: {resolved}");
    Ok(resolved)
}

/// Disable RVC voice conversion.
pub fn off_rvc() -> Result<()> {
    let mut cfg = crate::app_config::load().clone();
    cfg.rvc.model = String::new();
    crate::app_config::save(&cfg).context(error::IoSnafu)?;
    eprintln!("rvc disabled");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_rvc_models_does_not_panic() {
        // Should return a list (possibly empty) without panicking,
        // even when the rvc directory does not exist.
        let _models = list_rvc_models();
    }
}
