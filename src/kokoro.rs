//! Kokoro ONNX inference module for local text-to-speech synthesis.

use std::path::PathBuf;

use snafu::{ResultExt, Snafu};

/// Errors that can occur during Kokoro inference.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum KokoroError {
    /// Model file not found at the expected path.
    #[snafu(display("model not found: {path}"))]
    ModelNotFound { path: String },

    /// ONNX runtime error during session creation or inference.
    #[snafu(display("onnx runtime error: {source}"))]
    OnnxRuntime { source: ort::Error },

    /// I/O error when reading or writing files.
    #[snafu(display("io error: {source}"))]
    Io { source: std::io::Error },

    /// Blocking task join error.
    #[snafu(display("join error: {source}"))]
    Join { source: tokio::task::JoinError },
}

/// Module-level result type.
pub type Result<T> = std::result::Result<T, KokoroError>;

/// Return the directory where Kokoro models are stored (`~/.kotoba/models/kokoro`).
fn models_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "home directory not found")
    }).context(IoSnafu)?;
    Ok(home.join(".kotoba").join("models").join("kokoro"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_dir_is_under_kotoba() {
        let dir = models_dir().expect("models_dir should succeed");
        assert!(
            dir.ends_with(".kotoba/models/kokoro"),
            "expected path ending with .kotoba/models/kokoro, got {dir:?}"
        );
    }
}
