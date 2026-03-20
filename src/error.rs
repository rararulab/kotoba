//! Application-level error types.

use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum KotobaError {
    #[snafu(display("store error: {source}"))]
    Store { source: crate::store::StoreError },

    #[snafu(display("sqlx error: {source}"))]
    Sqlx { source: sqlx::Error },

    #[snafu(display("word not found: {word}"))]
    WordNotFound { word: String },

    #[snafu(display("grammar not found: {pattern}"))]
    GrammarNotFound { pattern: String },

    #[snafu(display("invalid quality rating: {value} (must be 1, 3, or 5)"))]
    InvalidQuality { value: u8 },

    #[snafu(display("home directory not found"))]
    HomeNotFound,

    #[snafu(display("IO error: {source}"))]
    Io { source: std::io::Error },

    #[snafu(display("VOICEVOX error: {message}"))]
    Voicevox { message: String },

    #[snafu(display("HTTP error: {source}"))]
    Http { source: reqwest::Error },

    #[snafu(display("JSON error: {source}"))]
    Json { source: serde_json::Error },

    #[snafu(display("unknown export format: {format} (use json, csv, or anki)"))]
    UnknownFormat { format: String },
}

pub type Result<T> = std::result::Result<T, KotobaError>;
