//! Kokoro ONNX TTS backend.

use std::path::Path;

use async_trait::async_trait;

use super::TtsBackend;
use crate::error::{self, Result};

/// Kokoro ONNX inference backend for high-quality multi-lingual TTS.
pub struct KokoroBackend {
    voice: String,
}

impl KokoroBackend {
    /// Create a new Kokoro backend for the given voice style.
    pub const fn new(voice: String) -> Self { Self { voice } }
}

#[async_trait]
impl TtsBackend for KokoroBackend {
    async fn synthesize(&self, text: &str, output: &Path) -> Result<()> {
        crate::kokoro::synthesize(text, "ja", &self.voice, output)
            .await
            .map_err(|e| {
                error::KokoroSnafu {
                    message: e.to_string(),
                }
                .build()
            })
    }

    fn name(&self) -> &'static str { "kokoro" }
}
