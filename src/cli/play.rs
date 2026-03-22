//! TTS audio generation and caching with configurable voice backend.

use std::path::PathBuf;

use snafu::ResultExt;

use crate::{
    error::{self, Result},
    tts::{KokoroBackend, TtsBackend, VitsBackend, VoicevoxBackend},
};

/// Parsed voice configuration specifying backend and speaker/model identifier.
struct VoiceConfig {
    backend:    String,
    speaker_id: String,
}

/// Parse a `backend:id` voice config string (e.g. `voicevox:3`,
/// `vits:model-name`).
fn parse_voice_config(raw: &str) -> VoiceConfig {
    match raw.split_once(':') {
        Some((backend, id)) => VoiceConfig {
            backend:    backend.to_string(),
            speaker_id: id.to_string(),
        },
        None => VoiceConfig {
            backend:    raw.to_string(),
            speaker_id: "1".to_string(),
        },
    }
}

/// Generate or return a cached WAV file for a word using the configured voice.
///
/// Reads the `voice.active` key from `config.toml` to determine which TTS
/// backend and speaker to use. Falls back to `voicevox:1` when no config is
/// set.
#[tracing::instrument]
pub async fn play_word(word: &str) -> Result<PathBuf> {
    let raw_config = crate::app_config::load().voice.active.clone();
    let config = parse_voice_config(&raw_config);

    let cache = crate::paths::audio_cache_dir();
    std::fs::create_dir_all(&cache).context(error::IoSnafu)?;

    let file = cache.join(format!(
        "{word}_{backend}_{speaker}.wav",
        backend = config.backend,
        speaker = config.speaker_id
    ));

    if file.exists() {
        eprintln!("using cached: {}", file.display());
        return Ok(file);
    }

    eprintln!("synthesizing: {word}...");

    // RVC model is only applicable to the kokoro backend
    let mut rvc_model: Option<String> = None;

    let backend: Box<dyn TtsBackend> = match config.backend.as_str() {
        "voicevox" => {
            let base_url = voicevox_base_url();
            check_voicevox_reachable(&base_url).await?;
            Box::new(VoicevoxBackend::new(base_url, config.speaker_id.clone()))
        }
        "vits" => Box::new(VitsBackend::new(config.speaker_id.clone())),
        "kokoro" => {
            // Split once to extract both the base voice and optional RVC model
            let (base_voice, rvc) = match config.speaker_id.split_once("+rvc:") {
                Some((base, model)) => (base.to_string(), Some(model.to_string())),
                None => (config.speaker_id.clone(), None),
            };
            rvc_model = rvc;
            Box::new(KokoroBackend::new(base_voice))
        }
        other => {
            return Err(error::VoicevoxSnafu {
                message: format!("unknown voice backend: {other}"),
            }
            .build());
        }
    };

    backend.synthesize(word, &file).await?;

    // Apply RVC voice conversion if requested (kokoro backend only)
    if let Some(ref model) = rvc_model {
        eprintln!("converting with RVC model: {model}...");
        crate::rvc::check_installed().await?;
        let tmp_file = file.with_extension("pre_rvc.wav");
        std::fs::rename(&file, &tmp_file).context(error::IoSnafu)?;
        crate::rvc::convert(&tmp_file, model, &file).await?;
        let _ = std::fs::remove_file(&tmp_file);
    }

    eprintln!("cached ({}): {}", backend.name(), file.display());

    Ok(file)
}

/// Resolve the VOICEVOX base URL: env var overrides config.
fn voicevox_base_url() -> String {
    std::env::var("VOICEVOX_URL").unwrap_or_else(|_| crate::app_config::load().voicevox.url.clone())
}

/// Check that the VOICEVOX engine is reachable at the given URL.
async fn check_voicevox_reachable(base_url: &str) -> Result<()> {
    crate::http::client()
        .get(format!("{base_url}/version"))
        .send()
        .await
        .map_err(|_| {
            error::VoicevoxNotRunningSnafu {
                url: base_url.to_string(),
            }
            .build()
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_voice_config_with_colon() {
        let config = parse_voice_config("voicevox:3");
        assert_eq!(config.backend, "voicevox");
        assert_eq!(config.speaker_id, "3");
    }

    #[test]
    fn parse_voice_config_without_colon() {
        let config = parse_voice_config("voicevox");
        assert_eq!(config.backend, "voicevox");
        assert_eq!(config.speaker_id, "1");
    }

    #[test]
    fn parse_voice_config_vits() {
        let config = parse_voice_config("vits:my-model");
        assert_eq!(config.backend, "vits");
        assert_eq!(config.speaker_id, "my-model");
    }

    #[test]
    fn parse_voice_config_kokoro() {
        let config = parse_voice_config("kokoro:af_heart");
        assert_eq!(config.backend, "kokoro");
        assert_eq!(config.speaker_id, "af_heart");
    }

    #[test]
    fn parse_voice_config_kokoro_with_rvc() {
        let config = parse_voice_config("kokoro:af_heart+rvc:naruto");
        assert_eq!(config.backend, "kokoro");
        // speaker_id should contain the full suffix for downstream parsing
        assert_eq!(config.speaker_id, "af_heart+rvc:naruto");
    }

    #[test]
    fn parse_voice_config_empty_string() {
        let config = parse_voice_config("");
        assert_eq!(config.backend, "");
        assert_eq!(config.speaker_id, "1");
    }

    #[test]
    fn parse_voice_config_multiple_colons() {
        let config = parse_voice_config("vits:model:extra");
        assert_eq!(config.backend, "vits");
        assert_eq!(config.speaker_id, "model:extra");
    }
}
