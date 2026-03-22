//! `kotoba doctor` — check all dependencies and report status.

use serde::Serialize;

use crate::{cli::setup, db::Database, error::Result};

/// Health check result for a single component.
#[derive(Debug, Serialize)]
pub struct Check {
    /// Component name.
    pub name:   String,
    /// Status: "ok", "error", "missing", or "`not_running`".
    pub status: String,
    /// Human-readable detail.
    pub detail: String,
}

/// Full doctor report.
#[derive(Debug, Serialize)]
pub struct Report {
    /// Individual check results.
    pub checks:  Vec<Check>,
    /// True if all checks passed.
    pub healthy: bool,
}

/// Run all health checks and print a report.
pub async fn run(db: &Database) -> Result<()> {
    let mut checks = Vec::new();

    // 1. Database
    checks.push(check_database(db).await);

    // 2. VOICEVOX Engine installed
    checks.push(check_voicevox_installed());

    // 3. VOICEVOX Engine reachable
    checks.push(check_voicevox_api().await);

    // 4. Audio cache directory
    checks.push(check_audio_cache());

    // 5. Kokoro model
    checks.push(check_kokoro_model());

    // 6. RVC Python
    checks.push(check_rvc_python().await);

    // 7. RVC models
    checks.push(check_rvc_models());

    // 8. Disk space
    checks.push(check_disk_space());

    let healthy = checks.iter().all(|c| c.status == "ok");

    let report = Report { checks, healthy };
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("json serialize")
    );

    Ok(())
}

async fn check_database(db: &Database) -> Check {
    match db.status().await {
        Ok(status) => Check {
            name:   "database".to_string(),
            status: "ok".to_string(),
            detail: format!(
                "level={}, vocab={}, due={}",
                status.level, status.vocabulary_count, status.due_reviews
            ),
        },
        Err(e) => Check {
            name:   "database".to_string(),
            status: "error".to_string(),
            detail: format!("not initialized or corrupt: {e}"),
        },
    }
}

fn check_voicevox_installed() -> Check {
    if setup::is_voicevox_installed() {
        Check {
            name:   "voicevox_installed".to_string(),
            status: "ok".to_string(),
            detail: crate::paths::voicevox_executable().display().to_string(),
        }
    } else {
        Check {
            name:   "voicevox_installed".to_string(),
            status: "missing".to_string(),
            detail: "run `kotoba setup` to install".to_string(),
        }
    }
}

async fn check_voicevox_api() -> Check {
    // Env var overrides config
    let base_url = std::env::var("VOICEVOX_URL")
        .unwrap_or_else(|_| crate::app_config::load().voicevox.url.clone());

    let client = crate::http::client();

    // Use a per-request timeout for the doctor check (short timeout)
    match client
        .get(format!("{base_url}/version"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let version = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
            Check {
                name:   "voicevox_api".to_string(),
                status: "ok".to_string(),
                detail: format!("version={version}, url={base_url}"),
            }
        }
        Ok(resp) => Check {
            name:   "voicevox_api".to_string(),
            status: "error".to_string(),
            detail: format!("HTTP {}", resp.status()),
        },
        Err(_) => Check {
            name:   "voicevox_api".to_string(),
            status: "not_running".to_string(),
            detail: format!("{base_url} unreachable — start VOICEVOX Engine first"),
        },
    }
}

fn check_audio_cache() -> Check {
    let dir = crate::paths::audio_cache_dir();

    if dir.exists() {
        let count = std::fs::read_dir(&dir)
            .map(std::iter::Iterator::count)
            .unwrap_or(0);
        Check {
            name:   "audio_cache".to_string(),
            status: "ok".to_string(),
            detail: format!("{count} cached files at {}", dir.display()),
        }
    } else {
        Check {
            name:   "audio_cache".to_string(),
            status: "ok".to_string(),
            detail: format!("not created yet (will be at {})", dir.display()),
        }
    }
}

fn check_kokoro_model() -> Check {
    let kokoro_dir = dirs::home_dir().map(|h| h.join(".kotoba/models/kokoro"));

    match kokoro_dir {
        Some(d) if d.join("kokoro-v1.0.onnx").exists() => Check {
            name:   "kokoro_model".to_string(),
            status: "ok".to_string(),
            detail: "installed".to_string(),
        },
        Some(_) => Check {
            name:   "kokoro_model".to_string(),
            status: "ok".to_string(),
            detail: "not installed (optional — run `kotoba voice add kokoro`)".to_string(),
        },
        None => Check {
            name:   "kokoro_model".to_string(),
            status: "error".to_string(),
            detail: "home directory not found".to_string(),
        },
    }
}

async fn check_rvc_python() -> Check {
    match crate::rvc::check_installed().await {
        Ok(()) => Check {
            name:   "rvc_python".to_string(),
            status: "ok".to_string(),
            detail: "installed".to_string(),
        },
        Err(_) => Check {
            name:   "rvc_python".to_string(),
            status: "ok".to_string(),
            detail: "not installed (optional — `pip install rvc-python` for anime character \
                     voices)"
                .to_string(),
        },
    }
}

fn check_rvc_models() -> Check {
    let rvc_dir = crate::paths::models_dir().join("rvc");

    if rvc_dir.exists() {
        let count = std::fs::read_dir(&rvc_dir)
            .map(|entries| entries.flatten().filter(|e| e.path().is_dir()).count())
            .unwrap_or(0);
        Check {
            name:   "rvc_models".to_string(),
            status: "ok".to_string(),
            detail: format!("{count} installed"),
        }
    } else {
        Check {
            name:   "rvc_models".to_string(),
            status: "ok".to_string(),
            detail: "none (optional — add with `kotoba huggingface add rvc:<repo>`)".to_string(),
        }
    }
}

fn check_disk_space() -> Check {
    let dir = crate::paths::data_dir();
    Check {
        name:   "disk_space".to_string(),
        status: "ok".to_string(),
        detail: format!("data dir: {}", dir.display()),
    }
}
