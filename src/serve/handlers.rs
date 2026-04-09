//! Route handlers for the OpenAI-compatible TTS API.

use std::{path::Path, sync::Arc, time::Duration};

use async_trait::async_trait;
use axum::{
    Json,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};

use super::models::{
    ApiError, SpeechRequest, VoiceEntry, VoiceListResponse, WsClientMessage, WsResponse,
};
use crate::tts::{KokoroBackend, TtsBackend, VitsBackend, VoicevoxBackend};

/// Built-in Kokoro v1.0 voice catalog grouped by language.
///
/// Kokoro v1.0 ships voices for many languages in a single `voices-v1.0.bin`
/// file. The first character of each voice name encodes the language family:
/// `a` = American English, `b` = British English, `j` = Japanese,
/// `z` = Mandarin Chinese, etc.
const KOKORO_VOICES: &[&str] = &[
    // American English
    "af_heart",
    "af_bella",
    "af_nicole",
    "af_sarah",
    "af_sky",
    "am_adam",
    "am_michael",
    // Japanese
    "jf_alpha",
    "jf_gongitsune",
    "jm_kumo",
    // Mandarin Chinese
    "zf_xiaobei",
    "zf_xiaoni",
    "zf_xiaoxiao",
    "zf_xiaoyi",
    "zm_yunjian",
    "zm_yunxi",
    "zm_yunxia",
    "zm_yunyang",
];

/// Factory for creating TTS backends from a resolved voice target.
#[async_trait]
pub trait BackendFactory: Send + Sync {
    /// Create a TTS backend for the given backend name, speaker, and speed.
    fn create(
        &self,
        backend: &str,
        speaker: &str,
        speed: f32,
        config: &crate::app_config::AppConfig,
    ) -> Result<Box<dyn TtsBackend>, String>;
}

/// Default factory that instantiates real TTS backends (Kokoro, VOICEVOX,
/// VITS).
pub struct DefaultBackendFactory;

impl BackendFactory for DefaultBackendFactory {
    fn create(
        &self,
        backend: &str,
        speaker: &str,
        speed: f32,
        config: &crate::app_config::AppConfig,
    ) -> Result<Box<dyn TtsBackend>, String> {
        match backend {
            "kokoro" => Ok(Box::new(KokoroBackend::new(speaker.to_string(), speed))),
            "voicevox" => {
                let url = config.voicevox.url.clone();
                Ok(Box::new(VoicevoxBackend::new(url, speaker.to_string())))
            }
            "vits" => Ok(Box::new(VitsBackend::new(speaker.to_string()))),
            other => Err(format!("unknown backend: {other}")),
        }
    }
}

/// Shared server state passed to handlers via axum's `State` extractor.
#[derive(Clone)]
pub struct AppState {
    /// Application configuration snapshot taken at server start.
    pub config:  Arc<crate::app_config::AppConfig>,
    /// Factory for creating TTS backends.
    pub factory: Arc<dyn BackendFactory>,
}

/// `GET /health` — returns a simple health-check response.
pub async fn health() -> impl IntoResponse { Json(serde_json::json!({"status": "ok"})) }

/// Bundled HTML demo page that exercises the streaming TTS WebSocket.
const DEMO_HTML: &str = include_str!("demo.html");

/// `GET /demo` — serve the bundled web demo page.
pub async fn demo() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        DEMO_HTML,
    )
}

/// `GET /v1/voices` — list all available voices.
pub async fn list_voices() -> impl IntoResponse {
    let models_dir = crate::paths::models_dir();
    let mut voices: Vec<VoiceEntry> = Vec::new();

    // Kokoro voices (available when the model file exists)
    let kokoro_model = models_dir.join("kokoro").join("kokoro-v1.0.onnx");
    if kokoro_model.exists() {
        for name in KOKORO_VOICES {
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

    let backend = state
        .factory
        .create(&backend_name, &speaker_id, speed, &state.config)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiError::bad_request(e))))?;

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
    if KOKORO_VOICES.contains(&voice) {
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

/// `GET /ws/tts` — WebSocket endpoint for streaming TTS synthesis.
///
/// Accepts JSON text messages with a [`WsClientMessage`] payload, splits the
/// input into sentences, synthesizes each independently, and streams the
/// resulting WAV audio as individual binary frames.  Each binary frame is
/// followed by a `{"type": "chunk", "index": N}` text message, and the
/// final chunk is followed by `{"type": "done", "chunks": N}`.
///
/// The client may send `{"type": "cancel"}` at any time to abort remaining
/// chunks.  The connection stays open for multiple sequential requests.
pub async fn ws_tts(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_ws_tts(socket, state))
}

/// Inner loop that processes TTS requests on an established WebSocket.
async fn handle_ws_tts(mut socket: WebSocket, state: AppState) {
    while let Some(Ok(msg)) = socket.recv().await {
        match msg {
            Message::Text(text) => {
                match WsClientMessage::parse(&text) {
                    Ok(WsClientMessage::Tts {
                        text: tts_text,
                        voice,
                        speed,
                    }) => {
                        if process_ws_tts_request(&mut socket, &state, &tts_text, &voice, speed)
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(WsClientMessage::Cancel) => {
                        // Cancel outside of synthesis is a no-op.
                    }
                    Err(e) => {
                        if send_ws_error(&mut socket, format!("invalid JSON: {e}"))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            Message::Close(_) => break,
            // Ignore binary / ping / pong frames.
            _ => {}
        }
    }
}

/// Split text into sentences on Japanese sentence boundaries.
///
/// Visible to sibling modules for testing.
///
/// Splits on `。`, `！`, `？`, `！`, `？`, and `\n`, keeping the delimiter
/// attached to the preceding sentence.  Empty chunks are filtered out.
/// If no delimiters are found, returns the whole text as a single chunk.
pub(super) fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if matches!(ch, '。' | '！' | '？' | '!' | '?' | '\n') {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
            }
            current.clear();
        }
    }

    // Remaining text after the last delimiter.
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        sentences.push(trimmed);
    }

    sentences
}

/// Check whether an incoming WebSocket message is a cancel request.
fn is_cancel_message(msg: &Message) -> bool {
    match msg {
        Message::Text(text) => {
            WsClientMessage::parse(text).is_ok_and(|m| matches!(m, WsClientMessage::Cancel))
        }
        _ => false,
    }
}

/// Process a single TTS request with sentence-level streaming.
///
/// Splits the input text into sentences, synthesizes each one independently,
/// and sends the audio as individual binary frames.  Between sentences, the
/// socket is polled (non-blocking) for a cancel message.
///
/// Returns `Err(())` when the socket write fails (connection lost).
async fn process_ws_tts_request(
    socket: &mut WebSocket,
    state: &AppState,
    text: &str,
    voice: &str,
    speed: Option<f64>,
) -> Result<(), ()> {
    if text.trim().is_empty() {
        return send_ws_error(socket, "text must not be empty".to_string()).await;
    }

    #[allow(clippy::cast_possible_truncation)]
    let speed = speed.unwrap_or(state.config.voice.speed).clamp(0.5, 2.0) as f32;

    let (backend_name, speaker_id, rvc_model) = match resolve_voice(voice, &state.config) {
        Ok(v) => v,
        Err((_, Json(api_err))) => {
            return send_ws_error(socket, api_err.error.message).await;
        }
    };

    let sentences = split_sentences(text);
    let total = sentences.len();

    for (i, sentence) in sentences.iter().enumerate() {
        // Non-blocking check for cancel before each synthesis.
        if check_for_cancel(socket).await {
            return send_ws_json(socket, &WsResponse::cancelled()).await;
        }

        // Synthesize this sentence to a temporary file.
        let tmp_dir = match tempfile::tempdir() {
            Ok(d) => d,
            Err(e) => {
                return send_ws_error(socket, format!("failed to create temp dir: {e}")).await;
            }
        };
        let tts_output = tmp_dir.path().join("speech.wav");

        let backend = match state
            .factory
            .create(&backend_name, &speaker_id, speed, &state.config)
        {
            Ok(b) => b,
            Err(e) => return send_ws_error(socket, e).await,
        };

        if let Err(e) = backend.synthesize(sentence, &tts_output).await {
            return send_ws_error(socket, format!("TTS synthesis failed: {e}")).await;
        }

        // Apply RVC voice conversion if configured.
        let final_path = if let Some(ref model) = rvc_model {
            let rvc_output = tmp_dir.path().join("speech_rvc.wav");
            #[allow(clippy::cast_possible_truncation)]
            let index_influence = state.config.rvc.index_influence.clamp(0.0, 1.0) as f32;
            if let Err(e) = crate::rvc::convert(
                &tts_output,
                model,
                state.config.rvc.pitch,
                &state.config.rvc.pitch_algo,
                index_influence,
                &rvc_output,
            )
            .await
            {
                return send_ws_error(socket, format!("RVC conversion failed: {e}")).await;
            }
            rvc_output
        } else {
            tts_output
        };

        // Read audio bytes and send as binary frame.
        let audio_bytes = match tokio::fs::read(&final_path).await {
            Ok(b) => b,
            Err(e) => {
                return send_ws_error(socket, format!("failed to read audio file: {e}")).await;
            }
        };

        if socket
            .send(Message::Binary(audio_bytes.into()))
            .await
            .is_err()
        {
            return Err(());
        }

        // Send chunk completion marker.
        send_ws_json(socket, &WsResponse::chunk(i)).await?;
    }

    // All chunks sent successfully.
    send_ws_json(socket, &WsResponse::done(total)).await
}

/// Non-blocking check for a cancel message on the socket.
///
/// Uses a zero-duration timeout so that this returns immediately when no
/// message is pending.  Cancel is only detected between sentences, which
/// is an acceptable trade-off to avoid splitting the socket.
async fn check_for_cancel(socket: &mut WebSocket) -> bool {
    matches!(
        tokio::time::timeout(Duration::ZERO, socket.recv()).await,
        Ok(Some(Ok(msg))) if is_cancel_message(&msg)
    )
}

/// Send a JSON-serialized [`WsResponse`] as a text frame.
///
/// Returns `Err(())` when the connection is broken.
async fn send_ws_json(socket: &mut WebSocket, response: &WsResponse) -> Result<(), ()> {
    let json = serde_json::to_string(response).expect("WsResponse serializes to JSON");
    socket
        .send(Message::Text(json.into()))
        .await
        .map_err(|_| ())
}

/// Send an error message over the WebSocket.
///
/// Returns `Ok(())` if the message was sent (caller should continue the
/// loop), or `Err(())` if the connection is broken.
async fn send_ws_error(socket: &mut WebSocket, message: String) -> Result<(), ()> {
    send_ws_json(socket, &WsResponse::error(message)).await
}
