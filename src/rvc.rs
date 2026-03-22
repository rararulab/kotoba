//! RVC v2 voice conversion via a persistent Python sidecar.
//!
//! On first use, spawns `python3 server.py` as a background subprocess
//! listening on `127.0.0.1:50022`. Subsequent calls reuse the running
//! server via HTTP.

use std::{path::Path, time::Duration};

use snafu::{ensure, ResultExt};

use crate::error::{self, Result};

/// Default port the RVC sidecar listens on.
const DEFAULT_PORT: u16 = 50022;

/// Embedded copy of the sidecar server script.
const SERVER_PY: &str = include_str!("../rvc-sidecar/server.py");

/// Return the RVC sidecar base URL.
pub fn base_url() -> String {
    std::env::var("RVC_URL").unwrap_or_else(|_| format!("http://127.0.0.1:{DEFAULT_PORT}"))
}

/// Check whether the sidecar is already responding with a healthy status.
async fn is_running() -> bool {
    let url = base_url();
    crate::http::client()
        .get(format!("{url}/version"))
        .timeout(Duration::from_secs(1))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

/// Return the directory where the sidecar script is installed.
fn sidecar_dir() -> std::path::PathBuf { crate::paths::data_dir().join("rvc-sidecar") }

/// Install the embedded `server.py` to disk if missing or outdated.
fn install_server_script() -> Result<std::path::PathBuf> {
    let dir = sidecar_dir();
    std::fs::create_dir_all(&dir).context(error::IoSnafu)?;

    let script_path = dir.join("server.py");

    // Always overwrite to keep in sync with the embedded version
    std::fs::write(&script_path, SERVER_PY).context(error::IoSnafu)?;

    Ok(script_path)
}

/// Spawn the sidecar as a detached background process.
///
/// Writes the child PID to `sidecar_dir()/pid` for lifecycle tracking,
/// and redirects stderr to `sidecar_dir()/sidecar.log` for diagnostics.
fn spawn_server(script_path: &Path) -> Result<()> {
    let port = std::env::var("RVC_PORT").unwrap_or_else(|_| DEFAULT_PORT.to_string());
    let dir = sidecar_dir();
    let log_file = std::fs::File::create(dir.join("sidecar.log")).context(error::IoSnafu)?;

    let child = std::process::Command::new("python3")
        .arg(script_path)
        .env("RVC_PORT", &port)
        .stdout(std::process::Stdio::null())
        .stderr(log_file)
        .stdin(std::process::Stdio::null())
        .spawn()
        .context(error::IoSnafu)?;

    // Record PID so the sidecar can be identified or cleaned up later
    let _ = std::fs::write(dir.join("pid"), child.id().to_string());

    Ok(())
}

/// Ensure the RVC sidecar is running, starting it if necessary.
///
/// Checks `/version` first. If unreachable, installs the embedded
/// `server.py` and spawns it as a background subprocess, then waits
/// up to 10 seconds for it to become ready.
pub async fn ensure_running() -> Result<()> {
    if is_running().await {
        return Ok(());
    }

    let script_path = install_server_script()?;
    spawn_server(&script_path)?;

    // Wait for server to become ready
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if is_running().await {
            return Ok(());
        }
    }

    error::RvcSnafu {
        message: format!(
            "sidecar failed to start at {} — check python3 and uvicorn are installed",
            base_url()
        ),
    }
    .fail()
}

/// Validate that a model name contains no path traversal characters.
fn validate_model_name(model: &str) -> Result<()> {
    ensure!(
        !model.is_empty() && !model.contains('/') && !model.contains('\\') && !model.contains(".."),
        error::RvcSnafu {
            message: format!("invalid model name: {model}"),
        }
    );
    Ok(())
}

/// Convert audio at `input_path` using the given RVC model,
/// writing the result to `output_path`.
///
/// Sends the WAV file to the sidecar's `/convert` endpoint via
/// HTTP multipart upload.
pub async fn convert(input_path: &Path, model: &str, output_path: &Path) -> Result<()> {
    validate_model_name(model)?;

    let url = base_url();
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
        let body = response.text().await.unwrap_or_default();
        return error::RvcSnafu {
            message: format!("conversion failed: {body}"),
        }
        .fail();
    }

    let audio = response.bytes().await.context(error::HttpSnafu)?;
    std::fs::write(output_path, &audio).context(error::IoSnafu)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_contains_port() {
        let url = base_url();
        assert!(url.starts_with("http"));
        assert!(url.contains("50022"));
    }

    #[test]
    fn sidecar_dir_is_under_data() {
        let dir = sidecar_dir();
        assert!(dir.ends_with("rvc-sidecar"));
    }

    #[test]
    fn validate_rejects_path_traversal() {
        assert!(validate_model_name("../evil").is_err());
        assert!(validate_model_name("foo/bar").is_err());
        assert!(validate_model_name("foo\\bar").is_err());
        assert!(validate_model_name("").is_err());
    }

    #[test]
    fn validate_accepts_normal_names() {
        assert!(validate_model_name("naruto-rvc-v2").is_ok());
        assert!(validate_model_name("my_model").is_ok());
    }

    #[test]
    fn server_py_is_embedded() {
        assert!(SERVER_PY.contains("kotoba-rvc-sidecar"));
    }
}
