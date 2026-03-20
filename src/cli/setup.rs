//! `kotoba setup` — download VOICEVOX Engine and initialize environment.

use std::{io::Write, path::PathBuf};

use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
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

fn voicevox_download_url() -> String {
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
        "https://github.com/VOICEVOX/voicevox_engine/releases/download/{VOICEVOX_VERSION}/voicevox_engine-{platform}-{VOICEVOX_VERSION}.vvpp"
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

    if is_voicevox_installed()? {
        eprintln!("  voicevox engine already installed");
    } else {
        download_voicevox().await?;
    }

    db.set_config("tts_backend", "voicevox").await?;
    db.set_config("voicevox_speaker", "1").await?;

    eprintln!("setup complete!");

    Ok(SetupResult {
        db_path:            db.path().display().to_string(),
        voicevox_installed: is_voicevox_installed()?,
    })
}

async fn download_voicevox() -> Result<()> {
    let url = voicevox_download_url();
    let dir = voicevox_dir()?;

    eprintln!("  downloading voicevox engine {VOICEVOX_VERSION}...");
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

    // Stream the response body to disk in chunks instead of buffering the
    // entire archive in memory.
    let mut stream = response.bytes_stream();
    let mut file = std::fs::File::create(&tmp).context(error::IoSnafu)?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context(error::HttpSnafu)?;
        file.write_all(&chunk).context(error::IoSnafu)?;
        pb.inc(chunk.len() as u64);
    }

    pb.finish_and_clear();

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
