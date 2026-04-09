//! OpenAI-compatible TTS API server.
//!
//! Exposes seven endpoints:
//! - `POST /v1/audio/speech` — synthesize speech from text
//! - `GET /v1/voices` — list available voices
//! - `GET /health` — health check
//! - `WS /ws/tts` — streaming TTS over WebSocket
//! - `WS /ws/voice` — real-time voice conversation pipeline
//! - `GET /static/pcm-recorder-worklet.js` — `AudioWorklet` processor for PCM
//!   recording
//! - `GET /demo` — bundled web demo for real-time voice conversation

mod asr;
mod handlers;
mod models;
#[cfg(test)]
mod tests;
mod voice;

use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};
use handlers::{AppState, DefaultBackendFactory};
use snafu::ResultExt;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::error;

/// Build the axum router from the given application state.
///
/// Extracted so that tests can construct a router with a custom
/// [`AppState`] (e.g. a stub backend factory).
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(handlers::health))
        .route("/v1/voices", get(handlers::list_voices))
        .route("/v1/audio/speech", post(handlers::speech))
        .route("/ws/tts", get(handlers::ws_tts))
        .route("/ws/voice", get(handlers::voice_ws))
        .route(
            "/static/pcm-recorder-worklet.js",
            get(handlers::pcm_worklet),
        )
        .route("/demo", get(handlers::demo))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the HTTP server and listen for requests.
pub async fn run(host: &str, port: u16) -> crate::error::Result<()> {
    // Start managed Whisper ASR server.
    eprintln!("Starting Whisper ASR server...");
    let mut whisper = match asr::WhisperProcess::start().await {
        Ok(w) => {
            eprintln!("  Whisper ASR: {}", w.url());
            Some(w)
        }
        Err(e) => {
            eprintln!("  WARNING: Failed to start Whisper ASR: {e}");
            eprintln!("  Voice pipeline will use external ASR endpoint");
            None
        }
    };

    let asr_url = whisper.as_ref().map_or_else(
        || "http://localhost:8000/v1/audio/transcriptions".to_string(),
        asr::WhisperProcess::url,
    );

    let config = Arc::new(crate::app_config::load().clone());

    let state = AppState {
        config,
        factory: Arc::new(DefaultBackendFactory),
        asr_url: Arc::new(asr_url),
    };

    let app = build_router(state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .context(error::IoSnafu)?;

    eprintln!("kotoba serve listening on http://{addr}");
    eprintln!("  POST /v1/audio/speech");
    eprintln!("  GET  /v1/voices");
    eprintln!("  WS   /ws/tts");
    eprintln!("  WS   /ws/voice");
    eprintln!("  GET  /health");
    eprintln!("  GET  /demo  →  http://{addr}/demo");

    axum::serve(listener, app).await.context(error::IoSnafu)?;

    // Cleanup on exit.
    if let Some(ref mut w) = whisper {
        w.shutdown().await;
    }

    Ok(())
}
