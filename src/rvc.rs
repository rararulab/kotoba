//! RVC v2 voice conversion client.
//!
//! Communicates with the RVC sidecar service to convert
//! base TTS audio into anime character voices.

use std::path::Path;

use snafu::ResultExt;

use crate::error::{self, Result};

/// Return the RVC sidecar base URL (`RVC_URL` env var, or `http://localhost:50022`).
pub fn rvc_base_url() -> String {
    std::env::var("RVC_URL").unwrap_or_else(|_| "http://localhost:50022".to_string())
}

/// Check that the RVC sidecar is reachable.
pub async fn check_reachable() -> Result<()> {
    let url = rvc_base_url();
    crate::http::client()
        .get(format!("{url}/version"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .map_err(|_| error::RvcNotRunningSnafu { url: url.clone() }.build())?;
    Ok(())
}

/// Convert audio at `input_path` using the given RVC model,
/// writing the result to `output_path`.
pub async fn convert(input_path: &Path, model: &str, output_path: &Path) -> Result<()> {
    let url = rvc_base_url();
    let client = crate::http::client();

    let input_bytes = std::fs::read(input_path).context(error::IoSnafu)?;

    let part = reqwest::multipart::Part::bytes(input_bytes)
        .file_name("input.wav")
        .mime_str("audio/wav")
        .expect("valid mime type");

    let form = reqwest::multipart::Form::new().part("file", part);

    let response = client
        .post(format!("{url}/convert"))
        .query(&[("model", model)])
        .multipart(form)
        .send()
        .await
        .context(error::HttpSnafu)?;

    if !response.status().is_success() {
        return Err(error::RvcSnafu {
            message: format!("RVC conversion failed: HTTP {}", response.status()),
        }
        .build());
    }

    let audio = response.bytes().await.context(error::HttpSnafu)?;
    std::fs::write(output_path, &audio).context(error::IoSnafu)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rvc_url_default() {
        let url = rvc_base_url();
        assert!(url.starts_with("http"));
        assert!(url.contains("50022"));
    }
}
