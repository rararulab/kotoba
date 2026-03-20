//! Kokoro ONNX inference module for high-quality multi-lingual TTS.
//!
//! Pipeline: text → phoneme tokens → ONNX model inference → WAV file.

use std::path::PathBuf;

use snafu::{ResultExt, Snafu};

/// Errors that can occur during Kokoro inference.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum KokoroError {
    /// Model file not found at the expected path.
    #[snafu(display("kokoro model not found at {path} — download with `kotoba voice add kokoro`"))]
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

/// Tokenize text into phoneme IDs for Kokoro ONNX model input.
///
/// For Japanese (`ja`), converts kana to romaji first via
/// [`crate::romaji::to_romaji`], then maps each character to Kokoro's
/// phoneme vocabulary. For English (`en`), uses character-level
/// tokenization on the lowercased input.
///
/// The returned vector is framed with BOS (0) and EOS (0) tokens.
fn tokenize(text: &str, lang: &str) -> Vec<i64> {
    let phonemes = match lang {
        "ja" => crate::romaji::to_romaji(text),
        _ => text.to_lowercase(),
    };

    let mut ids: Vec<i64> = Vec::with_capacity(phonemes.len() + 2);
    ids.push(0); // BOS

    for ch in phonemes.chars() {
        let id = match ch {
            ' ' => 1,
            c if c.is_ascii_alphabetic() => i64::from(c as u8 - b'a') + 2,
            '-' => 28,  // long vowel marker
            '\'' => 29, // glottal stop
            _ => 1,     // fallback to space token
        };
        ids.push(id);
    }

    ids.push(0); // EOS
    ids
}

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
    fn tokenize_japanese_kana() {
        let tokens = tokenize("こんにちは", "ja");
        assert!(!tokens.is_empty(), "should produce tokens for Japanese kana");
        assert!(tokens.iter().all(|&t| t >= 0));
    }

    #[test]
    fn tokenize_empty_input() {
        let tokens = tokenize("", "ja");
        // At minimum BOS + EOS
        assert!(tokens.len() >= 2);
    }

    #[test]
    fn tokenize_ascii_passthrough() {
        let tokens = tokenize("hello", "en");
        assert!(!tokens.is_empty());
    }

    #[test]
    fn models_dir_is_under_kotoba() {
        let dir = models_dir().expect("models_dir should succeed");
        assert!(
            dir.ends_with(".kotoba/models/kokoro"),
            "expected path ending with .kotoba/models/kokoro, got {dir:?}"
        );
    }
}
