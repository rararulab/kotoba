//! VOICEVOX TTS backend.

use std::path::Path;

use async_trait::async_trait;
use serde::Serialize;
use snafu::ResultExt;

use super::TtsBackend;
use crate::error::{self, Result};

/// Per-utterance prosody controls for VOICEVOX synthesis.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_field_names)]
pub struct VoicevoxProsody {
    pub speed_scale:      f32,
    pub pitch_scale:      f32,
    pub intonation_scale: f32,
}

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
    prosody:    Option<VoicevoxProsody>,
}

impl VoicevoxBackend {
    /// Create a new VOICEVOX backend with the given base URL and speaker ID.
    pub const fn new(base_url: String, speaker_id: String) -> Self {
        Self {
            base_url,
            speaker_id,
            prosody: None,
        }
    }

    /// Override VOICEVOX prosody values for this synthesis request.
    pub const fn with_prosody(mut self, prosody: VoicevoxProsody) -> Self {
        self.prosody = Some(prosody);
        self
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

        let mut query: serde_json::Value = client
            .post(format!("{}/audio_query", self.base_url))
            .query(&params)
            .send()
            .await
            .context(error::HttpSnafu)?
            .json()
            .await
            .context(error::HttpSnafu)?;

        if let Some(prosody) = self.prosody {
            apply_prosody(&mut query, prosody);
        }

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
}

fn apply_prosody(query: &mut serde_json::Value, prosody: VoicevoxProsody) {
    if let Some(obj) = query.as_object_mut() {
        obj.insert(
            "speedScale".to_string(),
            serde_json::json!(prosody.speed_scale.clamp(0.5, 2.0)),
        );
        obj.insert(
            "pitchScale".to_string(),
            serde_json::json!(prosody.pitch_scale.clamp(-0.3, 0.3)),
        );
        obj.insert(
            "intonationScale".to_string(),
            serde_json::json!(prosody.intonation_scale.clamp(0.5, 2.0)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_prosody_overrides_and_clamps_fields() {
        let mut query = serde_json::json!({
            "speedScale": 1.0,
            "pitchScale": 0.0,
            "intonationScale": 1.0
        });

        apply_prosody(
            &mut query,
            VoicevoxProsody {
                speed_scale:      9.9,
                pitch_scale:      -9.9,
                intonation_scale: 0.1,
            },
        );

        let speed = query["speedScale"]
            .as_f64()
            .expect("speedScale should be numeric");
        let pitch = query["pitchScale"]
            .as_f64()
            .expect("pitchScale should be numeric");
        let intonation = query["intonationScale"]
            .as_f64()
            .expect("intonationScale should be numeric");

        assert!((speed - 2.0).abs() < 1.0e-6);
        assert!((pitch + 0.3).abs() < 1.0e-6);
        assert!((intonation - 0.5).abs() < 1.0e-6);
    }
}
