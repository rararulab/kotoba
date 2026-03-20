//! TTS audio generation and caching with configurable voice backend.

use std::path::PathBuf;

use snafu::ResultExt;

use crate::{
    db::Database,
    error::{self, Result},
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

fn cache_dir() -> Result<PathBuf> {
    let dir = dirs::home_dir()
        .ok_or_else(|| error::HomeNotFoundSnafu.build())?
        .join(".kotoba")
        .join("audio");
    std::fs::create_dir_all(&dir).context(error::IoSnafu)?;
    Ok(dir)
}

/// Generate or return a cached WAV file for a word using the voice configured
/// in the database.
///
/// Reads the `voice` key from `user_profile` to determine which TTS backend and
/// speaker to use. Falls back to `voicevox:1` when no config is set.
#[tracing::instrument(skip(db))]
pub async fn play_word(db: &Database, word: &str) -> Result<PathBuf> {
    let raw_config = db
        .get_config("voice")
        .await?
        .unwrap_or_else(|| "voicevox:1".to_string());
    let config = parse_voice_config(&raw_config);

    let cache = cache_dir()?;
    let file = cache.join(format!(
        "{word}_{backend}_{speaker}.wav",
        backend = config.backend,
        speaker = config.speaker_id
    ));

    if file.exists() {
        return Ok(file);
    }

    match config.backend.as_str() {
        "voicevox" => synthesize_voicevox(word, &config.speaker_id, &file).await?,
        "vits" => synthesize_vits(&config.speaker_id, word, &file).await?,
        other => {
            return Err(error::VoicevoxSnafu {
                message: format!("unknown voice backend: {other}"),
            }
            .build());
        }
    }

    Ok(file)
}

/// Check that the VOICEVOX engine is reachable at the given URL.
async fn check_voicevox_reachable(base_url: &str) -> Result<()> {
    let client = reqwest::Client::new();
    client
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

/// Synthesize audio via the VOICEVOX engine and write to `out_path`.
async fn synthesize_voicevox(word: &str, speaker_id: &str, out_path: &PathBuf) -> Result<()> {
    let base_url =
        std::env::var("VOICEVOX_URL").unwrap_or_else(|_| "http://localhost:50021".to_string());

    check_voicevox_reachable(&base_url).await?;

    let client = reqwest::Client::new();

    let query: serde_json::Value = client
        .post(format!("{base_url}/audio_query"))
        .query(&[("text", word), ("speaker", speaker_id)])
        .send()
        .await
        .context(error::HttpSnafu)?
        .json()
        .await
        .context(error::HttpSnafu)?;

    let audio = client
        .post(format!("{base_url}/synthesis"))
        .query(&[("speaker", speaker_id)])
        .json(&query)
        .send()
        .await
        .context(error::HttpSnafu)?
        .bytes()
        .await
        .context(error::HttpSnafu)?;

    std::fs::write(out_path, &audio).context(error::IoSnafu)?;
    Ok(())
}

/// Synthesize audio using local VITS ONNX inference.
async fn synthesize_vits(model_name: &str, word: &str, out_path: &std::path::Path) -> Result<()> {
    crate::vits::synthesize(model_name, word, out_path)
        .await
        .map_err(|e| match e {
            crate::vits::VitsError::ModelNotFound { path } => {
                error::ModelNotFoundSnafu { name: path }.build()
            }
            other => error::VoicevoxSnafu {
                message: other.to_string(),
            }
            .build(),
        })
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
}
