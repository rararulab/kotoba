//! OpenAI-compatible TTS API server.
//!
//! Exposes three endpoints:
//! - `POST /v1/audio/speech` — synthesize speech from text
//! - `GET /v1/voices` — list available voices
//! - `GET /health` — health check

mod handlers;
mod models;

use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};
use handlers::AppState;
use snafu::ResultExt;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::error;

/// Start the HTTP server and listen for requests.
pub async fn run(host: &str, port: u16) -> crate::error::Result<()> {
    let config = Arc::new(crate::app_config::load().clone());

    let state = AppState { config };

    let app = Router::new()
        .route("/health", get(handlers::health))
        .route("/v1/voices", get(handlers::list_voices))
        .route("/v1/audio/speech", post(handlers::speech))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .context(error::IoSnafu)?;

    eprintln!("kotoba serve listening on http://{addr}");
    eprintln!("  POST /v1/audio/speech");
    eprintln!("  GET  /v1/voices");
    eprintln!("  GET  /health");

    axum::serve(listener, app).await.context(error::IoSnafu)?;

    Ok(())
}
