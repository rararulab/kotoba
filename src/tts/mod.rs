//! TTS backend trait and implementations.

mod kokoro;
mod vits;
mod voicevox;

use std::path::Path;

use async_trait::async_trait;
pub use kokoro::KokoroBackend;
pub use vits::VitsBackend;
pub use voicevox::VoicevoxBackend;

use crate::error::Result;

/// Common interface for text-to-speech backends.
#[async_trait]
pub trait TtsBackend: Send + Sync {
    /// Synthesize speech from text and write to output path.
    async fn synthesize(&self, text: &str, output: &Path) -> Result<()>;

    /// Backend name for display.
    fn name(&self) -> &'static str;
}
