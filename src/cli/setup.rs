//! `kotoba setup` — download VOICEVOX Engine and initialize environment.

use std::{io::Write, path::PathBuf};

use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use sha2::{Digest, Sha256};
use snafu::ResultExt;

use crate::{
    db::Database,
    error::{self, Result},
};

const VOICEVOX_VERSION: &str = "0.22.2";

/// Result of running the setup command.
#[derive(Debug, Serialize)]
pub struct SetupResult {
    /// Path to the database file.
    pub db_path:            String,
    /// Whether VOICEVOX Engine is installed after setup.
    pub voicevox_installed: bool,
}

fn voicevox_dir() -> Result<PathBuf> {
    let dir = dirs::home_dir()
        .ok_or_else(|| error::HomeNotFoundSnafu.build())?
        .join(".kotoba")
        .join("voicevox");
    Ok(dir)
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
pub fn is_voicevox_installed() -> Result<bool> {
    let dir = voicevox_dir()?;
    Ok(dir.exists() && dir.join("run").exists())
}

/// Get the path to the VOICEVOX Engine executable.
pub fn voicevox_executable() -> Result<PathBuf> { Ok(voicevox_dir()?.join("run")) }

/// Run full setup: download VOICEVOX Engine + initialize DB.
pub async fn run(db: &Database) -> Result<SetupResult> {
    eprintln!("initializing database...");
    db.init().await?;
    eprintln!("  database ready at {}", db.path().display());

    let version = db
        .get_config("voicevox_version")
        .await?
        .unwrap_or_else(|| VOICEVOX_VERSION.to_string());

    if is_voicevox_installed()? {
        eprintln!("  voicevox engine already installed");
    } else {
        download_voicevox(&version).await?;
    }

    // Only set voice defaults if not already configured, so re-running setup
    // does not overwrite user customizations.
    if db.get_config("tts_backend").await?.is_none() {
        db.set_config("tts_backend", "voicevox").await?;
    }
    if db.get_config("voicevox_speaker").await?.is_none() {
        db.set_config("voicevox_speaker", "1").await?;
    }

    eprintln!("setup complete!");

    Ok(SetupResult {
        db_path:            db.path().display().to_string(),
        voicevox_installed: is_voicevox_installed()?,
    })
}

async fn download_voicevox(version: &str) -> Result<()> {
    let url = voicevox_download_url(version);
    let dir = voicevox_dir()?;

    eprintln!("  downloading voicevox engine {version}...");
    eprintln!("  url: {url}");

    let client = reqwest::Client::new();
    let response = client.get(&url).send().await.context(error::HttpSnafu)?;

    if !response.status().is_success() {
        return Err(error::VoicevoxSnafu {
            message: format!("download failed: HTTP {}", response.status()),
        }
        .build());
    }

    let total_size = response.content_length().unwrap_or(0);

    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{bar:40.cyan/blue} {percent}% {bytes}/{total_bytes}  {bytes_per_sec}  ETA {eta}",
            )
            .expect("valid progress bar template")
            .progress_chars("=>-"),
    );

    let tmp = dir.with_extension("tmp");
    std::fs::create_dir_all(tmp.parent().expect("parent dir")).context(error::IoSnafu)?;

    // Stream the response body to disk in chunks, computing SHA256 as we go.
    let mut stream = response.bytes_stream();
    let mut file = std::fs::File::create(&tmp).context(error::IoSnafu)?;
    let mut hasher = Sha256::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context(error::HttpSnafu)?;
        hasher.update(&chunk);
        file.write_all(&chunk).context(error::IoSnafu)?;
        pb.inc(chunk.len() as u64);
    }

    pb.finish_and_clear();

    // Verify the download against the SHA256 sidecar file
    let actual_hash = format!("{:x}", hasher.finalize());
    verify_against_sidecar(&client, &url, &actual_hash, &tmp).await?;

    // Extract .vvpp archive (zip format)
    let file = std::fs::File::open(&tmp).context(error::IoSnafu)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        error::VoicevoxSnafu {
            message: format!("failed to open archive: {e}"),
        }
        .build()
    })?;

    std::fs::create_dir_all(&dir).context(error::IoSnafu)?;
    archive.extract(&dir).map_err(|e| {
        error::VoicevoxSnafu {
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

/// Download the `.txt` sidecar from GitHub releases and compare its hash
/// against the computed hash. Deletes the file on mismatch.
async fn verify_against_sidecar(
    client: &reqwest::Client,
    asset_url: &str,
    actual_hash: &str,
    downloaded_file: &PathBuf,
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
    let expected_hash = parse_checksum_sidecar(&sidecar_text);

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
fn parse_checksum_sidecar(content: &str) -> String {
    let trimmed = content.trim();
    // If the line contains whitespace, the hash is the first token
    trimmed
        .split_whitespace()
        .next()
        .unwrap_or(trimmed)
        .to_lowercase()
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
            parse_checksum_sidecar(content),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn parse_sidecar_with_filename() {
        let content = "E3B0C44298FC1C149AFBF4C8996FB924  voicevox_engine-macos-arm64-0.22.2.vvpp\n";
        assert_eq!(
            parse_checksum_sidecar(content),
            "e3b0c44298fc1c149afbf4c8996fb924"
        );
    }
}
