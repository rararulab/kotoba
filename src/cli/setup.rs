//! `kotoba setup` — initialize environment and configure default voice.

use std::{
    io::{Read as _, Write},
    process::{Command, Stdio},
    time::Duration,
};

use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::StatusCode;
use serde::Serialize;
use sha2::{Digest, Sha256};
use snafu::ResultExt;

use crate::{
    app_config::{self, AppConfig},
    db::Database,
    error::{self, Result},
};

const DEFAULT_SETUP_KOKORO_VOICE: &str = "jf_alpha";
const DEFAULT_SETUP_RVC_SPEC: &str = "rvc:lexaizero/ReuploadModel:by swapno Nakano Ichika (CV \
                                      Hanazawa Kana ) From - The Quintessential Quintuplets 300 \
                                      Epochs (RVC v2).zip";
const DEFAULT_SETUP_VOICE_SPEED: f64 = 0.90;

/// Result of running the setup command.
#[derive(Debug, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct SetupResult {
    /// Path to the database file.
    pub db_path:              String,
    /// Whether VOICEVOX Engine is installed after setup.
    pub voicevox_installed:   bool,
    /// Whether VOICEVOX API is reachable after setup.
    pub voicevox_running:     bool,
    /// Whether `CosyVoice` runtime command was auto-detected and saved.
    pub cosyvoice_configured: bool,
    /// Whether `CosyVoice` API is reachable after setup.
    pub cosyvoice_running:    bool,
}

fn voicevox_download_url(version: &str) -> String {
    // macOS assets: voicevox_engine-macos-{arch}-{ver}.vvpp (no -cpu suffix)
    // Linux/Windows: voicevox_engine-{os}-cpu-{ver}.vvpp (no arch, has -cpu)
    let platform = if cfg!(target_os = "macos") {
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        };
        format!("macos-{arch}")
    } else if cfg!(target_os = "linux") {
        "linux-cpu".to_owned()
    } else {
        "windows-cpu".to_owned()
    };

    format!(
        "https://github.com/VOICEVOX/voicevox_engine/releases/download/{version}/voicevox_engine-{platform}-{version}.vvpp"
    )
}

/// Check if VOICEVOX Engine is already installed.
pub fn is_voicevox_installed() -> bool {
    let dir = crate::paths::voicevox_dir();
    dir.exists() && dir.join("run").exists()
}

/// Run full setup: initialize DB, install VOICEVOX, and configure default
/// Kokoro+RVC voice.
pub async fn run(db: &Database) -> Result<SetupResult> {
    eprintln!("initializing database...");
    db.init().await?;
    eprintln!("  database ready at {}", db.path().display());

    let cfg = crate::app_config::load();
    let version = cfg.voicevox.version.clone();
    let voicevox_url = std::env::var("VOICEVOX_URL").unwrap_or_else(|_| cfg.voicevox.url.clone());

    if is_voicevox_installed() {
        eprintln!("  voicevox engine already installed");
    } else {
        download_voicevox(&version).await?;
    }

    eprintln!("  ensuring default Kokoro model...");
    crate::cli::huggingface::add("kokoro").await?;
    eprintln!("  ensuring default Hanazawa RVC model...");
    let rvc_result = crate::cli::huggingface::add(DEFAULT_SETUP_RVC_SPEC).await?;

    let mut updated = app_config::load().clone();
    let mut cosyvoice_configured = false;
    if updated.cosyvoice.command.trim().is_empty() && std::env::var("COSYVOICE_CMD").is_err() {
        eprintln!("  preparing managed CosyVoice runtime (clone + venv + deps)...");
        match crate::cosyvoice_runtime::bootstrap_managed_install() {
            Ok(command_template) => {
                updated.cosyvoice.command = command_template;
                cosyvoice_configured = true;
                eprintln!("  managed CosyVoice runtime prepared");
            }
            Err(err) => {
                eprintln!("  managed CosyVoice bootstrap failed: {err}");
                if let Some(inferred) = crate::cosyvoice_runtime::infer_command() {
                    updated.cosyvoice.command = inferred;
                    cosyvoice_configured = true;
                    eprintln!("  fell back to auto-detected local CosyVoice command");
                }
            }
        }
    }

    apply_default_voice_preset(&mut updated, &rvc_result.model);
    app_config::save(&updated).context(error::IoSnafu)?;
    eprintln!(
        "  default voice set to {} (speed={})",
        updated.voice.active, updated.voice.speed
    );

    ensure_voicevox_running(&voicevox_url).await?;

    let cosyvoice_running = match crate::cosyvoice_runtime::ensure_running(&updated.cosyvoice).await
    {
        Ok(()) => true,
        Err(err) => {
            eprintln!("  cosyvoice runtime not ready: {err}");
            false
        }
    };

    eprintln!("setup complete!");

    Ok(SetupResult {
        db_path: db.path().display().to_string(),
        voicevox_installed: is_voicevox_installed(),
        voicevox_running: is_voicevox_api_ready(&voicevox_url).await,
        cosyvoice_configured,
        cosyvoice_running,
    })
}

fn apply_default_voice_preset(cfg: &mut AppConfig, rvc_model_name: &str) {
    cfg.voice.active = format!("kokoro:{DEFAULT_SETUP_KOKORO_VOICE}");
    cfg.voice.speed = DEFAULT_SETUP_VOICE_SPEED;
    cfg.rvc.model = rvc_model_name.to_string();
}

fn voicevox_version_url(base_url: &str) -> String {
    format!("{}/version", base_url.trim_end_matches('/'))
}

async fn is_voicevox_api_ready(base_url: &str) -> bool {
    crate::http::client()
        .get(voicevox_version_url(base_url))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .is_ok_and(|resp| resp.status().is_success())
}

fn parse_voicevox_bind_addr(base_url: &str) -> Result<(String, u16)> {
    let url = reqwest::Url::parse(base_url).map_err(|e| {
        error::VoicevoxSnafu {
            message: format!("invalid VOICEVOX URL `{base_url}`: {e}"),
        }
        .build()
    })?;

    let host = url.host_str().ok_or_else(|| {
        error::VoicevoxSnafu {
            message: format!("VOICEVOX URL `{base_url}` does not include a host"),
        }
        .build()
    })?;

    let port = url.port_or_known_default().ok_or_else(|| {
        error::VoicevoxSnafu {
            message: format!("VOICEVOX URL `{base_url}` does not include a valid port"),
        }
        .build()
    })?;

    Ok((host.to_string(), port))
}

fn start_voicevox_engine(base_url: &str) -> Result<()> {
    let executable = crate::paths::voicevox_executable();
    if !executable.exists() {
        return error::VoicevoxNotInstalledSnafu.fail();
    }

    let (host, port) = parse_voicevox_bind_addr(base_url)?;
    let voicevox_dir = crate::paths::voicevox_dir();
    let log_path = voicevox_dir.join("engine.log");

    std::fs::create_dir_all(&voicevox_dir).context(error::IoSnafu)?;
    let stdout_log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .context(error::IoSnafu)?;
    let stderr_log = stdout_log.try_clone().context(error::IoSnafu)?;

    Command::new(&executable)
        .arg("--host")
        .arg(&host)
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(stderr_log))
        .spawn()
        .context(error::IoSnafu)?;

    eprintln!(
        "  started voicevox engine on {host}:{port} (logs: {})",
        log_path.display()
    );

    Ok(())
}

async fn wait_for_voicevox_ready(base_url: &str, timeout: Duration) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if is_voicevox_api_ready(base_url).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    error::VoicevoxNotRunningSnafu {
        url: base_url.to_string(),
    }
    .fail()
}

async fn ensure_voicevox_running(base_url: &str) -> Result<()> {
    if is_voicevox_api_ready(base_url).await {
        eprintln!("  voicevox api already running at {base_url}");
        return Ok(());
    }

    eprintln!("  voicevox api not reachable, starting engine...");
    start_voicevox_engine(base_url)?;
    wait_for_voicevox_ready(base_url, Duration::from_secs(30)).await?;
    eprintln!("  voicevox api ready at {base_url}");
    Ok(())
}

async fn download_voicevox(version: &str) -> Result<()> {
    let url = voicevox_download_url(version);
    let dir = crate::paths::voicevox_dir();
    let tmp = dir.with_extension("tmp");

    eprintln!("  downloading voicevox engine {version}...");
    eprintln!("  url: {url}");

    std::fs::create_dir_all(tmp.parent().expect("parent dir")).context(error::IoSnafu)?;

    let dl_client = crate::http::download_client();

    // Check for a partial download from a previous interrupted attempt.
    let existing_len = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);

    let mut request = dl_client.get(&url);
    if existing_len > 0 {
        eprintln!("  resuming from byte {existing_len}...");
        request = request.header(reqwest::header::RANGE, format!("bytes={existing_len}-"));
    }

    let response = request.send().await.context(error::HttpSnafu)?;
    let status = response.status();

    // 206 = server supports range, resume; 200 = full response, restart.
    let (resumed, total_size) = match status {
        StatusCode::PARTIAL_CONTENT => {
            let remaining = response.content_length().unwrap_or(0);
            (true, existing_len + remaining)
        }
        s if s.is_success() => {
            let total = response.content_length().unwrap_or(0);
            (false, total)
        }
        _ => {
            return Err(error::VoicevoxSnafu {
                message: format!("download failed: HTTP {status}"),
            }
            .build());
        }
    };

    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{bar:40.cyan/blue} {percent}% {bytes}/{total_bytes}  {bytes_per_sec}  ETA {eta}",
            )
            .expect("valid progress bar template")
            .progress_chars("=>-"),
    );

    // Hash the already-downloaded portion so the final checksum covers the
    // entire file, then open the file in the appropriate mode.
    let mut hasher = Sha256::new();
    let mut file = if resumed {
        hash_existing_file(&tmp, &mut hasher)?;
        pb.set_position(existing_len);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&tmp)
            .context(error::IoSnafu)?
    } else {
        std::fs::File::create(&tmp).context(error::IoSnafu)?
    };

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context(error::HttpSnafu)?;
        hasher.update(&chunk);
        file.write_all(&chunk).context(error::IoSnafu)?;
        pb.inc(chunk.len() as u64);
    }

    pb.finish_and_clear();

    // Verify the download against the SHA256 sidecar file
    let actual_hash = format!("{:x}", hasher.finalize());
    verify_against_sidecar(crate::http::client(), &url, &actual_hash, &tmp).await?;

    // Extract .vvpp archive (zip format)
    let archive_file = std::fs::File::open(&tmp).context(error::IoSnafu)?;
    let mut archive = zip::ZipArchive::new(archive_file).map_err(|e| {
        error::ZipSnafu {
            message: format!("failed to open archive: {e}"),
        }
        .build()
    })?;

    std::fs::create_dir_all(&dir).context(error::IoSnafu)?;
    archive.extract(&dir).map_err(|e| {
        error::ZipSnafu {
            message: format!("failed to extract archive: {e}"),
        }
        .build()
    })?;

    let _ = std::fs::remove_file(&tmp);

    // Make run executable on unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let run = dir.join("run");
        if run.exists() {
            let mut perms = std::fs::metadata(&run)
                .context(error::IoSnafu)?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&run, perms).context(error::IoSnafu)?;
        }
    }

    eprintln!("  voicevox engine installed at {}", dir.display());
    Ok(())
}

/// Feed the contents of an existing partial file into a hasher.
fn hash_existing_file(path: &std::path::Path, hasher: &mut Sha256) -> Result<()> {
    let mut file = std::fs::File::open(path).context(error::IoSnafu)?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).context(error::IoSnafu)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(())
}

/// Download the `.txt` sidecar from GitHub releases and compare its hash
/// against the computed hash. Deletes the file on mismatch.
async fn verify_against_sidecar(
    client: &reqwest::Client,
    asset_url: &str,
    actual_hash: &str,
    downloaded_file: &std::path::Path,
) -> Result<()> {
    let sidecar_url = format!("{asset_url}.txt");
    eprintln!("  verifying checksum...");

    let sidecar_resp = client
        .get(&sidecar_url)
        .send()
        .await
        .context(error::HttpSnafu)?;

    if !sidecar_resp.status().is_success() {
        eprintln!(
            "  warning: checksum sidecar not available (HTTP {}), skipping verification",
            sidecar_resp.status()
        );
        return Ok(());
    }

    let sidecar_text = sidecar_resp.text().await.context(error::HttpSnafu)?;

    let Some(expected_hash) = parse_checksum_sidecar(&sidecar_text) else {
        eprintln!(
            "  warning: sidecar does not contain a valid SHA-256 hash, skipping verification"
        );
        return Ok(());
    };

    verify_checksum(actual_hash, &expected_hash).inspect_err(|_| {
        // Delete the corrupted download before returning the error
        let _ = std::fs::remove_file(downloaded_file);
    })
}

/// Parse a checksum sidecar file, extracting the hex hash.
///
/// Handles two common formats:
/// - Just the hex hash on a line
/// - `<hash>  <filename>` (BSD/GNU coreutils style)
///
/// Returns `None` if no valid SHA-256 hex hash (64 hex chars) is found,
/// e.g. when the sidecar contains only a filename.
fn parse_checksum_sidecar(content: &str) -> Option<String> {
    content
        .split_whitespace()
        .find(|token| token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_lowercase)
}

/// Compare a computed hash against an expected hash string.
fn verify_checksum(actual: &str, expected: &str) -> Result<()> {
    if actual == expected {
        eprintln!("  checksum verified OK");
        Ok(())
    } else {
        error::ChecksumMismatchSnafu {
            expected: expected.to_string(),
            actual:   actual.to_string(),
        }
        .fail()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_checksum_match() {
        let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(verify_checksum(hash, hash).is_ok());
    }

    #[test]
    fn verify_checksum_mismatch() {
        let actual = "aaaa";
        let expected = "bbbb";
        let err = verify_checksum(actual, expected).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("aaaa"), "error should contain actual hash");
        assert!(msg.contains("bbbb"), "error should contain expected hash");
    }

    #[test]
    fn verify_checksum_with_known_sha256() {
        // SHA256 of empty input
        use sha2::{Digest, Sha256};
        let hash = format!("{:x}", Sha256::digest(b""));
        let expected = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(verify_checksum(&hash, expected).is_ok());
    }

    #[test]
    fn parse_sidecar_hash_only() {
        let content = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n";
        assert_eq!(
            parse_checksum_sidecar(content).as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
    }

    #[test]
    fn parse_sidecar_with_filename() {
        let content = "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855  \
                       voicevox_engine-macos-arm64-0.22.2.vvpp\n";
        assert_eq!(
            parse_checksum_sidecar(content).as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
    }

    #[test]
    fn parse_sidecar_filename_only() {
        // VOICEVOX sidecar contains only the filename, no hash
        let content = "voicevox_engine-macos-arm64-0.22.2.vvpp\n";
        assert_eq!(parse_checksum_sidecar(content), None);
    }

    #[test]
    fn parse_voicevox_bind_addr_with_explicit_port() {
        let (host, port) = parse_voicevox_bind_addr("http://127.0.0.1:50021").unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 50021);
    }

    #[test]
    fn parse_voicevox_bind_addr_with_default_http_port() {
        let (host, port) = parse_voicevox_bind_addr("http://localhost").unwrap();
        assert_eq!(host, "localhost");
        assert_eq!(port, 80);
    }

    #[test]
    fn parse_voicevox_bind_addr_rejects_invalid_url() {
        let err = parse_voicevox_bind_addr("localhost:50021").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("invalid VOICEVOX URL") || msg.contains("does not include a host"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn apply_default_voice_preset_sets_hanazawa_voice_and_slower_speed() {
        let mut cfg = AppConfig::default();
        apply_default_voice_preset(
            &mut cfg,
            "by swapno Nakano Ichika (CV Hanazawa Kana ) From - The Quintessential Quintuplets \
             300 Epochs (RVC v2).zip",
        );

        assert_eq!(cfg.voice.active, "kokoro:jf_alpha");
        assert_eq!(
            cfg.rvc.model,
            "by swapno Nakano Ichika (CV Hanazawa Kana ) From - The Quintessential Quintuplets \
             300 Epochs (RVC v2).zip"
        );
        assert!(
            (cfg.voice.speed - 0.90).abs() < f64::EPSILON,
            "expected setup default speed to be slower (0.90), got {}",
            cfg.voice.speed
        );
    }
}
