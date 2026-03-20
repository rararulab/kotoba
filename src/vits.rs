//! VITS ONNX inference module for local text-to-speech synthesis.
//!
//! Pipeline: kana text -> phoneme IDs -> ONNX model inference -> WAV file.

use std::path::Path;

use ndarray::Array2;
use ort::{inputs, session::Session, value::TensorRef};
use snafu::{ResultExt, Snafu};

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

/// Map a single kana character to its ASCII phoneme representation.
///
/// Returns `None` for characters that need special handling (ASCII passthrough
/// or unknown characters).
const fn kana_char_to_phoneme(ch: char) -> Option<&'static str> {
    // Merge hiragana and katakana by matching both in the same arm.
    // Katakana codepoints are hiragana + 0x60.
    match ch {
        // Vowels
        '\u{3042}' | '\u{30A2}' => Some("a"),
        '\u{3044}' | '\u{30A4}' => Some("i"),
        '\u{3046}' | '\u{30A6}' => Some("u"),
        '\u{3048}' | '\u{30A8}' => Some("e"),
        '\u{304A}' | '\u{30AA}' => Some("o"),
        // K-row
        '\u{304B}' | '\u{30AB}' => Some("ka"),
        '\u{304D}' | '\u{30AD}' => Some("ki"),
        '\u{304F}' | '\u{30AF}' => Some("ku"),
        '\u{3051}' | '\u{30B1}' => Some("ke"),
        '\u{3053}' | '\u{30B3}' => Some("ko"),
        // S-row
        '\u{3055}' | '\u{30B5}' => Some("sa"),
        '\u{3057}' | '\u{30B7}' => Some("shi"),
        '\u{3059}' | '\u{30B9}' => Some("su"),
        '\u{305B}' | '\u{30BB}' => Some("se"),
        '\u{305D}' | '\u{30BD}' => Some("so"),
        // T-row
        '\u{305F}' | '\u{30BF}' => Some("ta"),
        '\u{3061}' | '\u{30C1}' => Some("chi"),
        '\u{3064}' | '\u{30C4}' => Some("tsu"),
        '\u{3066}' | '\u{30C6}' => Some("te"),
        '\u{3068}' | '\u{30C8}' => Some("to"),
        // N-row
        '\u{306A}' | '\u{30CA}' => Some("na"),
        '\u{306B}' | '\u{30CB}' => Some("ni"),
        '\u{306C}' | '\u{30CC}' => Some("nu"),
        '\u{306D}' | '\u{30CD}' => Some("ne"),
        '\u{306E}' | '\u{30CE}' => Some("no"),
        // H-row
        '\u{306F}' | '\u{30CF}' => Some("ha"),
        '\u{3072}' | '\u{30D2}' => Some("hi"),
        '\u{3075}' | '\u{30D5}' => Some("fu"),
        '\u{3078}' | '\u{30D8}' => Some("he"),
        '\u{307B}' | '\u{30DB}' => Some("ho"),
        // M-row
        '\u{307E}' | '\u{30DE}' => Some("ma"),
        '\u{307F}' | '\u{30DF}' => Some("mi"),
        '\u{3080}' | '\u{30E0}' => Some("mu"),
        '\u{3081}' | '\u{30E1}' => Some("me"),
        '\u{3082}' | '\u{30E2}' => Some("mo"),
        // Y-row
        '\u{3084}' | '\u{30E4}' => Some("ya"),
        '\u{3086}' | '\u{30E6}' => Some("yu"),
        '\u{3088}' | '\u{30E8}' => Some("yo"),
        // R-row
        '\u{3089}' | '\u{30E9}' => Some("ra"),
        '\u{308A}' | '\u{30EA}' => Some("ri"),
        '\u{308B}' | '\u{30EB}' => Some("ru"),
        '\u{308C}' | '\u{30EC}' => Some("re"),
        '\u{308D}' | '\u{30ED}' => Some("ro"),
        // W-row + N
        '\u{308F}' | '\u{30EF}' => Some("wa"),
        '\u{3092}' | '\u{30F2}' => Some("wo"),
        '\u{3093}' | '\u{30F3}' => Some("n"),
        // Long vowel mark
        '\u{30FC}' => Some("-"),
        _ => None,
    }
}

/// Convert kana text to ASCII phoneme approximations.
///
/// Maps hiragana and katakana to romaji-like ASCII strings.
/// Characters that are already ASCII pass through unchanged.
fn kana_to_ascii(text: &str) -> String {
    let mut result = String::new();
    for ch in text.chars() {
        if let Some(phoneme) = kana_char_to_phoneme(ch) {
            result.push_str(phoneme);
        } else if ch.is_ascii() {
            result.push(ch);
        } else {
            // Unknown non-ASCII characters become space
            result.push(' ');
        }
    }
    result
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
    let home = dirs::home_dir().expect("home directory must exist");
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
        assert_eq!(kana_to_ascii("\u{3042}"), "a");
        assert_eq!(kana_to_ascii("\u{304B}"), "ka");
        assert_eq!(kana_to_ascii("\u{30AB}"), "ka");
        assert_eq!(kana_to_ascii("hello"), "hello");
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
