//! `kotoba voice` — manage TTS voice selection.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use snafu::{ResultExt, ensure};

use crate::{
    error::{self, Result},
    tts::{CosyvoiceBackend, TtsBackend},
};

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

/// Parameters for `voice clone` workflow.
#[derive(Debug)]
pub struct VoiceCloneRequest {
    pub source_url:         String,
    pub profile:            String,
    pub speaker:            String,
    pub start:              String,
    pub duration_sec:       u32,
    pub prompt_text:        String,
    pub verification_text:  String,
    pub skip_runtime_check: bool,
}

/// Result of `voice clone` workflow.
#[derive(Debug, Serialize)]
pub struct VoiceCloneResult {
    pub profile:             String,
    pub speaker:             String,
    pub prompt_wav:          String,
    pub source_audio:        String,
    pub voice_active:        String,
    pub cosyvoice_mode:      String,
    pub runtime_verified:    bool,
    pub verification_output: Option<String>,
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

/// Build a `CosyVoice` clone profile from a source URL, then switch backend to
/// `cosyvoice:<speaker>`.
pub async fn clone_from_url(request: VoiceCloneRequest) -> Result<VoiceCloneResult> {
    ensure!(
        !request.source_url.trim().is_empty(),
        error::CosyvoiceSnafu {
            message: "source URL is empty".to_string(),
        }
    );
    ensure!(
        request.duration_sec > 0,
        error::CosyvoiceSnafu {
            message: "duration_sec must be greater than 0".to_string(),
        }
    );

    let prompt_text = request.prompt_text.trim().to_string();
    ensure!(
        !prompt_text.is_empty(),
        error::CosyvoiceSnafu {
            message: "prompt_text is required for zero_shot cloning".to_string(),
        }
    );
    ensure_command_available("yt-dlp")?;
    ensure_command_available("ffmpeg")?;

    let profile = sanitize_profile_name(&request.profile);
    let profile_dir = crate::paths::cosyvoice_prompt_profile_dir(&profile);
    fs::create_dir_all(&profile_dir).context(error::IoSnafu)?;

    let source_audio = download_source_audio(&request.source_url, &profile_dir)?;
    let prompt_wav = profile_dir.join("prompt.wav");
    let start = request.start.trim();
    extract_prompt_clip(
        &source_audio,
        &prompt_wav,
        if start.is_empty() { "0" } else { start },
        request.duration_sec,
    )?;

    let prompt_wav_str = prompt_wav.to_string_lossy().to_string();
    let speaker = if request.speaker.trim().is_empty() {
        "clone".to_string()
    } else {
        request.speaker.trim().to_string()
    };
    let voice_active = format!("cosyvoice:{speaker}");

    let mut cfg = crate::app_config::load().clone();
    cfg.cosyvoice.mode = "zero_shot".to_string();
    cfg.cosyvoice.prompt_text = prompt_text;
    cfg.cosyvoice.prompt_wav = prompt_wav_str.clone();
    cfg.cosyvoice.instruct_text = String::new();
    cfg.cosyvoice.autostart = true;
    cfg.voice.active = voice_active.clone();
    crate::app_config::save(&cfg).context(error::IoSnafu)?;

    let mut runtime_verified = false;
    let mut verification_output = None;
    if !request.skip_runtime_check {
        let mut runtime_cfg = cfg.cosyvoice.clone();
        runtime_cfg.url = crate::cosyvoice_runtime::base_url(&runtime_cfg);
        crate::cosyvoice_runtime::ensure_running(&runtime_cfg).await?;
        runtime_verified = true;

        let verification_text = request.verification_text.trim();
        if !verification_text.is_empty() {
            let output = verification_output_path(&profile)?;
            let backend = CosyvoiceBackend::new(
                runtime_cfg.url.clone(),
                runtime_cfg.mode,
                speaker.clone(),
                runtime_cfg.prompt_text,
                runtime_cfg.prompt_wav,
                runtime_cfg.instruct_text,
            );
            backend.synthesize(verification_text, &output).await?;
            let metadata = fs::metadata(&output).context(error::IoSnafu)?;
            ensure!(
                metadata.len() > 44,
                error::CosyvoiceSnafu {
                    message: format!(
                        "verification synthesis produced an invalid wav file: {}",
                        output.display()
                    ),
                }
            );
            verification_output = Some(output.display().to_string());
        }
    }

    Ok(VoiceCloneResult {
        profile,
        speaker,
        prompt_wav: prompt_wav_str,
        source_audio: source_audio.display().to_string(),
        voice_active,
        cosyvoice_mode: "zero_shot".to_string(),
        runtime_verified,
        verification_output,
    })
}

fn ensure_command_available(command: &str) -> Result<()> {
    let exists = Command::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok();
    ensure!(
        exists,
        error::CosyvoiceSnafu {
            message: format!("required command not found in PATH: {command}"),
        }
    );
    Ok(())
}

fn verification_output_path(profile: &str) -> Result<PathBuf> {
    let cache_dir = crate::paths::audio_cache_dir();
    fs::create_dir_all(&cache_dir).context(error::IoSnafu)?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let safe_profile = sanitize_profile_name(profile);
    Ok(cache_dir.join(format!(
        "clone_verify_{safe_profile}_{}_{}.wav",
        std::process::id(),
        ts
    )))
}

fn sanitize_profile_name(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            output.push(ch.to_ascii_lowercase());
        } else {
            output.push('_');
        }
    }
    let collapsed = output
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if collapsed.is_empty() {
        "clone_profile".to_string()
    } else {
        collapsed
    }
}

fn download_source_audio(source_url: &str, profile_dir: &Path) -> Result<PathBuf> {
    let output_template = profile_dir.join("source.%(ext)s");
    let output = Command::new("yt-dlp")
        .arg("--no-playlist")
        .arg("-f")
        .arg("bestaudio/best")
        .arg("--print")
        .arg("after_move:filepath")
        .arg("-o")
        .arg(&output_template)
        .arg(source_url)
        .output()
        .context(error::IoSnafu)?;

    if !output.status.success() {
        let detail = command_failure_detail(&output.stdout, &output.stderr);
        return error::CosyvoiceSnafu {
            message: format!(
                "yt-dlp download failed (status {}): {detail}",
                output.status
            ),
        }
        .fail();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let source_path = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .map_or_else(|| infer_single_source_file(profile_dir), PathBuf::from);
    ensure!(
        source_path.is_file(),
        error::CosyvoiceSnafu {
            message: format!(
                "yt-dlp did not produce a source audio file at {}",
                source_path.display()
            ),
        }
    );
    Ok(source_path)
}

fn infer_single_source_file(profile_dir: &Path) -> PathBuf {
    let Ok(entries) = std::fs::read_dir(profile_dir) else {
        return profile_dir.join("source.unknown");
    };
    let mut candidates: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("source."))
        })
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .next_back()
        .unwrap_or_else(|| profile_dir.join("source.unknown"))
}

fn extract_prompt_clip(
    source_audio: &Path,
    prompt_wav: &Path,
    start: &str,
    duration_sec: u32,
) -> Result<()> {
    let output = Command::new("ffmpeg")
        .arg("-y")
        .arg("-ss")
        .arg(start)
        .arg("-t")
        .arg(duration_sec.to_string())
        .arg("-i")
        .arg(source_audio)
        .arg("-vn")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("22050")
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg(prompt_wav)
        .output()
        .context(error::IoSnafu)?;

    if !output.status.success() {
        let detail = command_failure_detail(&output.stdout, &output.stderr);
        return error::CosyvoiceSnafu {
            message: format!(
                "ffmpeg clip extraction failed (status {}): {detail}",
                output.status
            ),
        }
        .fail();
    }

    let metadata = std::fs::metadata(prompt_wav).context(error::IoSnafu)?;
    ensure!(
        metadata.len() > 44,
        error::CosyvoiceSnafu {
            message: format!(
                "prompt wav was generated but too small ({} bytes): {}",
                metadata.len(),
                prompt_wav.display()
            ),
        }
    );
    Ok(())
}

fn command_failure_detail(stdout: &[u8], stderr: &[u8]) -> String {
    let stderr_text = String::from_utf8_lossy(stderr);
    if !stderr_text.trim().is_empty() {
        return truncate_message(stderr_text.trim(), 500);
    }
    let stdout_text = String::from_utf8_lossy(stdout);
    if !stdout_text.trim().is_empty() {
        return truncate_message(stdout_text.trim(), 500);
    }
    "(no output)".to_string()
}

fn truncate_message(message: &str, max_chars: usize) -> String {
    if message.chars().count() <= max_chars {
        return message.to_string();
    }
    let head: String = message.chars().take(max_chars).collect();
    format!("{head}...")
}

#[cfg(test)]
mod tests {
    use std::{env, fs, path::Path};

    use super::*;

    #[test]
    fn sanitize_profile_name_filters_and_normalizes() {
        assert_eq!(
            sanitize_profile_name("To Love-Ru / Rara"),
            "to_love-ru_rara"
        );
        assert_eq!(sanitize_profile_name("___"), "clone_profile");
    }

    #[test]
    fn truncate_message_keeps_short_text() {
        assert_eq!(truncate_message("abc", 10), "abc");
    }

    #[test]
    fn truncate_message_trims_long_text() {
        assert_eq!(truncate_message("abcdef", 3), "abc...");
    }

    #[tokio::test]
    #[ignore = "requires network + yt-dlp + ffmpeg + running CosyVoice runtime"]
    async fn clone_to_love_ru_rara_from_yt_source() {
        let backup = ConfigFileBackup::new(crate::paths::config_file());
        let source_url = env::var("KOTOBA_TEST_RARA_SOURCE_URL")
            .expect("set KOTOBA_TEST_RARA_SOURCE_URL to a Rara source URL");

        let request = VoiceCloneRequest {
            source_url,
            profile: "to_love_ru_rara".to_string(),
            speaker: "clone".to_string(),
            start: env::var("KOTOBA_TEST_RARA_START").unwrap_or_else(|_| "0".to_string()),
            duration_sec: env::var("KOTOBA_TEST_RARA_DURATION_SEC")
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(12),
            prompt_text: env::var("KOTOBA_TEST_RARA_PROMPT_TEXT")
                .unwrap_or_else(|_| "ごめんね、ちょっとびっくりしちゃった。".to_string()),
            verification_text: env::var("KOTOBA_TEST_RARA_SYNTH_TEXT")
                .unwrap_or_else(|_| "よろしくね、わたしはララだよ。".to_string()),
            skip_runtime_check: false,
        };

        let result = clone_from_url(request)
            .await
            .expect("voice clone flow should complete");

        assert_eq!(result.profile, "to_love_ru_rara");
        assert_eq!(result.speaker, "clone");
        assert_eq!(result.voice_active, "cosyvoice:clone");
        assert_eq!(result.cosyvoice_mode, "zero_shot");
        assert!(result.runtime_verified);
        assert!(
            Path::new(&result.prompt_wav).is_file(),
            "prompt wav should exist: {}",
            result.prompt_wav
        );
        assert!(
            Path::new(&result.source_audio).is_file(),
            "source audio should exist: {}",
            result.source_audio
        );

        let verification = result
            .verification_output
            .expect("verification output should be produced");
        let verification_path = Path::new(&verification);
        assert!(verification_path.is_file());
        let bytes = fs::read(verification_path).expect("failed to read verification wav");
        assert!(
            bytes.starts_with(b"RIFF"),
            "verification output should be WAV"
        );
        assert!(bytes.len() > 44, "verification output should contain PCM");

        drop(backup);
    }

    struct ConfigFileBackup {
        path: PathBuf,
        data: Option<Vec<u8>>,
    }

    impl ConfigFileBackup {
        fn new(path: PathBuf) -> Self {
            let data = fs::read(&path).ok();
            Self { path, data }
        }
    }

    impl Drop for ConfigFileBackup {
        fn drop(&mut self) {
            if let Some(bytes) = &self.data {
                let _ = fs::write(&self.path, bytes);
            } else if self.path.exists() {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
}
