//! Kokoro ONNX inference module for high-quality multi-lingual TTS.
//!
//! Pipeline: text → phoneme tokens → ONNX model inference → WAV file.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use ndarray::{Array1, Array2, Array3, Axis};
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

    /// Failed to phonemize text with Kokoro-compatible tokenizer.
    #[snafu(display("kokoro tokenizer error: {message}"))]
    Tokenizer { message: String },
}

/// Module-level result type.
pub type Result<T> = std::result::Result<T, KokoroError>;

/// Tokenize text into phoneme IDs for Kokoro ONNX model input.
///
/// Preferred path uses the official `kokoro-onnx` Python tokenizer
/// (`phonemizer` + `espeak-ng`) via subprocess and returns Kokoro vocab IDs.
/// For non-Japanese languages, falls back to a lightweight approximate mapping
/// when the Python tokenizer is unavailable.
///
/// The returned vector is framed with BOS (0) and EOS (0) tokens.
fn tokenize(text: &str, lang: &str) -> Result<Vec<i64>> {
    let python_tokens = tokenize_with_kokoro_python(text, lang);
    if let Ok(core_tokens) = python_tokens {
        let mut tokens = Vec::with_capacity(core_tokens.len() + 2);
        tokens.push(0); // BOS
        tokens.extend(core_tokens);
        tokens.push(0); // EOS
        return Ok(tokens);
    }

    if lang == "ja" {
        let message = python_tokens
            .err()
            .unwrap_or_else(|| "unknown tokenizer failure".to_string());
        return TokenizerSnafu { message }.fail();
    }

    // Fallback tokenizer: only used when python kokoro tokenizer is unavailable.
    // This is approximate and less accurate than official phonemization.
    let phonemes = match lang {
        "ja" => crate::romaji::to_romaji(text).to_lowercase(),
        _ => text.to_lowercase(),
    };

    let mut ids: Vec<i64> = Vec::with_capacity(phonemes.len() + 2);
    ids.push(0); // BOS

    for ch in phonemes.chars() {
        if let Some(id) = kokoro_vocab_id(ch) {
            ids.push(id);
        }
    }

    ids.push(0); // EOS
    Ok(ids)
}

const fn kokoro_vocab_id(ch: char) -> Option<i64> {
    match ch {
        ';' => Some(1),
        ':' => Some(2),
        ',' => Some(3),
        '.' => Some(4),
        '!' => Some(5),
        '?' => Some(6),
        ' ' => Some(16),
        'a' => Some(43),
        'b' => Some(44),
        'c' => Some(45),
        'd' => Some(46),
        'e' => Some(47),
        'f' => Some(48),
        // Kokoro vocab uses IPA small script g (U+0261), but romaji contains
        // ASCII 'g'. Map it explicitly.
        'g' | 'ɡ' => Some(92),
        'h' => Some(50),
        'i' => Some(51),
        'j' => Some(52),
        'k' => Some(53),
        'l' => Some(54),
        'm' => Some(55),
        'n' => Some(56),
        'o' => Some(57),
        'p' => Some(58),
        'q' => Some(59),
        'r' => Some(60),
        's' => Some(61),
        't' => Some(62),
        'u' => Some(63),
        'v' => Some(64),
        'w' => Some(65),
        'x' => Some(66),
        'y' => Some(67),
        'z' => Some(68),
        _ => None,
    }
}

/// Python script that tokenizes text using `kokoro-onnx` and `misaki[ja]`.
///
/// For Japanese, misaki's pyopenjtalk G2P returns `phonemes + pitch` as a
/// single concatenated string (each half equal length). We split to extract
/// only the phoneme half, then normalize characters that are not in the
/// Kokoro tokenizer vocabulary:
///   - Palatalized consonants (U+1D80–U+1D89) → base + `j`
///   - ASCII `g` (U+0067) → IPA `ɡ` (U+0261)
const KOKORO_TOKENIZER_SCRIPT: &str = r#"
import json
import sys
from kokoro_onnx.tokenizer import Tokenizer

PALATAL_MAP = {
    "\u1D80": "bj",
    "\u1D83": "\u0261j",
    "\u1D84": "kj",
    "\u1D86": "mj",
    "\u1D88": "pj",
    "\u1D89": "rj",
}

def normalize_ja_phonemes(raw):
    """Extract phoneme half and decompose palatalized chars for Kokoro."""
    phonemes = raw[:len(raw) // 2]
    out = []
    for ch in phonemes:
        if ch in PALATAL_MAP:
            out.append(PALATAL_MAP[ch])
        elif ch == "g":
            out.append("\u0261")
        else:
            out.append(ch)
    return "".join(out)

text = sys.argv[1]
lang = sys.argv[2]
tokenizer = Tokenizer()

if lang == "ja":
    from misaki import ja as misaki_ja
    g2p = misaki_ja.JAG2P(version="pyopenjtalk")
    raw, _ = g2p(text)
    phonemes = normalize_ja_phonemes(raw)
else:
    phonemes = tokenizer.phonemize(text, lang)

tokens = tokenizer.tokenize(phonemes)
print(json.dumps(tokens))
"#;

fn tokenize_with_kokoro_python(text: &str, lang: &str) -> std::result::Result<Vec<i64>, String> {
    if lang == "ja" {
        if let Ok(tokens) = run_uv_tokenizer(text, lang) {
            return Ok(tokens);
        }
        return run_python_tokenizer("python3", &["-c"], text, lang);
    }

    if let Ok(tokens) = run_python_tokenizer("python3", &["-c"], text, lang) {
        return Ok(tokens);
    }

    run_uv_tokenizer(text, lang)
}

fn run_python_tokenizer(
    python_bin: &str,
    prefix_args: &[&str],
    text: &str,
    lang: &str,
) -> std::result::Result<Vec<i64>, String> {
    let mut cmd = Command::new(python_bin);
    cmd.args(prefix_args)
        .arg(KOKORO_TOKENIZER_SCRIPT)
        .arg(text)
        .arg(lang);
    parse_tokenizer_output(&cmd.output().map_err(|e| e.to_string())?)
}

fn run_uv_tokenizer(text: &str, lang: &str) -> std::result::Result<Vec<i64>, String> {
    let mut cmd = Command::new("uv");
    cmd.arg("run")
        .arg("--quiet")
        .arg("--with")
        .arg("kokoro-onnx");

    if lang == "ja" {
        // Follow common open-source Kokoro JA pipeline:
        // misaki + pyopenjtalk for Japanese G2P.
        cmd.arg("--with")
            .arg("misaki[ja]")
            .arg("--with")
            .arg("pyopenjtalk");
    }

    cmd.arg("python")
        .arg("-c")
        .arg(KOKORO_TOKENIZER_SCRIPT)
        .arg(text)
        .arg(lang);
    parse_tokenizer_output(&cmd.output().map_err(|e| e.to_string())?)
}

fn parse_tokenizer_output(output: &std::process::Output) -> std::result::Result<Vec<i64>, String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "tokenizer command failed (status {}): {}",
            output.status, stderr
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| "tokenizer returned empty output".to_string())?;

    serde_json::from_str::<Vec<i64>>(json_line)
        .map_err(|e| format!("failed to parse tokenizer JSON: {e}"))
}

/// Return the directory where Kokoro models are stored
/// (`~/.kotoba/models/kokoro`).
fn models_dir() -> PathBuf { crate::paths::models_dir().join("kokoro") }

/// Synthesize speech from text using the local Kokoro ONNX model.
///
/// Loads the model from `~/.kotoba/models/kokoro/kokoro-v1.0.onnx`,
/// tokenizes the input text, runs ONNX inference, and writes the
/// resulting audio to `output` as a WAV file.
pub async fn synthesize(
    text: &str,
    lang: &str,
    voice: &str,
    speed: f32,
    output: &Path,
) -> Result<()> {
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

    let tokens = tokenize(text, lang)?;
    let voice = voice.to_string();
    let speed = speed.clamp(0.5, 2.0);
    let output = output.to_path_buf();

    tokio::task::spawn_blocking(move || run_inference(&model_path, &tokens, &voice, speed, &output))
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
    extract_style_from_npz(&data, voice, token_len)
}

fn extract_style_from_npz(data: &[u8], voice: &str, token_len: usize) -> Result<Vec<f32>> {
    let voice_name = voice.to_string();

    if let Ok(style) = read_style_array_3d(data, voice) {
        let num_styles = style.shape()[0];
        if num_styles == 0 {
            return VoiceLoadSnafu { voice: voice_name }.fail();
        }
        let idx = token_len.min(num_styles.saturating_sub(1));
        let selected = style.index_axis(Axis(0), idx);
        let vector: Vec<f32> = selected.iter().copied().collect();
        if vector.len() != STYLE_DIM {
            return VoiceLoadSnafu { voice: voice_name }.fail();
        }
        return Ok(vector);
    }

    let style = read_style_array_2d(data, voice).map_err(|()| {
        VoiceLoadSnafu {
            voice: voice_name.clone(),
        }
        .build()
    })?;
    let num_styles = style.shape()[0];
    if num_styles == 0 {
        return VoiceLoadSnafu { voice: voice_name }.fail();
    }
    let idx = token_len.min(num_styles.saturating_sub(1));
    let row = style.index_axis(Axis(0), idx).to_vec();
    if row.len() != STYLE_DIM {
        return VoiceLoadSnafu { voice: voice_name }.fail();
    }
    Ok(row)
}

fn read_style_array_3d(data: &[u8], voice: &str) -> std::result::Result<Array3<f32>, ()> {
    let mut npz = ndarray_npy::NpzReader::new(std::io::Cursor::new(data)).map_err(|_| ())?;
    npz.by_name(voice).map_err(|_| ())
}

fn read_style_array_2d(data: &[u8], voice: &str) -> std::result::Result<Array2<f32>, ()> {
    let mut npz = ndarray_npy::NpzReader::new(std::io::Cursor::new(data)).map_err(|_| ())?;
    npz.by_name(voice).map_err(|_| ())
}

/// Run Kokoro ONNX inference synchronously.
///
/// Creates input tensors (`input_ids`, `style`, `speed`), runs the model,
/// and writes the output audio to a WAV file.
fn run_inference(
    model_path: &Path,
    tokens: &[i64],
    voice: &str,
    speed: f32,
    output: &Path,
) -> Result<()> {
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

    let speed_data = Array1::from_vec(vec![speed]);
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
    use ndarray::{Array3, arr1};

    use super::*;

    fn is_missing_kokoro_python_dep(err: &KokoroError) -> bool {
        let message = err.to_string();
        message.contains("No module named 'kokoro_onnx'")
            || message.contains("No module named 'misaki'")
            || message.contains("No such file or directory")
    }

    #[test]
    fn tokenize_japanese_kana() {
        let tokens = match tokenize("こんにちは", "ja") {
            Ok(tokens) => tokens,
            Err(err) if is_missing_kokoro_python_dep(&err) => {
                eprintln!("skipping tokenize_japanese_kana: {err}");
                return;
            }
            Err(err) => panic!("tokenize should succeed: {err}"),
        };
        assert!(
            !tokens.is_empty(),
            "should produce tokens for Japanese kana"
        );
        assert!(tokens.iter().all(|&t| t >= 0));
    }

    #[test]
    fn tokenize_empty_input() {
        let tokens = match tokenize("", "ja") {
            Ok(tokens) => tokens,
            Err(err) if is_missing_kokoro_python_dep(&err) => {
                eprintln!("skipping tokenize_empty_input: {err}");
                return;
            }
            Err(err) => panic!("tokenize should succeed: {err}"),
        };
        // At minimum BOS + EOS
        assert!(tokens.len() >= 2);
    }

    #[test]
    fn tokenize_ascii_passthrough() {
        let tokens = tokenize("hello", "en").expect("tokenize should succeed");
        assert!(!tokens.is_empty());
    }

    #[test]
    fn tokenize_uses_kokoro_vocab_ids() {
        // In Kokoro's official vocab, 'a' is 43 (not 2).
        let tokens = tokenize("a", "en").expect("tokenize should succeed");
        assert_eq!(tokens, vec![0, 43, 0]);
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
            1.0,
            std::path::Path::new("/tmp/kokoro_test.wav"),
        ));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("kokoro model not found") || err.contains("failed to load voice style"),
            "error should mention missing model or voice style, got: {err}"
        );
    }

    #[test]
    fn extract_style_from_npz_selects_row_by_token_len() {
        let mut style = Array3::<f32>::zeros((4, 1, STYLE_DIM));
        style[[0, 0, 0]] = 10.0;
        style[[1, 0, 0]] = 20.0;
        style[[2, 0, 0]] = 30.0;
        style[[3, 0, 0]] = 40.0;

        let mut out = std::io::Cursor::new(Vec::<u8>::new());
        let mut npz = ndarray_npy::NpzWriter::new(&mut out);
        npz.add_array("af_heart", &style).expect("add array");
        npz.finish().expect("finish npz");

        let data = out.into_inner();
        let selected = extract_style_from_npz(&data, "af_heart", 2).expect("extract");
        assert_eq!(selected.len(), STYLE_DIM);
        assert!((selected[0] - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    fn extract_style_from_npz_fails_for_missing_voice() {
        let style = arr1(&[1.0_f32, 2.0_f32]);
        let mut out = std::io::Cursor::new(Vec::<u8>::new());
        let mut npz = ndarray_npy::NpzWriter::new(&mut out);
        npz.add_array("af_heart", &style).expect("add array");
        npz.finish().expect("finish npz");

        let data = out.into_inner();
        let err = extract_style_from_npz(&data, "missing", 0)
            .expect_err("missing voice should return error")
            .to_string();
        assert!(err.contains("failed to load voice style"));
    }

    #[test]
    fn tokenizer_script_uses_misaki_for_japanese() {
        assert!(KOKORO_TOKENIZER_SCRIPT.contains("misaki_ja.JAG2P"));
        assert!(KOKORO_TOKENIZER_SCRIPT.contains("version=\"pyopenjtalk\""));
    }

    /// Verify that every common Japanese phoneme category tokenizes to
    /// non-empty, BOS/EOS-framed token sequences with no dropped characters.
    ///
    /// This catches regressions in the misaki G2P → normalize → Kokoro
    /// tokenizer pipeline. Each word is chosen to exercise a specific
    /// phoneme class (palatalized consonants, geminate stops, long vowels,
    /// etc.).
    #[test]
    fn tokenize_japanese_phoneme_coverage() {
        let cases: &[(&str, &str)] = &[
            // 清音 (seion)
            ("あいうえお", "vowels"),
            ("かきくけこ", "ka-row"),
            ("さしすせそ", "sa-row"),
            ("たちつてと", "ta-row"),
            ("なにぬねの", "na-row"),
            ("はひふへほ", "ha-row"),
            ("まみむめも", "ma-row"),
            ("やゆよ", "ya-row"),
            ("らりるれろ", "ra-row"),
            ("わをん", "wa-row"),
            // 濁音・半濁音 (dakuon)
            ("がぎぐげご", "ga-row"),
            ("ざじずぜぞ", "za-row"),
            ("だぢづでど", "da-row"),
            ("ばびぶべぼ", "ba-row"),
            ("ぱぴぷぺぽ", "pa-row"),
            // 拗音 (youon) — palatalized consonants
            ("きゃきゅきょ", "kya-row"),
            ("しゃしゅしょ", "sha-row"),
            ("ちゃちゅちょ", "cha-row"),
            ("にゃにゅにょ", "nya-row"),
            ("ひゃひゅひょ", "hya-row"),
            ("みゃみゅみょ", "mya-row"),
            ("りゃりゅりょ", "rya-row"),
            ("ぎゃぎゅぎょ", "gya-row"),
            ("じゃじゅじょ", "ja-row"),
            ("びゃびゅびょ", "bya-row"),
            ("ぴゃぴゅぴょ", "pya-row"),
            // 促音 (geminate)
            ("いっぱい", "ippai"),
            ("ちょっと", "chotto"),
            ("がっこう", "gakkou"),
            // 長音 (long vowels)
            ("おはよう", "ohayou"),
            ("ありがとう", "arigatou"),
            ("先生", "sensei"),
            // 腭化漢字詞 (palatalized kanji — past regression)
            ("今日", "kyou"),
            ("東京", "toukyou"),
            ("京都", "kyouto"),
            ("勉強", "benkyou"),
            ("教室", "kyoushitsu"),
            ("牛乳", "gyuunyuu"),
            ("病院", "byouin"),
            ("旅行", "ryokou"),
            // 一般実用
            ("こんにちは", "konnichiwa"),
            ("食べる", "taberu"),
            ("日本語", "nihongo"),
            ("写真", "shashin"),
            ("電車", "densha"),
            ("成功", "seikou"),
            ("友達", "tomodachi"),
        ];

        for &(word, label) in cases {
            let tokens = match tokenize(word, "ja") {
                Ok(tokens) => tokens,
                Err(err) if is_missing_kokoro_python_dep(&err) => {
                    eprintln!("skipping tokenize_japanese_phoneme_coverage ({label}): {err}");
                    return;
                }
                Err(err) => panic!("{label} ({word}): tokenize failed: {err}"),
            };

            // Must have BOS + at least one real token + EOS
            assert!(
                tokens.len() >= 3,
                "{label} ({word}): too few tokens ({} total, expected ≥3): {tokens:?}",
                tokens.len()
            );

            // BOS and EOS must be 0
            assert_eq!(
                tokens[0], 0,
                "{label} ({word}): first token should be BOS(0), got {}",
                tokens[0]
            );
            assert_eq!(
                *tokens.last().unwrap(),
                0,
                "{label} ({word}): last token should be EOS(0)"
            );

            // Inner tokens must all be positive (no unknown/dropped chars)
            let inner = &tokens[1..tokens.len() - 1];
            assert!(
                inner.iter().all(|&t| t > 0),
                "{label} ({word}): inner tokens contain zero (dropped char): {tokens:?}"
            );
        }
    }

    /// Synthesize and play all Japanese phoneme categories via Kokoro ONNX.
    ///
    /// Requires the Kokoro model to be downloaded (`~/.kotoba/models/kokoro/`).
    /// Skipped in CI — run locally with:
    ///
    /// ```bash
    /// cargo test -p kotoba pronunciation_audio_playback -- --ignored --nocapture
    /// ```
    ///
    /// Each word is synthesized to a temporary WAV and played via `afplay`.
    /// Listen for correct pronunciation of every category.
    #[tokio::test]
    #[ignore = "requires Kokoro model and audio device"]
    async fn pronunciation_audio_playback() {
        let cases: &[(&str, &str)] = &[
            // 清音 (seion)
            ("あいうえお", "vowels"),
            ("かきくけこ", "ka-row"),
            ("さしすせそ", "sa-row"),
            ("たちつてと", "ta-row"),
            ("なにぬねの", "na-row"),
            ("はひふへほ", "ha-row"),
            ("まみむめも", "ma-row"),
            ("やゆよ", "ya-row"),
            ("らりるれろ", "ra-row"),
            ("わをん", "wa-row"),
            // 濁音・半濁音 (dakuon)
            ("がぎぐげご", "ga-row"),
            ("ざじずぜぞ", "za-row"),
            ("だぢづでど", "da-row"),
            ("ばびぶべぼ", "ba-row"),
            ("ぱぴぷぺぽ", "pa-row"),
            // 拗音 (youon) — palatalized
            ("きゃきゅきょ", "kya-row"),
            ("しゃしゅしょ", "sha-row"),
            ("ちゃちゅちょ", "cha-row"),
            ("にゃにゅにょ", "nya-row"),
            ("ひゃひゅひょ", "hya-row"),
            ("みゃみゅみょ", "mya-row"),
            ("りゃりゅりょ", "rya-row"),
            ("ぎゃぎゅぎょ", "gya-row"),
            ("じゃじゅじょ", "ja-row"),
            ("びゃびゅびょ", "bya-row"),
            ("ぴゃぴゅぴょ", "pya-row"),
            // 促音 (geminate)
            ("いっぱい", "ippai"),
            ("ちょっと", "chotto"),
            ("がっこう", "gakkou"),
            // 長音 (long vowels)
            ("おはよう", "ohayou"),
            ("ありがとう", "arigatou"),
            ("先生", "sensei"),
            // 腭化漢字詞 (palatalized kanji — past regression)
            ("今日", "kyou"),
            ("東京", "toukyou"),
            ("京都", "kyouto"),
            ("勉強", "benkyou"),
            ("教室", "kyoushitsu"),
            ("牛乳", "gyuunyuu"),
            ("病院", "byouin"),
            ("旅行", "ryokou"),
            // 一般実用
            ("こんにちは", "konnichiwa"),
            ("食べる", "taberu"),
            ("日本語", "nihongo"),
            ("写真", "shashin"),
            ("電車", "densha"),
            ("成功", "seikou"),
            ("友達", "tomodachi"),
        ];

        let tmp = tempfile::tempdir().expect("tempdir");
        let voice = "jf_alpha";
        let speed = 1.0_f32;

        for (idx, &(word, label)) in cases.iter().enumerate() {
            let wav_path = tmp.path().join(format!("{idx:02}_{label}.wav"));
            eprintln!("  ▶ [{label}] {word}");

            let result = synthesize(word, "ja", voice, speed, &wav_path).await;
            match result {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("    SKIP (synthesis failed): {e}");
                    continue;
                }
            }

            // Play via afplay (macOS); ignore errors on non-macOS
            let _ = std::process::Command::new("afplay").arg(&wav_path).status();

            // Brief pause between words
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        eprintln!("Done. If any pronunciation sounds wrong, note the label.");
    }

    /// Verify palatalized consonants decompose to base+j (not base+ʲ).
    /// Token 52 is 'j' in Kokoro vocab; token 164 is 'ʲ' which the model
    /// cannot pronounce correctly.
    #[test]
    fn palatal_decomposition_uses_j_not_ipa_superscript() {
        // 今日 contains ᶄ (palatalized k) which must become kj, not kʲ
        let tokens = match tokenize("今日", "ja") {
            Ok(tokens) => tokens,
            Err(err) if is_missing_kokoro_python_dep(&err) => {
                eprintln!("skipping palatal_decomposition_uses_j_not_ipa_superscript: {err}");
                return;
            }
            Err(err) => panic!("tokenize kyou: {err}"),
        };
        let inner = &tokens[1..tokens.len() - 1];

        assert!(
            !inner.contains(&164),
            "tokens must not contain 164 (ʲ); got {inner:?}"
        );
        // 'k'=53 followed by 'j'=52
        assert!(
            inner.windows(2).any(|w| w[0] == 53 && w[1] == 52),
            "tokens should contain k(53)+j(52) sequence; got {inner:?}"
        );
    }
}
