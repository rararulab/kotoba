//! VITS ONNX inference module for local text-to-speech synthesis.
//!
//! Pipeline: kana text -> phoneme IDs -> ONNX model inference -> WAV file.

use std::path::Path;

use ndarray::Array2;
use ort::{inputs, session::Session, value::TensorRef};
use snafu::{ResultExt, Snafu};
use wana_kana::ConvertJapanese;

/// Errors that can occur during VITS inference.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum VitsError {
    /// Model file not found at the expected path.
    #[snafu(display("model not found: {path}"))]
    ModelNotFound { path: String },

    /// ONNX runtime error during session creation or inference.
    #[snafu(display("onnx runtime error: {source}"))]
    OnnxRuntime { source: ort::Error },

    /// I/O error when writing audio output.
    #[snafu(display("io error: {source}"))]
    Io { source: std::io::Error },

    /// Blocking task join error.
    #[snafu(display("join error: {source}"))]
    Join { source: tokio::task::JoinError },

    /// Home directory could not be determined.
    #[snafu(display("home directory not found"))]
    HomeNotFound,
}

/// Module-level result type.
pub type Result<T> = std::result::Result<T, VitsError>;

/// Convert kana text to a sequence of phoneme IDs for VITS input.
///
/// The encoding uses a simple ASCII-offset scheme:
/// - BOS/EOS padding: 0
/// - Space: 1
/// - Printable ASCII characters (`0x21..=0x7E`): code - `0x20` + 1
///
/// Kana characters are first converted to ASCII phoneme approximations,
/// then each character is mapped to its ID with inter-phoneme padding (0)
/// inserted between characters.
fn text_to_phoneme_ids(text: &str) -> Vec<i64> {
    let ascii = kana_to_ascii(text);

    let mut ids = vec![0_i64]; // BOS pad

    for (i, ch) in ascii.chars().enumerate() {
        if i > 0 {
            ids.push(0); // inter-phoneme padding
        }
        let id = match ch {
            ' ' => 1,
            c if (0x21..=0x7E).contains(&(c as u32)) => i64::from(c as u8 - 0x20 + 1),
            _ => 1, // fallback to space for unmapped characters
        };
        ids.push(id);
    }

    ids.push(0); // EOS pad
    ids
}

/// Convert kana text to ASCII phoneme approximations.
///
/// Delegates to `wana_kana` for romaji conversion, then strips the
/// apostrophe used for ん disambiguation (not needed for phoneme IDs).
/// Non-kana, non-ASCII characters become spaces.
fn kana_to_ascii(text: &str) -> String {
    let romaji = text.to_romaji();
    // Strip apostrophes (ん disambiguation) — not meaningful for phoneme encoding
    let stripped = romaji.replace('\'', "");
    stripped
        .chars()
        .map(|ch| if ch.is_ascii() { ch } else { ' ' })
        .collect()
}

/// Synthesize speech from kana text using a local VITS ONNX model.
///
/// Loads the model from `~/.kotoba/models/{model_name}/model.onnx`,
/// converts the input text to phoneme IDs, runs ONNX inference, and
/// writes the resulting audio to the specified output path as a WAV file.
///
/// The inference runs inside `spawn_blocking` to avoid blocking the
/// async runtime.
pub async fn synthesize(model_name: &str, text: &str, output: &Path) -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| HomeNotFoundSnafu.build())?;
    let model_path = home
        .join(".kotoba")
        .join("models")
        .join(model_name)
        .join("model.onnx");

    if !model_path.exists() {
        return ModelNotFoundSnafu {
            path: model_path.display().to_string(),
        }
        .fail();
    }

    let phoneme_ids = text_to_phoneme_ids(text);
    let output = output.to_path_buf();

    tokio::task::spawn_blocking(move || run_inference(&model_path, &phoneme_ids, &output))
        .await
        .context(JoinSnafu)?
}

/// Run VITS ONNX inference synchronously.
///
/// Creates input tensors (phoneme IDs, lengths, scales), runs the model,
/// and writes the output audio to a WAV file.
fn run_inference(model_path: &Path, phoneme_ids: &[i64], output: &Path) -> Result<()> {
    let mut session = Session::builder()
        .context(OnnxRuntimeSnafu)?
        .commit_from_file(model_path)
        .context(OnnxRuntimeSnafu)?;

    let seq_len = phoneme_ids.len();

    // Shape: [1, seq_len] — batch of 1 utterance
    let ids_array = Array2::from_shape_vec((1, seq_len), phoneme_ids.to_vec())
        .expect("phoneme_ids shape must match array dimensions");

    let ids_tensor = TensorRef::from_array_view(&ids_array).context(OnnxRuntimeSnafu)?;

    // Input lengths tensor: [1] — single utterance length
    #[allow(clippy::cast_possible_wrap)] // seq_len is always small (phoneme count)
    let lengths_data = vec![seq_len as i64];
    let lengths_tensor =
        TensorRef::from_array_view(([1usize], &*lengths_data)).context(OnnxRuntimeSnafu)?;

    // Scales tensor: [noise_scale, length_scale, noise_w]
    let scales_data: Vec<f32> = vec![0.667, 1.0, 0.8];
    let scales_tensor =
        TensorRef::from_array_view(([3usize], &*scales_data)).context(OnnxRuntimeSnafu)?;

    let outputs = session
        .run(inputs![
            "input" => ids_tensor,
            "input_lengths" => lengths_tensor,
            "scales" => scales_tensor
        ])
        .context(OnnxRuntimeSnafu)?;

    let audio_tensor = &outputs[0];
    let (_, audio_data) = audio_tensor
        .try_extract_tensor::<f32>()
        .context(OnnxRuntimeSnafu)?;

    write_wav(output, audio_data)
}

/// Write raw f32 audio samples to a 16-bit PCM WAV file.
///
/// Format: 22050 Hz, mono, 16-bit signed integer PCM.
/// Samples are clamped to `[-1.0, 1.0]` before conversion.
fn write_wav(path: &Path, samples: &[f32]) -> Result<()> {
    use std::io::Write;

    let sample_rate: u32 = 22050;
    let num_channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * u32::from(num_channels) * u32::from(bits_per_sample) / 8;
    let block_align = num_channels * bits_per_sample / 8;
    #[allow(clippy::cast_possible_truncation)] // audio data fits in u32 for practical utterances
    let data_size = samples.len() as u32 * u32::from(block_align);
    let file_size = 36 + data_size; // RIFF header minus 8 bytes + data

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
        #[allow(clippy::cast_possible_truncation)] // intentional float-to-int conversion for PCM
        let scaled = (clamped * f32::from(i16::MAX)) as i16;
        buf.write_all(&scaled.to_le_bytes()).context(IoSnafu)?;
    }

    std::fs::write(path, &buf).context(IoSnafu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phoneme_ids_has_bos_eos_padding() {
        let ids = text_to_phoneme_ids("a");
        assert_eq!(ids[0], 0, "must start with BOS pad");
        assert_eq!(
            *ids.last().expect("ids must not be empty"),
            0,
            "must end with EOS pad"
        );
    }

    #[test]
    fn phoneme_ids_has_inter_phoneme_padding() {
        let ids = text_to_phoneme_ids("ab");
        // Expected: [0, id_a, 0, id_b, 0]
        assert_eq!(ids.len(), 5);
        assert_eq!(ids[2], 0, "inter-phoneme padding between a and b");
    }

    #[test]
    fn kana_converts_to_ascii() {
        assert_eq!(kana_to_ascii("あ"), "a");
        assert_eq!(kana_to_ascii("か"), "ka");
        assert_eq!(kana_to_ascii("カ"), "ka");
        assert_eq!(kana_to_ascii("hello"), "hello");
        // Apostrophes from ん disambiguation are stripped for phoneme encoding
        assert_eq!(kana_to_ascii("おんよみ"), "onyomi");
    }

    #[test]
    fn phoneme_ids_padding_structure() {
        let ids = text_to_phoneme_ids("abc");
        // BOS + a + pad + b + pad + c + EOS = 7
        assert_eq!(ids.len(), 7);
        assert_eq!(ids[0], 0, "BOS");
        assert_eq!(ids[2], 0, "padding after a");
        assert_eq!(ids[4], 0, "padding after b");
        assert_eq!(ids[6], 0, "EOS");
    }

    #[test]
    fn phoneme_ids_empty_input() {
        let ids = text_to_phoneme_ids("");
        // Only BOS + EOS
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], 0);
        assert_eq!(ids[1], 0);
    }

    #[test]
    fn phoneme_ids_space_encoding() {
        let ids = text_to_phoneme_ids(" ");
        // BOS + space(1) + EOS
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[1], 1, "space should map to 1");
    }

    #[test]
    fn write_wav_produces_valid_header() {
        let dir = std::env::temp_dir().join("kotoba_test_wav");
        std::fs::create_dir_all(&dir).expect("failed to create temp dir");
        let path = dir.join("test.wav");
        let samples = vec![0.0_f32; 100];
        write_wav(&path, &samples).expect("write_wav failed");

        let data = std::fs::read(&path).expect("failed to read wav");
        assert_eq!(&data[0..4], b"RIFF");
        assert_eq!(&data[8..12], b"WAVE");
        assert_eq!(&data[12..16], b"fmt ");

        std::fs::remove_dir_all(&dir).ok();
    }
}
