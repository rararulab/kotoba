//! End-to-end tests for the serve module.
//!
//! Uses a [`StubBackendFactory`] that creates a [`StubTtsBackend`] writing
//! minimal valid WAV files, so the full handler pipeline runs without real
//! TTS models.

use std::{path::Path, sync::Arc};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite;

use super::{
    handlers::{AppState, BackendFactory},
    models::{WsClientMessage, WsResponse},
};
use crate::{app_config::AppConfig, tts::TtsBackend};

// ---------------------------------------------------------------------------
// Stub backend
// ---------------------------------------------------------------------------

/// TTS backend that writes a minimal valid WAV file (silence).
struct StubTtsBackend;

#[async_trait::async_trait]
impl TtsBackend for StubTtsBackend {
    async fn synthesize(&self, _text: &str, output: &Path) -> crate::error::Result<()> {
        write_stub_wav(output).map_err(|e| crate::error::KotobaError::Io { source: e })
    }
}

/// Write a minimal valid WAV (0.1 s of silence at 16 kHz, mono, 16-bit PCM).
fn write_stub_wav(path: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let samples: Vec<i16> = vec![0; 1600];
    #[allow(clippy::cast_possible_truncation)]
    let data_size = (samples.len() * 2) as u32;
    let mut f = std::fs::File::create(path)?;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_size).to_le_bytes())?;
    f.write_all(b"WAVE")?;
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&1u16.to_le_bytes())?; // mono
    f.write_all(&16000u32.to_le_bytes())?;
    f.write_all(&32000u32.to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_size.to_le_bytes())?;
    for s in &samples {
        f.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}

/// Factory that always returns a [`StubTtsBackend`], regardless of backend
/// name. Rejects only the literal `"bad"` backend for error-path testing.
struct StubBackendFactory;

impl BackendFactory for StubBackendFactory {
    fn create(
        &self,
        backend: &str,
        _speaker: &str,
        _speed: f32,
        _config: &AppConfig,
    ) -> Result<Box<dyn TtsBackend>, String> {
        if backend == "bad" {
            return Err(format!("unknown backend: {backend}"));
        }
        Ok(Box::new(StubTtsBackend))
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Spin up a test server with stub TTS on a random port and return the base
/// URL.
async fn test_server() -> String {
    let state = AppState {
        config:  Arc::new(AppConfig::default()),
        factory: Arc::new(StubBackendFactory),
        asr_url: Arc::new("http://localhost:8000/v1/audio/transcriptions".to_string()),
    };
    let app = super::build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind to random port");
    let port = listener.local_addr().expect("local addr").port();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("test server exited unexpectedly");
    });
    format!("http://127.0.0.1:{port}")
}

/// POST `/v1/audio/speech` and return `(status_code, body_bytes)`.
async fn speech(url: &str, input: &str, voice: &str) -> (u16, Vec<u8>) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{url}/v1/audio/speech"))
        .json(&serde_json::json!({
            "input": input,
            "voice": voice,
        }))
        .send()
        .await
        .expect("send speech request");
    let status = resp.status().as_u16();
    let body = resp.bytes().await.expect("read response body").to_vec();
    (status, body)
}

/// Decoded WebSocket frame for assertions.
#[derive(Debug)]
enum WsFrame {
    Binary,
    Text(WsResponse),
}

/// Connect to the WS endpoint, send each message, then collect all responses
/// until the server sends a `done`, `cancelled`, or `error` terminal message
/// (or the connection closes).
async fn ws_tts(url: &str, messages: Vec<String>) -> Vec<WsFrame> {
    let ws_url = url.replacen("http://", "ws://", 1);
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("{ws_url}/ws/tts"))
        .await
        .expect("ws connect");

    for msg in messages {
        ws.send(tungstenite::Message::Text(msg.into()))
            .await
            .expect("ws send");
    }

    let mut frames = Vec::new();
    while let Some(Ok(msg)) = ws.next().await {
        match msg {
            tungstenite::Message::Binary(_) => frames.push(WsFrame::Binary),
            tungstenite::Message::Text(t) => {
                let resp: WsResponse = serde_json::from_str(&t).expect("parse WsResponse");
                let terminal = matches!(resp.msg_type.as_str(), "done" | "cancelled" | "error");
                frames.push(WsFrame::Text(resp));
                if terminal {
                    break;
                }
            }
            tungstenite::Message::Close(_) => break,
            _ => {}
        }
    }
    frames
}

/// Build a tagged TTS JSON message.
fn tts_msg(text: &str, voice: &str) -> String {
    serde_json::json!({
        "type": "tts",
        "text": text,
        "voice": voice,
    })
    .to_string()
}

/// Build a legacy (untagged) TTS JSON message.
fn legacy_tts_msg(text: &str, voice: &str) -> String {
    serde_json::json!({
        "text": text,
        "voice": voice,
    })
    .to_string()
}

/// Count binary frames in a frame list.
fn count_binary(frames: &[WsFrame]) -> usize {
    frames
        .iter()
        .filter(|f| matches!(f, WsFrame::Binary))
        .count()
}

/// Extract the `chunks` field from the last `done` frame, if any.
fn last_done_chunks(frames: &[WsFrame]) -> Option<usize> {
    frames.iter().rev().find_map(|f| match f {
        WsFrame::Text(r) if r.msg_type == "done" => r.chunks,
        WsFrame::Text(_) | WsFrame::Binary => None,
    })
}

/// Extract the error message from the last `error` frame, if any.
fn last_error_message(frames: &[WsFrame]) -> Option<String> {
    frames.iter().rev().find_map(|f| match f {
        WsFrame::Text(r) if r.msg_type == "error" => r.message.clone(),
        WsFrame::Text(_) | WsFrame::Binary => None,
    })
}

// ---------------------------------------------------------------------------
// Feature 1: Health endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn health_returns_200_and_status_ok() {
    let url = test_server().await;
    let resp = reqwest::get(format!("{url}/health"))
        .await
        .expect("health request");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn demo_returns_html_page() {
    let url = test_server().await;
    let resp = reqwest::get(format!("{url}/demo"))
        .await
        .expect("demo request");
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .expect("content-type header")
        .to_str()
        .expect("ascii content-type");
    assert!(ct.starts_with("text/html"), "got content-type: {ct}");
    let body = resp.text().await.expect("html body");
    assert!(body.contains("<title>kotoba"));
    // mlx-live-style voice pipeline: connects via /ws/voice and uses
    // an AudioWorklet for PCM recording.
    assert!(body.contains("/ws/voice"));
    assert!(body.contains("pcm-recorder-worklet"));
    assert!(body.contains("AudioWorklet"));
    // Settings are persisted in localStorage.
    assert!(body.contains("localStorage"));
}

#[tokio::test]
async fn pcm_worklet_returns_javascript() {
    let url = test_server().await;
    let resp = reqwest::get(format!("{url}/static/pcm-recorder-worklet.js"))
        .await
        .expect("worklet request");
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .expect("content-type header")
        .to_str()
        .expect("ascii content-type");
    assert!(
        ct.contains("javascript"),
        "expected javascript content-type, got: {ct}"
    );
    let body = resp.text().await.expect("js body");
    assert!(body.contains("PcmRecorderProcessor"));
    assert!(body.contains("registerProcessor"));
}

// ---------------------------------------------------------------------------
// Feature 2: Voices endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn voices_returns_json_list() {
    let url = test_server().await;
    let resp = reqwest::get(format!("{url}/v1/voices"))
        .await
        .expect("voices request");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert!(body["voices"].is_array());
}

// ---------------------------------------------------------------------------
// Feature 3: Speech endpoint returns WAV
// ---------------------------------------------------------------------------

#[tokio::test]
async fn speech_returns_wav_audio() {
    let url = test_server().await;
    let (status, body) = speech(&url, "hello", "kokoro:jf_alpha").await;
    assert_eq!(status, 200);
    assert!(body.starts_with(b"RIFF"), "response should be valid WAV");
}

// ---------------------------------------------------------------------------
// Feature 4: Speech endpoint validation errors
// ---------------------------------------------------------------------------

#[tokio::test]
async fn speech_rejects_empty_input() {
    let url = test_server().await;
    let (status, _) = speech(&url, "", "kokoro:jf_alpha").await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn speech_rejects_unsupported_format() {
    let url = test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{url}/v1/audio/speech"))
        .json(&serde_json::json!({
            "input": "hello",
            "voice": "kokoro:jf_alpha",
            "response_format": "mp3",
        }))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn speech_rejects_bad_voice() {
    let url = test_server().await;
    let (status, _) = speech(&url, "hello", "bad:speaker").await;
    assert_eq!(status, 400);
}

// ---------------------------------------------------------------------------
// Feature 5: WebSocket single sentence
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_single_sentence_produces_one_chunk() {
    let url = test_server().await;
    let frames = ws_tts(&url, vec![tts_msg("hello", "kokoro:jf_alpha")]).await;
    assert_eq!(count_binary(&frames), 1);
    assert_eq!(last_done_chunks(&frames), Some(1));
}

// ---------------------------------------------------------------------------
// Feature 6: WebSocket multi-sentence
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_multi_sentence_produces_n_chunks() {
    let url = test_server().await;
    let frames = ws_tts(&url, vec![tts_msg("あ。い。う", "kokoro:jf_alpha")]).await;
    // "あ。い。う" splits into ["あ。", "い。", "う"] -> 3 sentences
    assert_eq!(count_binary(&frames), 3);
    assert_eq!(last_done_chunks(&frames), Some(3));
}

// ---------------------------------------------------------------------------
// Feature 7: WebSocket empty text
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_empty_text_returns_error() {
    let url = test_server().await;
    let frames = ws_tts(&url, vec![tts_msg("", "kokoro:jf_alpha")]).await;
    assert!(
        last_error_message(&frames).is_some(),
        "expected an error frame for empty text"
    );
}

// ---------------------------------------------------------------------------
// Feature 8: WebSocket cancel
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_cancel_returns_cancelled() {
    let url = test_server().await;
    let ws_url = url.replacen("http://", "ws://", 1);
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("{ws_url}/ws/tts"))
        .await
        .expect("ws connect");

    // Send a multi-sentence request, then immediately cancel.
    let text = "あ。い。う。え。お。か。き。く。け。こ。";
    ws.send(tungstenite::Message::Text(
        tts_msg(text, "kokoro:jf_alpha").into(),
    ))
    .await
    .expect("send tts");
    ws.send(tungstenite::Message::Text(
        serde_json::json!({"type": "cancel"}).to_string().into(),
    ))
    .await
    .expect("send cancel");

    let mut frames = Vec::new();
    while let Some(Ok(msg)) = ws.next().await {
        match msg {
            tungstenite::Message::Text(t) => {
                let resp: WsResponse = serde_json::from_str(&t).expect("parse WsResponse");
                let terminal = matches!(resp.msg_type.as_str(), "done" | "cancelled" | "error");
                frames.push(WsFrame::Text(resp));
                if terminal {
                    break;
                }
            }
            tungstenite::Message::Binary(_) => frames.push(WsFrame::Binary),
            tungstenite::Message::Close(_) => break,
            _ => {}
        }
    }

    // The server should have either completed (done) or cancelled. Both are
    // acceptable since the stub backend is very fast, but the cancel path
    // should not panic or hang.
    let last_type = frames.iter().rev().find_map(|f| match f {
        WsFrame::Text(r) => Some(r.msg_type.as_str().to_string()),
        WsFrame::Binary => None,
    });
    assert!(
        matches!(last_type.as_deref(), Some("done" | "cancelled")),
        "expected done or cancelled, got {last_type:?}"
    );
}

// ---------------------------------------------------------------------------
// Feature 9: Legacy untagged JSON format
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_legacy_untagged_format_works() {
    let url = test_server().await;
    let frames = ws_tts(&url, vec![legacy_tts_msg("hello", "kokoro:jf_alpha")]).await;
    assert_eq!(count_binary(&frames), 1);
    assert_eq!(last_done_chunks(&frames), Some(1));
}

// ---------------------------------------------------------------------------
// Feature 10: Sentence splitting (pure function)
// ---------------------------------------------------------------------------

/// Access the private `split_sentences` via a re-export trick: we test it
/// from the same crate so we can call it directly.
use super::handlers::split_sentences;

#[test]
fn split_sentences_japanese_periods() {
    let result = split_sentences("あ。い。う");
    assert_eq!(result, vec!["あ。", "い。", "う"]);
}

#[test]
fn split_sentences_no_delimiter() {
    let result = split_sentences("hello world");
    assert_eq!(result, vec!["hello world"]);
}

#[test]
fn split_sentences_mixed_delimiters() {
    let result = split_sentences("あ！い？う。");
    assert_eq!(result, vec!["あ！", "い？", "う。"]);
}

#[test]
fn split_sentences_empty_string() {
    let result = split_sentences("");
    assert!(result.is_empty());
}

#[test]
fn split_sentences_trailing_newline() {
    let result = split_sentences("line one\nline two\n");
    assert_eq!(result, vec!["line one", "line two"]);
}

// ---------------------------------------------------------------------------
// WsClientMessage::parse (pure function)
// ---------------------------------------------------------------------------

#[test]
fn parse_tagged_tts_message() {
    let msg = r#"{"type":"tts","text":"hello","voice":"kokoro:jf_alpha"}"#;
    let parsed = WsClientMessage::parse(msg).expect("parse");
    assert!(matches!(parsed, WsClientMessage::Tts { .. }));
}

#[test]
fn parse_tagged_cancel_message() {
    let msg = r#"{"type":"cancel"}"#;
    let parsed = WsClientMessage::parse(msg).expect("parse");
    assert!(matches!(parsed, WsClientMessage::Cancel));
}

#[test]
fn parse_legacy_untagged_message() {
    let msg = r#"{"text":"hello","voice":"kokoro:jf_alpha"}"#;
    let parsed = WsClientMessage::parse(msg).expect("parse");
    assert!(matches!(parsed, WsClientMessage::Tts { .. }));
}

#[test]
fn parse_invalid_json_returns_error() {
    let result = WsClientMessage::parse("not json");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// WsResponse serialization (pure function)
// ---------------------------------------------------------------------------

#[test]
fn ws_response_chunk_serializes_correctly() {
    let resp = WsResponse::chunk(2);
    let json: serde_json::Value = serde_json::to_value(&resp).expect("serialize");
    assert_eq!(json["type"], "chunk");
    assert_eq!(json["index"], 2);
    assert!(json.get("chunks").is_none());
    assert!(json.get("message").is_none());
}

#[test]
fn ws_response_done_serializes_correctly() {
    let resp = WsResponse::done(5);
    let json: serde_json::Value = serde_json::to_value(&resp).expect("serialize");
    assert_eq!(json["type"], "done");
    assert_eq!(json["chunks"], 5);
}

#[test]
fn ws_response_error_serializes_correctly() {
    let resp = WsResponse::error("something broke");
    let json: serde_json::Value = serde_json::to_value(&resp).expect("serialize");
    assert_eq!(json["type"], "error");
    assert_eq!(json["message"], "something broke");
}

#[test]
fn ws_response_cancelled_serializes_correctly() {
    let resp = WsResponse::cancelled();
    let json: serde_json::Value = serde_json::to_value(&resp).expect("serialize");
    assert_eq!(json["type"], "cancelled");
}
