//! Request and response types for the OpenAI-compatible TTS API.

use serde::{Deserialize, Serialize};

/// OpenAI-compatible speech synthesis request body.
#[derive(Debug, Deserialize)]
pub struct SpeechRequest {
    /// Model identifier (currently ignored; included for API compatibility).
    #[allow(dead_code)]
    pub model:           Option<String>,
    /// Text to synthesize into speech.
    pub input:           String,
    /// Voice identifier: `backend:speaker_id`, an RVC model name, or a bare
    /// Kokoro voice name.
    pub voice:           String,
    /// Desired output audio format (e.g. "wav", "mp3"). Defaults to "wav".
    pub response_format: Option<String>,
    /// Speech speed multiplier. Defaults to 1.0.
    pub speed:           Option<f64>,
}

/// A single voice entry returned by the voice listing endpoint.
#[derive(Debug, Serialize)]
pub struct VoiceEntry {
    /// Unique voice identifier usable in `SpeechRequest.voice`.
    pub id:      String,
    /// Human-readable voice name.
    pub name:    String,
    /// Backend that provides this voice.
    pub backend: String,
}

/// Response body for `GET /v1/voices`.
#[derive(Debug, Serialize)]
pub struct VoiceListResponse {
    /// Available voice entries.
    pub voices: Vec<VoiceEntry>,
}

/// OpenAI-compatible error response body.
#[derive(Debug, Serialize)]
pub struct ApiError {
    /// Error details.
    pub error: ApiErrorDetail,
}

/// Inner error detail matching the `OpenAI` error format.
#[derive(Debug, Serialize)]
pub struct ApiErrorDetail {
    /// Human-readable error message.
    pub message:    String,
    /// Error type classification.
    #[serde(rename = "type")]
    pub error_type: String,
    /// HTTP status code.
    pub code:       u16,
}

/// Incoming WebSocket TTS request payload.
#[derive(Debug, Deserialize)]
pub struct WsTtsRequest {
    /// Text to synthesize into speech.
    pub text:  String,
    /// Voice identifier (same format as `SpeechRequest.voice`).
    pub voice: String,
    /// Speech speed multiplier. Defaults to 1.0 when absent.
    pub speed: Option<f64>,
}

/// WebSocket response message sent after successful synthesis or on error.
#[derive(Debug, Serialize)]
pub struct WsResponse {
    /// Message type: `"done"` or `"error"`.
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Error message (only present when `msg_type` is `"error"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message:  Option<String>,
}

impl WsResponse {
    /// Create a `{"type": "done"}` response.
    pub fn done() -> Self {
        Self {
            msg_type: "done".to_string(),
            message:  None,
        }
    }

    /// Create a `{"type": "error", "message": "..."}` response.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            msg_type: "error".to_string(),
            message:  Some(message.into()),
        }
    }
}

impl ApiError {
    /// Create a bad-request (400) error response.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            error: ApiErrorDetail {
                message:    message.into(),
                error_type: "invalid_request_error".to_string(),
                code:       400,
            },
        }
    }

    /// Create an internal-server-error (500) response.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            error: ApiErrorDetail {
                message:    message.into(),
                error_type: "server_error".to_string(),
                code:       500,
            },
        }
    }
}
