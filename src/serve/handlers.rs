//! Route handlers for the OpenAI-compatible TTS API.

use std::{path::Path, sync::Arc};

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};

use super::models::{ApiError, SpeechRequest, VoiceEntry, VoiceListResponse};
use crate::tts::{KokoroBackend, TtsBackend, VitsBackend, VoicevoxBackend};

/// Shared server state passed to handlers via axum's `State` extractor.
#[derive(Clone)]
pub struct AppState {
    /// Application configuration snapshot taken at server start.
    pub config: Arc<crate::app_config::AppConfig>,
}

/// `GET /health` — returns a simple health-check response.
pub async fn health() -> impl IntoResponse { Json(serde_json::json!({"status": "ok"})) }

/// `GET /v1/voices` — list all available voices.
pub async fn list_voices() -> impl IntoResponse {
    let models_dir = crate::paths::models_dir();
    let mut voices: Vec<VoiceEntry> = Vec::new();

    // Kokoro voices (available when the model file exists)
    let kokoro_model = models_dir.join("kokoro").join("kokoro-v1.0.onnx");
    if kokoro_model.exists() {
        let kokoro_voices = [
            "af_heart",
            "af_bella",
            "af_nicole",
            "af_sarah",
            "af_sky",
            "am_adam",
            "am_michael",
            "jf_alpha",
            "jf_gongitsune",
            "jm_kumo",
        ];
        for name in &kokoro_voices {
            voices.push(VoiceEntry {
                id:      format!("kokoro:{name}"),
                name:    format!("Kokoro {name}"),
                backend: "kokoro".to_string(),
            });
        }
    }

    // VOICEVOX built-in speakers (subset)
    let voicevox_speakers = [
        ("1", "Shikoku Metan (normal)"),
        ("3", "Zundamon (normal)"),
        ("8", "Kasukabe Tsumugi"),
        ("13", "Aoyama Ryusei"),
    ];
    for (id, name) in &voicevox_speakers {
        voices.push(VoiceEntry {
            id:      format!("voicevox:{id}"),
            name:    (*name).to_string(),
            backend: "voicevox".to_string(),
        });
    }

    // RVC models
    let rvc_dir = models_dir.join("rvc");
    if let Ok(entries) = std::fs::read_dir(&rvc_dir) {
        let mut rvc_names: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        rvc_names.sort();
        for name in rvc_names {
            voices.push(VoiceEntry {
                id:      name.clone(),
                name:    format!("RVC {name}"),
                backend: "rvc".to_string(),
            });
        }
    }

    // VITS models
    if let Ok(entries) = std::fs::read_dir(&models_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let dir_name = entry.file_name().to_string_lossy().to_string();
                if dir_name == "kokoro" || dir_name == "rvc" {
                    continue;
                }
                voices.push(VoiceEntry {
                    id:      format!("vits:{dir_name}"),
                    name:    dir_name,
                    backend: "vits".to_string(),
                });
            }
        }
    }

    Json(VoiceListResponse { voices })
}

/// `POST /v1/audio/speech` — synthesize speech from text.
pub async fn speech(
    State(state): State<AppState>,
    Json(req): Json<SpeechRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiError>)> {
    if req.input.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError::bad_request("input text must not be empty")),
        ));
    }

    let format = req.response_format.as_deref().unwrap_or("wav");

    if format != "wav" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError::bad_request(format!(
                "unsupported response_format: {format} (only \"wav\" is supported)"
            ))),
        ));
    }

    #[allow(clippy::cast_possible_truncation)]
    let speed = req
        .speed
        .unwrap_or(state.config.voice.speed)
        .clamp(0.5, 2.0) as f32;

    // Resolve voice to (backend, speaker_id, rvc_model)
    let (backend_name, speaker_id, rvc_model) = resolve_voice(&req.voice, &state.config)?;

    // Synthesize to a temporary file
    let tmp_dir = tempfile::tempdir().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::internal(format!(
                "failed to create temp dir: {e}"
            ))),
        )
    })?;
    let tts_output = tmp_dir.path().join("speech.wav");

    let backend: Box<dyn TtsBackend> = match backend_name.as_str() {
        "kokoro" => Box::new(KokoroBackend::new(speaker_id.clone(), speed)),
        "voicevox" => {
            let url = state.config.voicevox.url.clone();
            Box::new(VoicevoxBackend::new(url, speaker_id.clone()))
        }
        "vits" => Box::new(VitsBackend::new(speaker_id.clone())),
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiError::bad_request(format!("unknown backend: {other}"))),
            ));
        }
    };

    backend
        .synthesize(&req.input, &tts_output)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::internal(format!("TTS synthesis failed: {e}"))),
            )
        })?;

    // Apply RVC voice conversion if needed
    let final_path = if let Some(model) = rvc_model {
        let rvc_output = tmp_dir.path().join("speech_rvc.wav");
        #[allow(clippy::cast_possible_truncation)]
        let index_influence = state.config.rvc.index_influence.clamp(0.0, 1.0) as f32;
        crate::rvc::convert(
            &tts_output,
            &model,
            state.config.rvc.pitch,
            &state.config.rvc.pitch_algo,
            index_influence,
            &rvc_output,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::internal(format!("RVC conversion failed: {e}"))),
            )
        })?;
        rvc_output
    } else {
        tts_output
    };

    // Read audio bytes and return with appropriate content type
    let audio_bytes = tokio::fs::read(&final_path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::internal(format!(
                "failed to read audio file: {e}"
            ))),
        )
    })?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "audio/wav".parse().expect("valid header value"),
    );

    Ok((headers, audio_bytes))
}

/// Resolved voice target: backend name, speaker identifier, and optional RVC
/// model to apply after synthesis.
type VoiceTarget = (String, String, Option<String>);

/// Resolve a voice string into a `VoiceTarget`.
///
/// Resolution order:
/// 1. Contains `:` -> parse as `backend:speaker_id`, TTS only
/// 2. Matches an RVC model directory -> TTS with default voice + RVC
/// 3. Try as bare Kokoro voice name -> `kokoro:{voice}`
/// 4. Otherwise -> 400 error
fn resolve_voice(
    voice: &str,
    config: &crate::app_config::AppConfig,
) -> Result<VoiceTarget, (StatusCode, Json<ApiError>)> {
    // 1. Explicit backend:speaker_id format
    if let Some((backend, speaker)) = voice.split_once(':') {
        return Ok((backend.to_string(), speaker.to_string(), None));
    }

    // 2. Check if it matches an RVC model directory
    let rvc_dir = crate::paths::models_dir().join("rvc").join(voice);
    if rvc_dir.is_dir() && has_pth_file(&rvc_dir) {
        // Use default Kokoro voice as TTS source, apply RVC on top
        let default_voice = extract_kokoro_voice(&config.voice.active);
        return Ok(("kokoro".to_string(), default_voice, Some(voice.to_string())));
    }

    // 3. Try as bare Kokoro voice name
    let kokoro_voices = [
        "af_heart",
        "af_bella",
        "af_nicole",
        "af_sarah",
        "af_sky",
        "am_adam",
        "am_michael",
        "jf_alpha",
        "jf_gongitsune",
        "jm_kumo",
    ];
    if kokoro_voices.contains(&voice) {
        return Ok(("kokoro".to_string(), voice.to_string(), None));
    }

    Err((
        StatusCode::BAD_REQUEST,
        Json(ApiError::bad_request(format!(
            "could not resolve voice: {voice} (use backend:id, an RVC model name, or a Kokoro \
             voice name)"
        ))),
    ))
}

/// Extract the Kokoro voice name from a `voice.active` config value.
/// Falls back to `jf_alpha` if the active voice is not a Kokoro voice.
fn extract_kokoro_voice(active: &str) -> String {
    active
        .strip_prefix("kokoro:")
        .map_or_else(|| "jf_alpha".to_string(), String::from)
}

/// Check whether a directory contains at least one `.pth` file.
fn has_pth_file(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pth"))
        })
}
