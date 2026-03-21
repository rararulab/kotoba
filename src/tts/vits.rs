//! VITS local ONNX TTS backend.

use std::path::Path;

use async_trait::async_trait;

use super::TtsBackend;
use crate::error::{self, Result};

/// Local VITS ONNX inference backend.
pub struct VitsBackend {
    model_name: String,
}

impl VitsBackend {
    /// Create a new VITS backend for the given model.
    pub const fn new(model_name: String) -> Self { Self { model_name } }
}

#[async_trait]
impl TtsBackend for VitsBackend {
    async fn synthesize(&self, text: &str, output: &Path) -> Result<()> {
        crate::vits::synthesize(&self.model_name, text, output)
            .await
            .map_err(|e| {
                error::VoicevoxSnafu {
                    message: e.to_string(),
                }
                .build()
            })
    }

    fn name(&self) -> &'static str { "vits" }
}
