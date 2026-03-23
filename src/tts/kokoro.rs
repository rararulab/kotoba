//! Kokoro ONNX TTS backend.

use std::path::Path;

use async_trait::async_trait;
use snafu::ResultExt;

use super::TtsBackend;
use crate::error::{self, Result};

/// Kokoro ONNX inference backend for high-quality multi-lingual TTS.
pub struct KokoroBackend {
    voice: String,
    speed: f32,
}

impl KokoroBackend {
    /// Create a new Kokoro backend for the given voice style.
    pub const fn new(voice: String, speed: f32) -> Self { Self { voice, speed } }
}

#[async_trait]
impl TtsBackend for KokoroBackend {
    async fn synthesize(&self, text: &str, output: &Path) -> Result<()> {
        crate::kokoro::synthesize(text, "ja", &self.voice, self.speed, output)
            .await
            .context(error::KokoroSnafu)
    }
}
