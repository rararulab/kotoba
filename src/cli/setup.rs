//! `kotoba setup` — download VOICEVOX Engine and initialize environment.

use std::path::PathBuf;

use snafu::ResultExt;

use crate::{
    db::Database,
    error::{self, Result},
};

const VOICEVOX_VERSION: &str = "0.22.2";

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
pub async fn run(db: &Database) -> Result<()> {
    println!("initializing database...");
    db.init().await?;
    println!("  database ready at {}", db.path().display());

    if is_voicevox_installed()? {
        println!("  voicevox engine already installed");
    } else {
        download_voicevox().await?;
    }

    db.set_config("tts_backend", "voicevox").await?;
    db.set_config("voicevox_speaker", "1").await?;

    println!("setup complete!");
    Ok(())
}

async fn download_voicevox() -> Result<()> {
    let url = voicevox_download_url();
    let dir = voicevox_dir()?;

    println!("  downloading voicevox engine {VOICEVOX_VERSION}...");
    println!("  url: {url}");

    let client = reqwest::Client::new();
    let response = client.get(&url).send().await.context(error::HttpSnafu)?;

    if !response.status().is_success() {
        return Err(error::VoicevoxSnafu {
            message: format!("download failed: HTTP {}", response.status()),
        }
        .build());
    }

    let bytes = response.bytes().await.context(error::HttpSnafu)?;

    let tmp = dir.with_extension("tmp");
    std::fs::create_dir_all(tmp.parent().expect("parent dir")).context(error::IoSnafu)?;
    std::fs::write(&tmp, &bytes).context(error::IoSnafu)?;

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

    println!("  voicevox engine installed at {}", dir.display());
    Ok(())
}
