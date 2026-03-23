//! TTS audio generation and caching with configurable voice backend.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

use rodio::{Decoder, DeviceSinkBuilder, Player, Source};
use sha2::{Digest, Sha256};
use snafu::ResultExt;

use crate::{
    cli::PlayStyle,
    error::{self, Result},
    tts::{
        CosyvoiceBackend, KokoroBackend, TtsBackend, VitsBackend, VoicevoxBackend, VoicevoxProsody,
    },
};

/// Parsed voice configuration specifying backend and speaker/model identifier.
struct VoiceConfig {
    backend:    String,
    speaker_id: String,
}

#[derive(Debug, Clone, Copy)]
struct SegmentProsody {
    speed_scale:               f32,
    voicevox_pitch_scale:      f32,
    voicevox_intonation_scale: f32,
    rvc_pitch_delta:           i32,
}

impl SegmentProsody {
    const fn neutral() -> Self {
        Self {
            speed_scale:               1.0,
            voicevox_pitch_scale:      0.0,
            voicevox_intonation_scale: 1.0,
            rvc_pitch_delta:           0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct StyleProfile {
    dynamic_gain:          f32,
    base_speed_delta:      f32,
    base_pitch_delta:      f32,
    base_intonation_delta: f32,
    arc_strength:          f32,
    pause_scale:           f32,
    jitter_strength:       f32,
}

/// Parse a `backend:id` voice config string (e.g. `voicevox:3`,
/// `vits:model-name`).
fn parse_voice_config(raw: &str) -> VoiceConfig {
    match raw.split_once(':') {
        Some((backend, id)) => VoiceConfig {
            backend:    backend.to_string(),
            speaker_id: id.to_string(),
        },
        None => VoiceConfig {
            backend:    raw.to_string(),
            speaker_id: "1".to_string(),
        },
    }
}

/// Generate or return a cached WAV file for a word using the configured voice.
///
/// Reads the `voice.active` key from `config.toml` to determine which TTS
/// backend and speaker to use. Falls back to `voicevox:1` when no config is
/// set.
#[tracing::instrument]
#[allow(clippy::too_many_lines)]
pub async fn play_word(word: &str, enable: bool, style: PlayStyle) -> Result<PathBuf> {
    let cfg = crate::app_config::load();
    let raw_config = cfg.voice.active.clone();
    let config = parse_voice_config(&raw_config);
    #[allow(clippy::cast_possible_truncation)] // clamped to [0.5, 2.0]; no precision concern
    let base_speed = cfg.voice.speed.clamp(0.5, 2.0) as f32;
    let rvc_pitch = cfg.rvc.pitch;
    let rvc_pitch_algo = if cfg.rvc.pitch_algo.trim().is_empty() {
        "rmvpe".to_string()
    } else {
        cfg.rvc.pitch_algo.clone()
    };
    #[allow(clippy::cast_possible_truncation)] // clamped to [0.0, 1.0]; no precision concern
    let rvc_index_influence = cfg.rvc.index_influence.clamp(0.0, 1.0) as f32;

    let cache = crate::paths::audio_cache_dir();
    std::fs::create_dir_all(&cache).context(error::IoSnafu)?;

    let file = cache.join(build_cache_filename(
        word,
        &config.backend,
        &config.speaker_id,
    ));

    if file.exists() {
        match validate_generated_audio(&file) {
            Ok(()) => eprintln!("overwriting existing audio: {}", file.display()),
            Err(err) => eprintln!("existing audio invalid, regenerating: {err}"),
        }
        let _ = std::fs::remove_file(&file);
    }

    eprintln!("synthesizing: {word}...");

    let voice_segments = split_text_for_tts(word, MAX_SEGMENT_CHARS);
    if voice_segments.is_empty() {
        return error::VoicevoxSnafu {
            message: "input text is empty".to_string(),
        }
        .fail();
    }

    let voicevox_url = if config.backend == "voicevox" {
        let url = voicevox_base_url();
        check_voicevox_reachable(&url).await?;
        Some(url)
    } else {
        None
    };

    let kokoro_voice = if config.backend == "kokoro" {
        config.speaker_id.clone()
    } else {
        String::new()
    };
    let mut cosyvoice_cfg = cfg.cosyvoice.clone();
    if config.backend == "cosyvoice" {
        cosyvoice_cfg.url = crate::cosyvoice_runtime::base_url(&cosyvoice_cfg);
        crate::cosyvoice_runtime::ensure_running(&cosyvoice_cfg).await?;
    }
    let rvc_model_cfg = cfg.rvc.model.trim().to_string();
    let rvc_model = if rvc_model_cfg.is_empty() {
        None
    } else {
        Some(rvc_model_cfg)
    };
    let rvc_enabled = rvc_model.is_some();

    let backend_label = match config.backend.as_str() {
        "voicevox" => "voicevox",
        "vits" => "vits",
        "kokoro" => "kokoro",
        "cosyvoice" => "cosyvoice",
        other => {
            return error::VoicevoxSnafu {
                message: format!("unknown voice backend: {other}"),
            }
            .fail();
        }
    };

    let mut segment_files: Vec<PathBuf> = Vec::with_capacity(voice_segments.len());
    for (idx, segment) in voice_segments.iter().enumerate() {
        let segment_file = segment_output_path(&file, idx);
        let prosody = prosody_for_segment(segment, idx, voice_segments.len(), style);
        let segment_speed = (base_speed * prosody.speed_scale).clamp(0.5, 2.0);

        let synth_result = match config.backend.as_str() {
            "voicevox" => {
                let backend = VoicevoxBackend::new(
                    voicevox_url.clone().unwrap_or_else(voicevox_base_url),
                    config.speaker_id.clone(),
                )
                .with_prosody(voicevox_prosody(segment_speed, prosody));
                backend.synthesize(segment, &segment_file).await
            }
            "vits" => {
                let backend = VitsBackend::new(config.speaker_id.clone());
                backend.synthesize(segment, &segment_file).await
            }
            "kokoro" => {
                let backend = KokoroBackend::new(
                    kokoro_voice.clone(),
                    stabilized_kokoro_speed(segment_speed, base_speed, rvc_enabled),
                );
                backend.synthesize(segment, &segment_file).await
            }
            "cosyvoice" => {
                let backend = CosyvoiceBackend::new(
                    cosyvoice_cfg.url.clone(),
                    cosyvoice_cfg.mode.clone(),
                    config.speaker_id.clone(),
                    cosyvoice_cfg.prompt_text.clone(),
                    cosyvoice_cfg.prompt_wav.clone(),
                    cosyvoice_cfg.instruct_text.clone(),
                );
                backend.synthesize(segment, &segment_file).await
            }
            _ => unreachable!("backend checked before synthesis loop"),
        };

        if let Err(err) = synth_result {
            cleanup_temp_files(&segment_files);
            let _ = std::fs::remove_file(&segment_file);
            return Err(err);
        }

        if let Err(err) = validate_generated_audio(&segment_file) {
            cleanup_temp_files(&segment_files);
            let _ = std::fs::remove_file(&segment_file);
            return Err(err);
        }

        if let Some(model) = rvc_model.as_deref() {
            let segment_pitch =
                stabilized_rvc_pitch(rvc_pitch, prosody.rvc_pitch_delta, style, segment);
            eprintln!(
                "segment {}: converting with RVC model {model} (pitch={segment_pitch}, \
                 algo={rvc_pitch_algo}, index={rvc_index_influence})...",
                idx + 1
            );
            let pre_rvc_file =
                segment_file.with_extension(format!("pre_rvc_{}_{}.wav", std::process::id(), idx));
            if let Err(err) = std::fs::rename(&segment_file, &pre_rvc_file).context(error::IoSnafu)
            {
                cleanup_temp_files(&segment_files);
                let _ = std::fs::remove_file(&segment_file);
                let _ = std::fs::remove_file(&pre_rvc_file);
                return Err(err);
            }

            let convert_result = crate::rvc::convert(
                &pre_rvc_file,
                model,
                segment_pitch,
                &rvc_pitch_algo,
                rvc_index_influence,
                &segment_file,
            )
            .await;
            let _ = std::fs::remove_file(&pre_rvc_file);
            if let Err(err) = convert_result {
                cleanup_temp_files(&segment_files);
                let _ = std::fs::remove_file(&segment_file);
                return Err(err);
            }
            if let Err(err) = clean_tail_artifacts(&segment_file) {
                cleanup_temp_files(&segment_files);
                let _ = std::fs::remove_file(&segment_file);
                return Err(err);
            }
        }

        segment_files.push(segment_file);
    }

    let stitch_result = if segment_files.len() == 1 {
        std::fs::copy(&segment_files[0], &file)
            .context(error::IoSnafu)
            .map(|_| ())
    } else {
        stitch_segments(&segment_files, &voice_segments, style, &file)
    };
    if let Err(err) = stitch_result {
        cleanup_temp_files(&segment_files);
        let _ = std::fs::remove_file(&file);
        return Err(err);
    }
    cleanup_temp_files(&segment_files);

    if let Err(err) = validate_generated_audio(&file) {
        let _ = std::fs::remove_file(&file);
        return Err(err);
    }

    if enable {
        eprintln!("playing: {}", file.display());
        play_audio_now(&file)?;
    }

    eprintln!("cached ({backend_label}): {}", file.display());

    Ok(file)
}

const MAX_SEGMENT_CHARS: usize = 72;
const MIN_WAV_SIZE_BYTES: u64 = 44;
const SILENCE_THRESHOLD: f32 = 1.0e-6;
const TAIL_ACTIVITY_THRESHOLD: f32 = 2.0e-3;
const TAIL_ZERO_THRESHOLD: f32 = 1.0e-5;
const TAIL_GUARD_MS: usize = 25;
const TAIL_FADE_MS: usize = 20;
const TAIL_MIN_TRIM_MS: usize = 60;

fn split_text_for_tts(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(8);
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    let push_current = |chunks: &mut Vec<String>, current: &mut String| {
        let trimmed = current.trim();
        if !trimmed.is_empty() {
            chunks.push(trimmed.to_string());
        }
        current.clear();
    };

    for ch in text.trim().chars() {
        current.push(ch);
        current_len = current_len.saturating_add(1);

        let punctuation_split =
            matches!(ch, '。' | '！' | '？' | '!' | '?' | '\n' | '…' | ';' | '；');
        if current_len >= max_chars || (punctuation_split && current_len >= max_chars / 2) {
            push_current(&mut chunks, &mut current);
            current_len = 0;
        }
    }
    push_current(&mut chunks, &mut current);

    chunks
}

fn build_cache_filename(text: &str, backend: &str, speaker: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher.update(b"|");
    hasher.update(backend.as_bytes());
    hasher.update(b"|");
    hasher.update(speaker.as_bytes());
    let digest = hasher.finalize();

    let mut hash_hex = String::with_capacity(24);
    for b in digest.iter().take(12) {
        let _ = write!(&mut hash_hex, "{b:02x}");
    }

    format!(
        "tts_{}_{}_{}.wav",
        sanitize_component(backend, 18),
        sanitize_component(speaker, 18),
        hash_hex
    )
}

fn sanitize_component(value: &str, max_len: usize) -> String {
    let mut out = String::with_capacity(max_len);
    for ch in value.chars() {
        if out.len() >= max_len {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.trim_matches('_').is_empty() {
        "default".to_string()
    } else {
        out
    }
}

const fn style_profile(style: PlayStyle) -> StyleProfile {
    match style {
        PlayStyle::Neutral => StyleProfile {
            dynamic_gain:          0.75,
            base_speed_delta:      0.00,
            base_pitch_delta:      0.00,
            base_intonation_delta: 0.00,
            arc_strength:          0.45,
            pause_scale:           1.00,
            jitter_strength:       0.00,
        },
        PlayStyle::Character => StyleProfile {
            dynamic_gain:          1.00,
            base_speed_delta:      0.01,
            base_pitch_delta:      0.01,
            base_intonation_delta: 0.05,
            arc_strength:          0.85,
            pause_scale:           1.00,
            jitter_strength:       0.015,
        },
        PlayStyle::Dramatic => StyleProfile {
            dynamic_gain:          1.22,
            base_speed_delta:      0.03,
            base_pitch_delta:      0.03,
            base_intonation_delta: 0.10,
            arc_strength:          1.20,
            pause_scale:           1.10,
            jitter_strength:       0.025,
        },
        PlayStyle::Soft => StyleProfile {
            dynamic_gain:          0.85,
            base_speed_delta:      -0.04,
            base_pitch_delta:      -0.01,
            base_intonation_delta: 0.03,
            arc_strength:          0.55,
            pause_scale:           1.22,
            jitter_strength:       0.010,
        },
        PlayStyle::Energetic => StyleProfile {
            dynamic_gain:          1.15,
            base_speed_delta:      0.07,
            base_pitch_delta:      0.04,
            base_intonation_delta: 0.07,
            arc_strength:          0.95,
            pause_scale:           0.78,
            jitter_strength:       0.020,
        },
    }
}

fn deterministic_jitter(text: &str, segment_index: usize) -> f32 {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher.update(segment_index.to_le_bytes());
    let digest = hasher.finalize();
    let raw = f32::from(digest[0]) / 255.0;
    raw * 2.0 - 1.0
}

fn prosody_for_segment(
    segment: &str,
    segment_index: usize,
    total_segments: usize,
    style: PlayStyle,
) -> SegmentProsody {
    let mut p = SegmentProsody::neutral();
    let text = segment.trim();
    let char_count = text.chars().count();
    let profile = style_profile(style);

    // Always keep a slight expressive baseline (including short utterances)
    // so speech does not sound fully flat.
    p.speed_scale += 0.02;
    p.voicevox_intonation_scale += 0.06;
    if char_count <= 14 {
        p.voicevox_pitch_scale += 0.02;
        p.voicevox_intonation_scale += 0.04;
    }

    let exclamation_count = text.matches(['!', '！']).count();
    if exclamation_count > 0 {
        p.speed_scale += 0.12;
        p.voicevox_pitch_scale += 0.10;
        p.voicevox_intonation_scale += 0.24;
        p.rvc_pitch_delta += 2;
    }

    let question_count = text.matches(['?', '？']).count();
    if question_count > 0 {
        p.speed_scale += 0.06;
        p.voicevox_pitch_scale += 0.05;
        p.voicevox_intonation_scale += 0.16;
        p.rvc_pitch_delta += 1;
    }

    if text.contains("...") || text.contains('…') {
        p.speed_scale -= 0.12;
        p.voicevox_pitch_scale -= 0.05;
        p.voicevox_intonation_scale -= 0.10;
        p.rvc_pitch_delta -= 1;
    }

    if text.contains("最高")
        || text.contains("すご")
        || text.contains("やば")
        || text.contains("好き")
    {
        p.speed_scale += 0.06;
        p.voicevox_pitch_scale += 0.04;
        p.voicevox_intonation_scale += 0.08;
        p.rvc_pitch_delta += 1;
    }

    if text.contains("ごめん")
        || text.contains("悲")
        || text.contains("寂")
        || text.contains("つら")
        || text.contains("辛")
    {
        p.speed_scale -= 0.05;
        p.voicevox_pitch_scale -= 0.05;
        p.voicevox_intonation_scale -= 0.06;
        p.rvc_pitch_delta -= 1;
    }

    if text.ends_with("かな")
        || text.ends_with("かも")
        || text.ends_with("よね")
        || text.ends_with('ね')
    {
        p.voicevox_pitch_scale += 0.03;
        p.voicevox_intonation_scale += 0.05;
    }

    if text.ends_with('。') {
        p.voicevox_intonation_scale -= 0.04;
    }

    if total_segments > 1 {
        #[allow(clippy::cast_precision_loss)] // segment counts are small (< 100)
        let denom = (total_segments.saturating_sub(1)).max(1) as f32;
        #[allow(clippy::cast_precision_loss)]
        let progress = (segment_index as f32 / denom).clamp(0.0, 1.0);
        // Middle segments carry more energy; ending cadence slightly settles.
        let center_weight = (progress - 0.5).abs().mul_add(-2.0, 1.0).clamp(0.0, 1.0);
        p.speed_scale += 0.04 * center_weight * profile.arc_strength;
        p.voicevox_pitch_scale += 0.03 * center_weight * profile.arc_strength;
        p.voicevox_intonation_scale += 0.09 * center_weight * profile.arc_strength;

        if segment_index == 0 {
            p.speed_scale -= 0.02;
        }
        if segment_index + 1 == total_segments && exclamation_count == 0 {
            p.speed_scale -= 0.03;
            p.voicevox_intonation_scale -= 0.03;
        }
    }

    let jitter = deterministic_jitter(text, segment_index) * profile.jitter_strength;
    p.voicevox_pitch_scale += jitter;
    p.voicevox_intonation_scale += jitter.abs() * 0.6;

    // Style profile remaps dynamic range and baseline toward chosen tone.
    p.speed_scale =
        (p.speed_scale - 1.0).mul_add(profile.dynamic_gain, 1.0) + profile.base_speed_delta;
    p.voicevox_pitch_scale = p
        .voicevox_pitch_scale
        .mul_add(profile.dynamic_gain, profile.base_pitch_delta);
    p.voicevox_intonation_scale = (p.voicevox_intonation_scale - 1.0)
        .mul_add(profile.dynamic_gain, 1.0)
        + profile.base_intonation_delta;

    p.speed_scale = p.speed_scale.clamp(0.70, 1.35);
    p.voicevox_pitch_scale = p.voicevox_pitch_scale.clamp(-0.25, 0.25);
    p.voicevox_intonation_scale = p.voicevox_intonation_scale.clamp(0.70, 1.40);
    p.rvc_pitch_delta = p.rvc_pitch_delta.clamp(-3, 4);
    p
}

const fn voicevox_prosody(segment_speed: f32, p: SegmentProsody) -> VoicevoxProsody {
    VoicevoxProsody {
        speed_scale:      segment_speed,
        pitch_scale:      p.voicevox_pitch_scale,
        intonation_scale: p.voicevox_intonation_scale,
    }
}

fn segment_output_path(final_output: &Path, index: usize) -> PathBuf {
    let stem = final_output
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("tts");
    final_output.with_file_name(format!(
        "{stem}_seg_{:03}_{}.wav",
        index,
        std::process::id()
    ))
}

fn cleanup_temp_files(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

fn stitch_segments(
    segment_files: &[PathBuf],
    segment_texts: &[String],
    style: PlayStyle,
    output: &Path,
) -> Result<()> {
    if segment_files.is_empty() {
        return error::AudioInvalidSnafu {
            path:    output.display().to_string(),
            message: "no segment audio to stitch".to_string(),
        }
        .fail();
    }
    if segment_files.len() != segment_texts.len() {
        return error::AudioInvalidSnafu {
            path:    output.display().to_string(),
            message: "segment audio/text count mismatch".to_string(),
        }
        .fail();
    }

    let mut merged: Vec<f32> = Vec::new();
    let mut sample_rate = 0u32;
    let mut channels = 0usize;

    for (idx, segment) in segment_files.iter().enumerate() {
        let file = std::fs::File::open(segment).context(error::IoSnafu)?;
        let decoder = Decoder::try_from(file).map_err(|source| {
            error::AudioInvalidSnafu {
                path:    segment.display().to_string(),
                message: format!("decode failed while stitching: {source}"),
            }
            .build()
        })?;

        let seg_channels = usize::from(decoder.channels().get());
        let seg_rate = decoder.sample_rate().get();
        let seg_samples: Vec<f32> = decoder.collect();

        if seg_samples.is_empty() {
            return error::AudioInvalidSnafu {
                path:    segment.display().to_string(),
                message: "segment contains no samples".to_string(),
            }
            .fail();
        }

        if idx == 0 {
            sample_rate = seg_rate;
            channels = seg_channels;
        } else if seg_rate != sample_rate || seg_channels != channels {
            return error::AudioInvalidSnafu {
                path:    segment.display().to_string(),
                message: format!(
                    "segment format mismatch: expected {sample_rate}Hz/{channels}ch, got \
                     {seg_rate}Hz/{seg_channels}ch"
                ),
            }
            .fail();
        }

        if idx > 0 {
            let gap_ms = natural_pause_ms(&segment_texts[idx - 1], style);
            let gap_frames = ((sample_rate as usize) * gap_ms) / 1000;
            merged.extend(std::iter::repeat_n(0.0_f32, gap_frames * channels));
        }
        merged.extend(seg_samples);
    }

    let channels_u16 = u16::try_from(channels).expect("audio channel count fits in u16");
    write_pcm16_wav(output, &merged, sample_rate, channels_u16)
}

fn natural_pause_ms(segment: &str, style: PlayStyle) -> usize {
    let text = segment.trim();
    let base = if text.contains("...") || text.contains('…') {
        170
    } else if text.ends_with('。') {
        110
    } else if text.ends_with('！') || text.ends_with('!') {
        75
    } else if text.ends_with('？') || text.ends_with('?') {
        95
    } else if text.ends_with('、') || text.ends_with(',') {
        65
    } else {
        45
    };
    #[allow(clippy::cast_precision_loss)] // base is a small integer (< 260)
    let scaled = (base as f32 * style_profile(style).pause_scale).round();
    // pause_scale and base are both positive, so the result is non-negative
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let scaled = (scaled as usize).clamp(30, 260);
    scaled
}

fn stabilized_rvc_pitch(base_pitch: i32, raw_delta: i32, style: PlayStyle, text: &str) -> i32 {
    // Prioritize intelligibility over expressiveness.
    // For typical styles, keep pitch stable; only allow tiny modulation for
    // highly expressive styles when base pitch is already near neutral.
    let style_delta = match style {
        PlayStyle::Dramatic if base_pitch.abs() <= 2 => raw_delta.clamp(-1, 1),
        PlayStyle::Energetic if base_pitch.abs() <= 1 => raw_delta.clamp(-1, 1),
        _ => 0,
    };
    let mut stabilized = (base_pitch + style_delta).clamp(-6, 6);
    if japanese_char_ratio(text) >= 0.45 {
        // Japanese line delivery is highly sensitive to aggressive formant/pitch
        // shifts in RVC, especially for short utterances. Prioritize pronunciation
        // fidelity by neutralizing base pitch on short lines.
        let jp_chars = japanese_char_count(text);
        let short_japanese_line = jp_chars <= 4;
        if short_japanese_line {
            return 0;
        }

        stabilized = stabilized.clamp(-1, 1);
        let limit = match style {
            PlayStyle::Dramatic | PlayStyle::Energetic => 2,
            _ => 1,
        };
        stabilized = stabilized.clamp(-limit, limit);
    }
    stabilized
}

fn japanese_char_ratio(text: &str) -> f32 {
    let mut total = 0usize;
    let mut jp = 0usize;
    for ch in text.chars() {
        if ch.is_whitespace() || ch.is_ascii_punctuation() {
            continue;
        }
        total = total.saturating_add(1);
        if is_japanese_char(ch) {
            jp = jp.saturating_add(1);
        }
    }

    if total == 0 {
        0.0
    } else {
        // Character counts in a single text segment are small; precision loss is
        // negligible.
        #[allow(clippy::cast_precision_loss)]
        {
            jp as f32 / total as f32
        }
    }
}

fn japanese_char_count(text: &str) -> usize {
    text.chars().filter(|&ch| is_japanese_char(ch)).count()
}

const fn is_japanese_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3040}'..='\u{30FF}'
            | '\u{31F0}'..='\u{31FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{3005}'
    )
}

fn stabilized_kokoro_speed(segment_speed: f32, base_speed: f32, rvc_enabled: bool) -> f32 {
    if !rvc_enabled {
        return segment_speed;
    }

    // RVC conversion gets unstable with large speed swings. Keep local variation
    // narrow around user-configured base speed.
    let low = (base_speed - 0.08).max(0.75);
    let high = (base_speed + 0.06).min(1.35);
    segment_speed.clamp(low, high)
}

fn validate_generated_audio(path: &Path) -> Result<()> {
    let meta = std::fs::metadata(path).context(error::IoSnafu)?;
    if meta.len() <= MIN_WAV_SIZE_BYTES {
        return error::AudioInvalidSnafu {
            path:    path.display().to_string(),
            message: "file too small to be a valid WAV".to_string(),
        }
        .fail();
    }

    let file = std::fs::File::open(path).context(error::IoSnafu)?;
    let decoder = Decoder::try_from(file).map_err(|source| {
        error::AudioInvalidSnafu {
            path:    path.display().to_string(),
            message: format!("decode failed: {source}"),
        }
        .build()
    })?;

    let mut sample_count = 0usize;
    let mut non_silent = false;

    for sample in decoder {
        sample_count = sample_count.saturating_add(1);
        if sample.abs() > SILENCE_THRESHOLD {
            non_silent = true;
            break;
        }
    }

    if sample_count == 0 {
        return error::AudioInvalidSnafu {
            path:    path.display().to_string(),
            message: "decoded audio contains no samples".to_string(),
        }
        .fail();
    }

    if !non_silent {
        return error::AudioInvalidSnafu {
            path:    path.display().to_string(),
            message: "decoded audio is silent".to_string(),
        }
        .fail();
    }

    Ok(())
}

fn play_audio_now(path: &Path) -> Result<()> {
    let mut sink = DeviceSinkBuilder::open_default_sink().map_err(|source| {
        error::AudioPlaybackSnafu {
            path:    path.display().to_string(),
            message: source.to_string(),
        }
        .build()
    })?;
    sink.log_on_drop(false);
    let player = Player::connect_new(sink.mixer());
    let file = std::fs::File::open(path).context(error::IoSnafu)?;
    let audio = Decoder::try_from(file).map_err(|source| {
        error::AudioPlaybackSnafu {
            path:    path.display().to_string(),
            message: format!("decode failed: {source}"),
        }
        .build()
    })?;

    player.append(audio);
    player.sleep_until_end();
    Ok(())
}

/// Remove trailing low-energy artifacts and apply short fade-out to avoid
/// audible end clicks in converted output.
fn clean_tail_artifacts(path: &Path) -> Result<()> {
    let file = std::fs::File::open(path).context(error::IoSnafu)?;
    let decoder = Decoder::try_from(file).map_err(|source| {
        error::AudioInvalidSnafu {
            path:    path.display().to_string(),
            message: format!("decode failed during tail cleanup: {source}"),
        }
        .build()
    })?;

    let channels = usize::from(decoder.channels().get());
    let sample_rate = decoder.sample_rate().get();
    let mut samples: Vec<f32> = decoder.collect();

    if channels == 0 || samples.is_empty() {
        return Ok(());
    }

    if !trim_and_fade_tail(&mut samples, channels, sample_rate) {
        return Ok(());
    }

    let channels_u16 = u16::try_from(channels).expect("audio channel count fits in u16");
    write_pcm16_wav(path, &samples, sample_rate, channels_u16)
}

/// Trim trailing low-energy residuals and fade out the last short region.
///
/// Returns `true` when the sample buffer was modified.
fn trim_and_fade_tail(samples: &mut Vec<f32>, channels: usize, sample_rate: u32) -> bool {
    if channels == 0 || samples.is_empty() {
        return false;
    }

    let frame_count = samples.len() / channels;
    if frame_count == 0 {
        return false;
    }

    let mut changed = false;
    let last_active_frame = (0..frame_count).rev().find(|&frame| {
        let start = frame * channels;
        let end = start + channels;
        samples[start..end]
            .iter()
            .any(|sample| sample.abs() > TAIL_ACTIVITY_THRESHOLD)
    });

    let Some(last_active_frame) = last_active_frame else {
        samples.fill(0.0);
        return true;
    };

    let guard_frames = ((sample_rate as usize) * TAIL_GUARD_MS) / 1000;
    let keep_frames = (last_active_frame + 1 + guard_frames).min(frame_count);
    let trailing_frames = frame_count.saturating_sub(keep_frames);
    let min_trim_frames = ((sample_rate as usize) * TAIL_MIN_TRIM_MS) / 1000;
    if trailing_frames >= min_trim_frames {
        samples.truncate(keep_frames * channels);
        changed = true;
    }

    let frame_count = samples.len() / channels;
    let fade_frames = (((sample_rate as usize) * TAIL_FADE_MS) / 1000).min(frame_count);
    if fade_frames > 0 {
        let fade_start = frame_count.saturating_sub(fade_frames);
        #[allow(clippy::cast_precision_loss)] // fade frame counts are small (< 1000)
        let denom = (fade_frames.saturating_sub(1)).max(1) as f32;
        for i in 0..fade_frames {
            #[allow(clippy::cast_precision_loss)]
            let gain = (fade_frames.saturating_sub(1 + i)) as f32 / denom;
            let frame = fade_start + i;
            let start = frame * channels;
            let end = start + channels;
            for sample in &mut samples[start..end] {
                *sample *= gain;
            }
        }
        changed = true;
    }

    for sample in samples.iter_mut() {
        if sample.abs() < TAIL_ZERO_THRESHOLD {
            *sample = 0.0;
        }
    }

    let last_frame_start = samples.len().saturating_sub(channels);
    for sample in &mut samples[last_frame_start..] {
        if *sample != 0.0 {
            *sample = 0.0;
            changed = true;
        }
    }

    changed
}

fn write_pcm16_wav(path: &Path, samples: &[f32], sample_rate: u32, channels: u16) -> Result<()> {
    use std::io::Write;

    let bits_per_sample: u16 = 16;
    let block_align: u16 = channels * (bits_per_sample / 8);
    let byte_rate: u32 = sample_rate * u32::from(block_align);
    let data_size: u32 = (samples.len() * std::mem::size_of::<i16>())
        .try_into()
        .map_err(|_| {
            error::AudioInvalidSnafu {
                path:    path.display().to_string(),
                message: "audio is too large to write as PCM16 WAV".to_string(),
            }
            .build()
        })?;
    let riff_size: u32 = 36 + data_size;

    let mut file = std::fs::File::create(path).context(error::IoSnafu)?;
    file.write_all(b"RIFF").context(error::IoSnafu)?;
    file.write_all(&riff_size.to_le_bytes())
        .context(error::IoSnafu)?;
    file.write_all(b"WAVE").context(error::IoSnafu)?;
    file.write_all(b"fmt ").context(error::IoSnafu)?;
    file.write_all(&16u32.to_le_bytes())
        .context(error::IoSnafu)?;
    file.write_all(&1u16.to_le_bytes())
        .context(error::IoSnafu)?;
    file.write_all(&channels.to_le_bytes())
        .context(error::IoSnafu)?;
    file.write_all(&sample_rate.to_le_bytes())
        .context(error::IoSnafu)?;
    file.write_all(&byte_rate.to_le_bytes())
        .context(error::IoSnafu)?;
    file.write_all(&block_align.to_le_bytes())
        .context(error::IoSnafu)?;
    file.write_all(&bits_per_sample.to_le_bytes())
        .context(error::IoSnafu)?;
    file.write_all(b"data").context(error::IoSnafu)?;
    file.write_all(&data_size.to_le_bytes())
        .context(error::IoSnafu)?;

    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        #[allow(clippy::cast_possible_truncation)] // intentional f32 -> i16 PCM quantization
        let pcm = (clamped * f32::from(i16::MAX)).round() as i16;
        file.write_all(&pcm.to_le_bytes()).context(error::IoSnafu)?;
    }

    Ok(())
}

/// Resolve the VOICEVOX base URL: env var overrides config.
fn voicevox_base_url() -> String {
    std::env::var("VOICEVOX_URL").unwrap_or_else(|_| crate::app_config::load().voicevox.url.clone())
}

/// Check that the VOICEVOX engine is reachable at the given URL.
async fn check_voicevox_reachable(base_url: &str) -> Result<()> {
    crate::http::client()
        .get(format!("{base_url}/version"))
        .send()
        .await
        .map_err(|_| {
            error::VoicevoxNotRunningSnafu {
                url: base_url.to_string(),
            }
            .build()
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write};

    use super::*;

    #[test]
    fn parse_voice_config_with_colon() {
        let config = parse_voice_config("voicevox:3");
        assert_eq!(config.backend, "voicevox");
        assert_eq!(config.speaker_id, "3");
    }

    #[test]
    fn parse_voice_config_without_colon() {
        let config = parse_voice_config("voicevox");
        assert_eq!(config.backend, "voicevox");
        assert_eq!(config.speaker_id, "1");
    }

    #[test]
    fn parse_voice_config_vits() {
        let config = parse_voice_config("vits:my-model");
        assert_eq!(config.backend, "vits");
        assert_eq!(config.speaker_id, "my-model");
    }

    #[test]
    fn parse_voice_config_kokoro() {
        let config = parse_voice_config("kokoro:af_heart");
        assert_eq!(config.backend, "kokoro");
        assert_eq!(config.speaker_id, "af_heart");
    }

    #[test]
    fn parse_voice_config_kokoro_voice_only() {
        let config = parse_voice_config("kokoro:jf_alpha");
        assert_eq!(config.backend, "kokoro");
        assert_eq!(config.speaker_id, "jf_alpha");
    }

    #[test]
    fn parse_voice_config_cosyvoice() {
        let config = parse_voice_config("cosyvoice:default");
        assert_eq!(config.backend, "cosyvoice");
        assert_eq!(config.speaker_id, "default");
    }

    #[test]
    fn parse_voice_config_empty_string() {
        let config = parse_voice_config("");
        assert_eq!(config.backend, "");
        assert_eq!(config.speaker_id, "1");
    }

    #[test]
    fn parse_voice_config_multiple_colons() {
        let config = parse_voice_config("vits:model:extra");
        assert_eq!(config.backend, "vits");
        assert_eq!(config.speaker_id, "model:extra");
    }

    #[test]
    fn validate_generated_audio_rejects_silent_wav() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wav = dir.path().join("silent.wav");
        write_pcm16_wav(&wav, &[0, 0, 0, 0, 0, 0, 0, 0]).expect("write wav");

        let err = validate_generated_audio(&wav).expect_err("expected silent wav to be rejected");
        let message = err.to_string();
        assert!(
            message.contains("silent"),
            "unexpected error message: {message}"
        );
    }

    #[test]
    fn validate_generated_audio_accepts_non_silent_wav() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wav = dir.path().join("voiced.wav");
        write_pcm16_wav(&wav, &[0, 42, -42, 0, 10, -10]).expect("write wav");

        validate_generated_audio(&wav).expect("expected voiced wav to be accepted");
    }

    #[test]
    fn trim_and_fade_tail_reduces_trailing_noise() {
        let mut samples = vec![0.0_f32; 4800];
        samples.extend(std::iter::repeat_n(0.25_f32, 4800));
        // Simulate RVC tail residual noise near the end.
        samples.extend(std::iter::repeat_n(0.001_f32, 4800));

        let original_len = samples.len();
        let changed = trim_and_fade_tail(&mut samples, 1, 48_000);

        assert!(changed, "tail processing should report modifications");
        assert!(
            samples.len() < original_len,
            "tail processing should trim trailing residual noise"
        );
        assert!(samples.last().copied().unwrap_or_default().abs() < 1.0e-6);
    }

    #[test]
    fn split_text_for_tts_splits_long_input_into_bounded_chunks() {
        let text = "今日は天気がいいですね。明日も晴れるといいですね！\
                    長い文章をそのまま一発で音声化せず、区切って処理したいです。";
        let chunks = split_text_for_tts(text, 16);

        assert!(
            chunks.len() > 1,
            "long input should be split into multiple chunks"
        );
        assert!(
            chunks.iter().all(|chunk| chunk.chars().count() <= 16),
            "every chunk should be bounded by max chars: {chunks:?}"
        );

        let reconstructed = chunks.join("");
        let normalized_original: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
        let normalized_reconstructed: String = reconstructed
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect();
        assert_eq!(
            normalized_reconstructed, normalized_original,
            "split should preserve textual content"
        );
    }

    #[test]
    fn cache_filename_is_short_and_stable_for_long_text() {
        let long_text = "あ".repeat(400);
        let name1 = build_cache_filename(&long_text, "kokoro", "jf_alpha");
        let name2 = build_cache_filename(&long_text, "kokoro", "jf_alpha");

        assert_eq!(name1, name2, "filename hash should be stable");
        assert!(
            name1.len() < 200,
            "cache filename must stay below common filesystem limits: {name1}"
        );
        assert!(
            std::path::Path::new(&name1)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
        );
        assert!(
            !name1.contains(&long_text),
            "long raw text should not be embedded directly in file name"
        );
    }

    #[test]
    fn prosody_for_exclamation_is_more_energetic() {
        let neutral = prosody_for_segment("今日はいい天気です。", 0, 1, PlayStyle::Character);
        let excited = prosody_for_segment("今日は最高だ！！", 0, 1, PlayStyle::Character);

        assert!(excited.speed_scale > neutral.speed_scale);
        assert!(excited.voicevox_pitch_scale > neutral.voicevox_pitch_scale);
        assert!(excited.voicevox_intonation_scale > neutral.voicevox_intonation_scale);
    }

    #[test]
    fn prosody_for_question_raises_intonation() {
        let neutral = prosody_for_segment("明日会える。", 0, 1, PlayStyle::Character);
        let question = prosody_for_segment("明日会える？", 0, 1, PlayStyle::Character);

        assert!(
            question.voicevox_intonation_scale > neutral.voicevox_intonation_scale,
            "question tone should have higher intonation"
        );
        assert!(
            question.voicevox_pitch_scale >= neutral.voicevox_pitch_scale,
            "question tone should not lower pitch"
        );
    }

    #[test]
    fn short_text_still_gets_expressive_baseline() {
        let short = prosody_for_segment("こんばんは。", 0, 1, PlayStyle::Character);
        assert!(
            short.voicevox_intonation_scale > 1.0,
            "short text should not stay completely flat"
        );
    }

    #[test]
    fn multi_segment_arc_adds_middle_energy() {
        let first = prosody_for_segment("今日はね。", 0, 3, PlayStyle::Character);
        let middle = prosody_for_segment("本当にすごいことがあって！", 1, 3, PlayStyle::Character);
        let last = prosody_for_segment("また後で話すね。", 2, 3, PlayStyle::Character);

        assert!(
            middle.voicevox_intonation_scale >= first.voicevox_intonation_scale,
            "middle segment should carry stronger performance energy than opener"
        );
        assert!(
            middle.voicevox_intonation_scale >= last.voicevox_intonation_scale,
            "middle segment should carry stronger performance energy than closer"
        );
    }

    #[test]
    fn pause_after_ellipsis_is_longer_than_after_exclamation() {
        let ellipsis = natural_pause_ms("えっと…", PlayStyle::Character);
        let exclamation = natural_pause_ms("すごい！", PlayStyle::Character);
        assert!(
            ellipsis > exclamation,
            "ellipsis pause should be longer than exclamation pause"
        );
    }

    #[test]
    fn dramatic_style_is_more_expressive_than_neutral() {
        let neutral = prosody_for_segment("本当に嬉しい！", 0, 1, PlayStyle::Neutral);
        let dramatic = prosody_for_segment("本当に嬉しい！", 0, 1, PlayStyle::Dramatic);
        assert!(dramatic.voicevox_intonation_scale > neutral.voicevox_intonation_scale);
        assert!(dramatic.voicevox_pitch_scale >= neutral.voicevox_pitch_scale);
    }

    #[test]
    fn soft_style_uses_longer_pause_than_energetic() {
        let soft = natural_pause_ms("またね。", PlayStyle::Soft);
        let energetic = natural_pause_ms("またね。", PlayStyle::Energetic);
        assert!(soft > energetic, "soft style should keep longer pause");
    }

    #[test]
    fn character_style_keeps_rvc_pitch_stable() {
        let pitch = stabilized_rvc_pitch(5, 3, PlayStyle::Character, "今日は本当に嬉しい！");
        assert_eq!(pitch, 1, "japanese text should clamp toward stable pitch");
    }

    #[test]
    fn dramatic_style_only_allows_small_rvc_shift_when_base_is_low() {
        let low_base = stabilized_rvc_pitch(1, 3, PlayStyle::Dramatic, "今日はどうかな？");
        let high_base = stabilized_rvc_pitch(5, 3, PlayStyle::Dramatic, "今日はどうかな？");
        assert_eq!(low_base, 1, "dramatic style allows only tiny shift");
        assert_eq!(
            high_base, 1,
            "high base pitch should be clamped for japanese"
        );
    }

    #[test]
    fn short_japanese_lines_force_neutral_rvc_pitch() {
        let pitch = stabilized_rvc_pitch(5, 3, PlayStyle::Character, "今日");
        assert_eq!(pitch, 0, "short japanese lines should force neutral pitch");
    }

    #[test]
    fn non_japanese_text_keeps_user_pitch_setting() {
        let pitch = stabilized_rvc_pitch(5, 3, PlayStyle::Character, "This is so awesome!");
        assert_eq!(
            pitch, 5,
            "non-japanese lines should keep configured pitch behavior"
        );
    }

    #[test]
    fn rvc_speed_is_stabilized_around_base() {
        let base_speed = 1.24;
        let stabilized = stabilized_kokoro_speed(1.70, base_speed, true);
        assert!(stabilized <= 1.30, "speed should be capped in RVC mode");
        let unchanged = stabilized_kokoro_speed(1.10, base_speed, false);
        assert!(
            (unchanged - 1.10).abs() < 1.0e-6,
            "non-RVC mode should keep original speed"
        );
    }

    fn write_pcm16_wav(path: &std::path::Path, samples: &[i16]) -> std::io::Result<()> {
        let mut file = File::create(path)?;

        let num_channels: u16 = 1;
        let sample_rate: u32 = 24_000;
        let bits_per_sample: u16 = 16;
        let block_align: u16 = num_channels * (bits_per_sample / 8);
        let byte_rate: u32 = sample_rate * u32::from(block_align);
        let data_size: u32 = std::mem::size_of_val(samples)
            .try_into()
            .expect("sample data too large");
        let riff_size: u32 = 36 + data_size;

        file.write_all(b"RIFF")?;
        file.write_all(&riff_size.to_le_bytes())?;
        file.write_all(b"WAVE")?;

        file.write_all(b"fmt ")?;
        file.write_all(&16u32.to_le_bytes())?;
        file.write_all(&1u16.to_le_bytes())?;
        file.write_all(&num_channels.to_le_bytes())?;
        file.write_all(&sample_rate.to_le_bytes())?;
        file.write_all(&byte_rate.to_le_bytes())?;
        file.write_all(&block_align.to_le_bytes())?;
        file.write_all(&bits_per_sample.to_le_bytes())?;

        file.write_all(b"data")?;
        file.write_all(&data_size.to_le_bytes())?;

        for sample in samples {
            file.write_all(&sample.to_le_bytes())?;
        }

        Ok(())
    }
}
