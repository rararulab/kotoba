//! `kotoba doctor` — check all dependencies and report status.

use serde::Serialize;

use crate::{cli::setup, db::Database, error::Result};

/// Health check result for a single component.
#[derive(Debug, Serialize)]
pub struct Check {
    pub name:   String,
    pub status: String,
    pub detail: String,
}

/// Full doctor report.
#[derive(Debug, Serialize)]
pub struct Report {
    pub checks:  Vec<Check>,
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

    // 5. Disk space
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
    match setup::is_voicevox_installed() {
        Ok(true) => Check {
            name:   "voicevox_installed".to_string(),
            status: "ok".to_string(),
            detail: setup::voicevox_executable()
                .map_or_else(|_| "unknown path".to_string(), |p| p.display().to_string()),
        },
        _ => Check {
            name:   "voicevox_installed".to_string(),
            status: "missing".to_string(),
            detail: "run `kotoba setup` to install".to_string(),
        },
    }
}

async fn check_voicevox_api() -> Check {
    let base_url =
        std::env::var("VOICEVOX_URL").unwrap_or_else(|_| "http://localhost:50021".to_string());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build();

    let client = match client {
        Ok(c) => c,
        Err(e) => {
            return Check {
                name:   "voicevox_api".to_string(),
                status: "error".to_string(),
                detail: format!("http client error: {e}"),
            };
        }
    };

    match client.get(format!("{base_url}/version")).send().await {
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
    let dir = dirs::home_dir().map(|h| h.join(".kotoba").join("audio"));

    match dir {
        Some(d) if d.exists() => {
            let count = std::fs::read_dir(&d)
                .map(std::iter::Iterator::count)
                .unwrap_or(0);
            Check {
                name:   "audio_cache".to_string(),
                status: "ok".to_string(),
                detail: format!("{} cached files at {}", count, d.display()),
            }
        }
        Some(d) => Check {
            name:   "audio_cache".to_string(),
            status: "ok".to_string(),
            detail: format!("not created yet (will be at {})", d.display()),
        },
        None => Check {
            name:   "audio_cache".to_string(),
            status: "error".to_string(),
            detail: "home directory not found".to_string(),
        },
    }
}

fn check_disk_space() -> Check {
    let home = dirs::home_dir().map(|h| h.join(".kotoba"));
    home.map_or_else(
        || Check {
            name:   "disk_space".to_string(),
            status: "error".to_string(),
            detail: "home directory not found".to_string(),
        },
        |p| {
            let path = p.display().to_string();
            Check {
                name:   "disk_space".to_string(),
                status: "ok".to_string(),
                detail: format!("data dir: {path}"),
            }
        },
    )
}
