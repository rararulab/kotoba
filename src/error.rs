//! Application-level error types.

use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum KotobaError {
    #[snafu(display("store error: {source}"))]
    Store { source: crate::store::StoreError },

    #[snafu(display("sqlx error: {source}"))]
    Sqlx { source: sqlx::Error },

    #[snafu(display("migration error: {source}"))]
    Migrate { source: sqlx::migrate::MigrateError },

    #[snafu(display("word not found: {word}"))]
    WordNotFound { word: String },

    #[snafu(display("grammar not found: {pattern}"))]
    GrammarNotFound { pattern: String },

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

    #[snafu(display("database not initialized — run `kotoba init` first"))]
    DatabaseNotInitialized,

    #[snafu(display("VOICEVOX not installed — run `kotoba setup`"))]
    VoicevoxNotInstalled,

    #[snafu(display("VOICEVOX not running at {url}"))]
    VoicevoxNotRunning { url: String },

    #[snafu(display("model not found: {name} — download with `kotoba voice add`"))]
    ModelNotFound { name: String },

    #[snafu(display("checksum mismatch: expected {expected}, got {actual}"))]
    ChecksumMismatch { expected: String, actual: String },

    #[snafu(display("zip error: {message}"))]
    Zip { message: String },

    #[snafu(display("Kokoro TTS error: {source}"))]
    Kokoro { source: crate::kokoro::KokoroError },

    #[snafu(display("RVC error: {message}"))]
    Rvc { message: String },

    #[snafu(display("download failed for {url}: HTTP {status}"))]
    DownloadFailed { url: String, status: String },
}

pub type Result<T> = std::result::Result<T, KotobaError>;
