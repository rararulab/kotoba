//! TTS backend trait and implementations.

mod cosyvoice;
mod kokoro;
mod vits;
mod voicevox;

use std::path::Path;

use async_trait::async_trait;
pub use cosyvoice::CosyvoiceBackend;
pub use kokoro::KokoroBackend;
pub use vits::VitsBackend;
pub use voicevox::{VoicevoxBackend, VoicevoxProsody};

use crate::error::Result;

/// Common interface for text-to-speech backends.
#[async_trait]
pub trait TtsBackend: Send + Sync {
    /// Synthesize speech from text and write to output path.
    async fn synthesize(&self, text: &str, output: &Path) -> Result<()>;
}
