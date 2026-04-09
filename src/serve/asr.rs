//! Managed Whisper ASR child process.
//!
//! Spawns a Python-based Whisper ASR server as a child process using `uv run`,
//! writing the embedded Python script to a temporary file at runtime.

use std::time::Duration;

use snafu::{ResultExt, Snafu};
use tempfile::NamedTempFile;
use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

/// Embedded Python script for the Whisper ASR server.
const WHISPER_SCRIPT: &str = include_str!("whisper_server.py");

/// Errors that can occur while managing the Whisper ASR process.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum AsrError {
    /// `uv` is not installed or not on `PATH`.
    #[snafu(display(
        "uv not found on PATH — install with: curl -LsSf https://astral.sh/uv/install.sh | sh"
    ))]
    UvNotFound,

    /// Failed to write the embedded Python script to a temp file.
    #[snafu(display("failed to write whisper script: {source}"))]
    ScriptWrite { source: std::io::Error },

    /// Failed to spawn the `uv run` child process.
    #[snafu(display("failed to spawn whisper process: {source}"))]
    Spawn { source: std::io::Error },

    /// The server did not become ready within the timeout.
    #[snafu(display("whisper server did not start within {timeout_secs}s"))]
    StartTimeout { timeout_secs: u64 },

    /// Failed to find a free port.
    #[snafu(display("failed to bind ephemeral port: {source}"))]
    PortBind { source: std::io::Error },
}

/// Result type for ASR operations.
pub type Result<T> = std::result::Result<T, AsrError>;

/// Managed Whisper ASR child process.
pub struct WhisperProcess {
    /// The spawned child process.
    child:   Child,
    /// The port the server is listening on.
    port:    u16,
    /// Keep the temp file alive so the script is not deleted.
    _script: NamedTempFile,
}

impl WhisperProcess {
    /// Start the Whisper ASR server on a random available port.
    ///
    /// Writes the embedded Python script to a temp file, then spawns it
    /// via `uv run` with the required dependencies.
    pub async fn start() -> Result<Self> {
        // Verify uv is available.
        let uv_ok = Command::new("uv")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;

        match uv_ok {
            Ok(status) if status.success() => {}
            _ => return Err(AsrError::UvNotFound),
        }

        // Find a free port by binding to port 0.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").context(PortBindSnafu)?;
            listener.local_addr().context(PortBindSnafu)?.port()
        };

        // Write the embedded script to a temp file.
        let script = tempfile::Builder::new()
            .prefix("kotoba-whisper-")
            .suffix(".py")
            .tempfile()
            .context(ScriptWriteSnafu)?;

        std::fs::write(script.path(), WHISPER_SCRIPT).context(ScriptWriteSnafu)?;

        info!(port, "spawning whisper ASR server");

        let child = Command::new("uv")
            .arg("run")
            .arg("--python")
            .arg("3.11")
            .arg("--with")
            .arg("faster-whisper")
            .arg("--with")
            .arg("fastapi[standard]")
            .arg("python")
            .arg(script.path())
            .arg("large-v3-turbo")
            .arg(port.to_string())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context(SpawnSnafu)?;

        let process = Self {
            child,
            port,
            _script: script,
        };

        process.wait_ready(Duration::from_secs(120)).await?;

        Ok(process)
    }

    /// The URL of the running ASR endpoint.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/v1/audio/transcriptions", self.port)
    }

    /// Wait for the server to be ready by polling the TCP port.
    async fn wait_ready(&self, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        let addr = format!("127.0.0.1:{}", self.port);

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(AsrError::StartTimeout {
                    timeout_secs: timeout.as_secs(),
                });
            }

            match tokio::net::TcpStream::connect(&addr).await {
                Ok(_) => {
                    debug!(port = self.port, "whisper ASR server is ready");
                    return Ok(());
                }
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }

    /// Kill the child process.
    pub async fn shutdown(&mut self) {
        if let Err(e) = self.child.kill().await {
            warn!("failed to kill whisper process: {e}");
        }
    }
}
