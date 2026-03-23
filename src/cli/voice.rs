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

/// A pre-defined tone/prosody preset.
#[derive(Debug, Clone, Copy)]
pub struct TonePreset {
    pub name:                &'static str,
    pub description:         &'static str,
    pub voice_speed:         f64,
    pub rvc_pitch:           i32,
    pub rvc_pitch_algo:      &'static str,
    pub rvc_index_influence: f64,
}

/// Serializable tone preset view for CLI output.
#[derive(Debug, Serialize)]
pub struct TonePresetInfo {
    pub name:                String,
    pub description:         String,
    pub voice_speed:         f64,
    pub rvc_pitch:           i32,
    pub rvc_pitch_algo:      String,
    pub rvc_index_influence: f64,
    pub active:              bool,
}

const TONE_PRESETS: [TonePreset; 4] = [
    TonePreset {
        name:                "balanced",
        description:         "Natural and stable default tone.",
        voice_speed:         1.00,
        rvc_pitch:           0,
        rvc_pitch_algo:      "rmvpe",
        rvc_index_influence: 0.66,
    },
    TonePreset {
        name:                "genki",
        description:         "Brighter and slightly faster energetic tone.",
        voice_speed:         1.16,
        rvc_pitch:           3,
        rvc_pitch_algo:      "rmvpe+",
        rvc_index_influence: 0.72,
    },
    TonePreset {
        name:                "kawaii",
        description:         "Higher pitch and playful cute tone.",
        voice_speed:         1.20,
        rvc_pitch:           4,
        rvc_pitch_algo:      "rmvpe+",
        rvc_index_influence: 0.74,
    },
    TonePreset {
        name:                "miku",
        description:         "Miku-focused tuning inspired by common RVC community presets.",
        voice_speed:         1.24,
        rvc_pitch:           5,
        rvc_pitch_algo:      "rmvpe+",
        rvc_index_influence: 0.78,
    },
];

fn find_preset(name: &str) -> Option<&'static TonePreset> {
    TONE_PRESETS
        .iter()
        .find(|preset| preset.name.eq_ignore_ascii_case(name))
}

fn preset_is_active(cfg: &crate::app_config::AppConfig, preset: &TonePreset) -> bool {
    const EPS: f64 = 1.0e-9;
    (cfg.voice.speed - preset.voice_speed).abs() < EPS
        && cfg.rvc.pitch == preset.rvc_pitch
        && cfg
            .rvc
            .pitch_algo
            .eq_ignore_ascii_case(preset.rvc_pitch_algo)
        && (cfg.rvc.index_influence - preset.rvc_index_influence).abs() < EPS
}

/// List available tone presets.
pub fn list_tones() -> Result<()> {
    let cfg = crate::app_config::load();
    let rows: Vec<TonePresetInfo> = TONE_PRESETS
        .iter()
        .map(|preset| TonePresetInfo {
            name:                preset.name.to_string(),
            description:         preset.description.to_string(),
            voice_speed:         preset.voice_speed,
            rvc_pitch:           preset.rvc_pitch,
            rvc_pitch_algo:      preset.rvc_pitch_algo.to_string(),
            rvc_index_influence: preset.rvc_index_influence,
            active:              preset_is_active(cfg, preset),
        })
        .collect();

    let output = serde_json::to_string_pretty(&rows).context(error::JsonSnafu)?;
    println!("{output}");
    Ok(())
}

/// Apply a tone preset to config.
pub fn set_tone(name: &str) -> Result<TonePresetInfo> {
    let preset = find_preset(name).ok_or_else(|| {
        error::VoicevoxSnafu {
            message: format!(
                "unknown tone preset: {name} (available: {})",
                TONE_PRESETS
                    .iter()
                    .map(|p| p.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
        .build()
    })?;

    let mut cfg = crate::app_config::load().clone();
    cfg.voice.speed = preset.voice_speed;
    cfg.rvc.pitch = preset.rvc_pitch;
    cfg.rvc.pitch_algo = preset.rvc_pitch_algo.to_string();
    cfg.rvc.index_influence = preset.rvc_index_influence;
    crate::app_config::save(&cfg).context(error::IoSnafu)?;

    eprintln!(
        "tone set to: {} (speed={}, rvc.pitch={}, rvc.pitch_algo={}, rvc.index_influence={})",
        preset.name,
        preset.voice_speed,
        preset.rvc_pitch,
        preset.rvc_pitch_algo,
        preset.rvc_index_influence
    );
    if !cfg.voice.active.starts_with("kokoro:") {
        eprintln!("note: current voice is not kokoro; tone speed may not affect all backends");
    } else if cfg.rvc.model.trim().is_empty() {
        eprintln!("note: no rvc.model configured; rvc parameters are saved for later");
    }

    Ok(TonePresetInfo {
        name:                preset.name.to_string(),
        description:         preset.description.to_string(),
        voice_speed:         preset.voice_speed,
        rvc_pitch:           preset.rvc_pitch,
        rvc_pitch_algo:      preset.rvc_pitch_algo.to_string(),
        rvc_index_influence: preset.rvc_index_influence,
        active:              true,
    })
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

    let cosyvoice_speaker = current
        .strip_prefix("cosyvoice:")
        .filter(|speaker| !speaker.trim().is_empty())
        .unwrap_or("default");
    let cosyvoice_key = format!("cosyvoice:{cosyvoice_speaker}");
    voices.push(VoiceInfo {
        name:    format!("CosyVoice {cosyvoice_speaker} [{cosyvoice_key}]"),
        backend: "cosyvoice".to_string(),
        active:  current == cosyvoice_key,
    });

    // Downloaded HuggingFace models
    let models_path = crate::paths::models_dir();
    std::fs::create_dir_all(&models_path).context(error::IoSnafu)?;
    let mut rvc_models: Vec<String> = Vec::new();
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
    let rvc_path = models_path.join("rvc");
    if let Ok(entries) = std::fs::read_dir(&rvc_path) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                rvc_models.push(entry.file_name().to_string_lossy().to_string());
            }
        }
    }
    rvc_models.sort();

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

        let active_rvc = &crate::app_config::load().rvc.model;
        for model_name in &rvc_models {
            let is_active_rvc = active_rvc == model_name;
            voices.push(VoiceInfo {
                name:    format!("RVC {model_name}"),
                backend: "rvc".to_string(),
                active:  is_active_rvc,
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
