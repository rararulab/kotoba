//! VOICEVOX TTS backend.

use std::path::Path;

use async_trait::async_trait;
use serde::Serialize;
use snafu::ResultExt;

use super::TtsBackend;
use crate::error::{self, Result};

/// Parameters for VOICEVOX audio query.
#[derive(Debug, Serialize)]
struct AudioQueryParams<'a> {
    text:    &'a str,
    speaker: &'a str,
}

/// Parameters for VOICEVOX synthesis.
#[derive(Debug, Serialize)]
struct SynthesisParams<'a> {
    speaker: &'a str,
}

/// VOICEVOX cloud/local engine backend.
pub struct VoicevoxBackend {
    base_url:   String,
    speaker_id: String,
}

impl VoicevoxBackend {
    /// Create a new VOICEVOX backend with the given base URL and speaker ID.
    pub const fn new(base_url: String, speaker_id: String) -> Self {
        Self {
            base_url,
            speaker_id,
        }
    }
}

#[async_trait]
impl TtsBackend for VoicevoxBackend {
    async fn synthesize(&self, text: &str, output: &Path) -> Result<()> {
        let client = crate::http::client();

        let params = AudioQueryParams {
            text,
            speaker: &self.speaker_id,
        };

        let query: serde_json::Value = client
            .post(format!("{}/audio_query", self.base_url))
            .query(&params)
            .send()
            .await
            .context(error::HttpSnafu)?
            .json()
            .await
            .context(error::HttpSnafu)?;

        let synthesis_params = SynthesisParams {
            speaker: &self.speaker_id,
        };

        let audio = client
            .post(format!("{}/synthesis", self.base_url))
            .query(&synthesis_params)
            .json(&query)
            .send()
            .await
            .context(error::HttpSnafu)?
            .bytes()
            .await
            .context(error::HttpSnafu)?;

        std::fs::write(output, &audio).context(error::IoSnafu)?;
        Ok(())
    }

    fn name(&self) -> &'static str { "voicevox" }
}
