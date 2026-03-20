//! VOICEVOX TTS audio generation and caching.

use std::path::PathBuf;

use snafu::ResultExt;

use crate::error::{self, Result};

fn cache_dir() -> Result<PathBuf> {
    let dir = dirs::home_dir()
        .ok_or(error::HomeNotFoundSnafu.build())?
        .join(".kotoba")
        .join("audio");
    std::fs::create_dir_all(&dir).context(error::IoSnafu)?;
    Ok(dir)
}

/// Generate or return a cached WAV file for a word via VOICEVOX.
pub async fn play_word(word: &str) -> Result<PathBuf> {
    let cache = cache_dir()?;
    let file = cache.join(format!("{word}.wav"));

    if file.exists() {
        return Ok(file);
    }

    let base_url =
        std::env::var("VOICEVOX_URL").unwrap_or_else(|_| "http://localhost:50021".to_string());
    let client = reqwest::Client::new();

    let query: serde_json::Value = client
        .post(format!("{base_url}/audio_query"))
        .query(&[("text", word), ("speaker", "1")])
        .send()
        .await
        .context(error::HttpSnafu)?
        .json()
        .await
        .context(error::HttpSnafu)?;

    let audio = client
        .post(format!("{base_url}/synthesis"))
        .query(&[("speaker", "1")])
        .json(&query)
        .send()
        .await
        .context(error::HttpSnafu)?
        .bytes()
        .await
        .context(error::HttpSnafu)?;

    std::fs::write(&file, &audio).context(error::IoSnafu)?;
    Ok(file)
}
