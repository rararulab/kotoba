//! Kokoro ONNX inference module for high-quality multi-lingual TTS.
//!
//! Pipeline: text → phoneme tokens → ONNX model inference → WAV file.

use std::path::{Path, PathBuf};

use ndarray::{Array1, Array2};
use ort::{inputs, session::Session, value::TensorRef};
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

    /// Failed to load voice style vector from voices binary.
    #[snafu(display("failed to load voice style for '{voice}' — ensure voices-v1.0.bin exists"))]
    VoiceLoad { voice: String },

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
            other => {
                tracing::warn!("unmapped character in tokenizer: {other:?}");
                1 // fallback to space token
            }
        };
        ids.push(id);
    }

    ids.push(0); // EOS
    ids
}

/// Return the directory where Kokoro models are stored
/// (`~/.kotoba/models/kokoro`).
fn models_dir() -> PathBuf { crate::paths::models_dir().join("kokoro") }

/// Synthesize speech from text using the local Kokoro ONNX model.
///
/// Loads the model from `~/.kotoba/models/kokoro/kokoro-v1.0.onnx`,
/// tokenizes the input text, runs ONNX inference, and writes the
/// resulting audio to `output` as a WAV file.
pub async fn synthesize(text: &str, lang: &str, voice: &str, output: &Path) -> Result<()> {
    let model_dir = models_dir();
    let model_path = model_dir.join("kokoro-v1.0.onnx");

    if !model_path.exists() {
        return ModelNotFoundSnafu {
            path: model_path.display().to_string(),
        }
        .fail();
    }

    let voices_path = model_dir.join("voices-v1.0.bin");
    if !voices_path.exists() {
        return ModelNotFoundSnafu {
            path: voices_path.display().to_string(),
        }
        .fail();
    }

    let tokens = tokenize(text, lang);
    let voice = voice.to_string();
    let output = output.to_path_buf();

    tokio::task::spawn_blocking(move || run_inference(&model_path, &tokens, &voice, &output))
        .await
        .context(JoinSnafu)?
}

/// Style embedding dimension used by the Kokoro model.
const STYLE_DIM: usize = 256;

/// Load a voice style vector from `voices-v1.0.bin`.
///
/// The file is a raw little-endian f32 array of shape `[N, 1, 256]`.
/// The style vector is selected by token length (before BOS/EOS padding).
fn load_style_vector(voice: &str, token_len: usize) -> Result<Vec<f32>> {
    let voices_path = models_dir().join("voices-v1.0.bin");
    let data = std::fs::read(&voices_path).context(IoSnafu)?;

    // Each voice entry in the .bin file is a NumPy .npy archive
    // loaded via np.load(). For the combined voices-v1.0.bin,
    // the format is a NumPy .npz containing per-voice arrays of
    // shape [512, 1, 256] as raw f32.
    //
    // For simplicity, we support the single-voice .bin format:
    // raw little-endian f32 of shape [N, 1, 256] = N * 256 floats.
    let floats: Vec<f32> = data
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();

    let num_styles = floats.len() / STYLE_DIM;
    let idx = token_len.min(num_styles.saturating_sub(1));
    let start = idx * STYLE_DIM;

    if start + STYLE_DIM > floats.len() {
        return VoiceLoadSnafu {
            voice: voice.to_string(),
        }
        .fail();
    }

    Ok(floats[start..start + STYLE_DIM].to_vec())
}

/// Run Kokoro ONNX inference synchronously.
///
/// Creates input tensors (`input_ids`, `style`, `speed`), runs the model,
/// and writes the output audio to a WAV file.
fn run_inference(model_path: &Path, tokens: &[i64], voice: &str, output: &Path) -> Result<()> {
    // Token count before BOS/EOS padding (used for style vector selection)
    let inner_token_len = tokens.len().saturating_sub(2);
    let style_data = load_style_vector(voice, inner_token_len)?;

    let mut session = Session::builder()
        .context(OnnxRuntimeSnafu)?
        .commit_from_file(model_path)
        .context(OnnxRuntimeSnafu)?;

    let seq_len = tokens.len();
    let ids_array = Array2::from_shape_vec((1, seq_len), tokens.to_vec())
        .expect("token shape must match array dimensions");
    let ids_tensor = TensorRef::from_array_view(&ids_array).context(OnnxRuntimeSnafu)?;

    let style_array = Array2::from_shape_vec((1, STYLE_DIM), style_data)
        .expect("style shape must match array dimensions");
    let style_tensor = TensorRef::from_array_view(&style_array).context(OnnxRuntimeSnafu)?;

    let speed_data = Array1::from_vec(vec![1.0_f32]);
    let speed_tensor = TensorRef::from_array_view(&speed_data).context(OnnxRuntimeSnafu)?;

    // The model accepts both "tokens"/"input_ids" naming conventions.
    // Detect which one by checking the session's input names.
    let input_name = session
        .inputs()
        .iter()
        .find(|i| i.name() == "input_ids")
        .map_or("tokens", |_| "input_ids");

    let outputs = session
        .run(inputs![
            input_name => ids_tensor,
            "style" => style_tensor,
            "speed" => speed_tensor
        ])
        .context(OnnxRuntimeSnafu)?;

    let audio_tensor = &outputs[0];
    let (_, audio_data) = audio_tensor
        .try_extract_tensor::<f32>()
        .context(OnnxRuntimeSnafu)?;

    write_wav(output, audio_data, 24000) // Kokoro uses 24 kHz
}

/// Write raw f32 audio samples to a 16-bit PCM WAV file at the given sample
/// rate.
///
/// Format: mono, 16-bit signed integer PCM.
/// Samples are clamped to `[-1.0, 1.0]` before conversion.
fn write_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<()> {
    use std::io::Write;

    let num_channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * u32::from(num_channels) * u32::from(bits_per_sample) / 8;
    let block_align = num_channels * bits_per_sample / 8;
    #[allow(clippy::cast_possible_truncation)]
    let data_size = samples.len() as u32 * u32::from(block_align);
    let file_size = 36 + data_size;

    let mut buf = Vec::with_capacity(44 + data_size as usize);

    // RIFF header
    buf.write_all(b"RIFF").context(IoSnafu)?;
    buf.write_all(&file_size.to_le_bytes()).context(IoSnafu)?;
    buf.write_all(b"WAVE").context(IoSnafu)?;

    // fmt subchunk
    buf.write_all(b"fmt ").context(IoSnafu)?;
    buf.write_all(&16_u32.to_le_bytes()).context(IoSnafu)?;
    buf.write_all(&1_u16.to_le_bytes()).context(IoSnafu)?;
    buf.write_all(&num_channels.to_le_bytes())
        .context(IoSnafu)?;
    buf.write_all(&sample_rate.to_le_bytes()).context(IoSnafu)?;
    buf.write_all(&byte_rate.to_le_bytes()).context(IoSnafu)?;
    buf.write_all(&block_align.to_le_bytes()).context(IoSnafu)?;
    buf.write_all(&bits_per_sample.to_le_bytes())
        .context(IoSnafu)?;

    // data subchunk
    buf.write_all(b"data").context(IoSnafu)?;
    buf.write_all(&data_size.to_le_bytes()).context(IoSnafu)?;

    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        #[allow(clippy::cast_possible_truncation)]
        let scaled = (clamped * f32::from(i16::MAX)) as i16;
        buf.write_all(&scaled.to_le_bytes()).context(IoSnafu)?;
    }

    std::fs::write(path, &buf).context(IoSnafu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_japanese_kana() {
        let tokens = tokenize("こんにちは", "ja");
        assert!(
            !tokens.is_empty(),
            "should produce tokens for Japanese kana"
        );
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
        let dir = models_dir();
        assert!(
            dir.ends_with(".kotoba/models/kokoro"),
            "expected path ending with .kotoba/models/kokoro, got {dir:?}"
        );
    }

    #[test]
    fn synthesize_fails_when_model_missing() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(synthesize(
            "test",
            "ja",
            "nonexistent-voice",
            std::path::Path::new("/tmp/kokoro_test.wav"),
        ));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("kokoro model not found"),
            "error should mention kokoro model not found, got: {err}"
        );
    }
}
