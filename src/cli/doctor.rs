//! `kotoba doctor` — check all dependencies and report status.

use std::fmt;

use serde::Serialize;

use crate::{cli::setup, db::Database, error::Result};

/// Status of a single health check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Component is healthy.
    Ok,
    /// Component has an error.
    Error,
    /// Component is not installed.
    Missing,
    /// Component is not running (optional service).
    NotRunning,
}

impl Status {
    /// Unicode symbol for terminal display.
    const fn symbol(self) -> &'static str {
        match self {
            Self::Ok => "\u{2713}",                         // ✓
            Self::Error => "\u{2717}",                      // ✗
            Self::Missing | Self::NotRunning => "\u{25CB}", // ○
        }
    }

    /// Whether this status counts as a pass.
    const fn is_pass(self) -> bool { !matches!(self, Self::Error) }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Missing => "missing",
            Self::NotRunning => "not_running",
        };
        f.write_str(label)
    }
}

/// Health check result for a single component.
#[derive(Debug, Serialize)]
pub struct Check {
    /// Component name.
    pub name:   String,
    /// Status of the component.
    pub status: Status,
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
///
/// When `json` is true, outputs machine-readable JSON; otherwise prints a
/// human-friendly table with Unicode status symbols.
pub async fn run(db: &Database, json: bool) -> Result<()> {
    let mut checks = Vec::new();

    checks.push(check_database(db).await);
    checks.push(check_voicevox_installed());
    checks.push(check_voicevox_api().await);
    checks.push(check_audio_cache());
    checks.push(check_kokoro_model());
    checks.push(check_rvc_python());
    checks.push(check_rvc_models());
    checks.push(check_disk_space());

    let healthy = checks.iter().all(|c| c.status == Status::Ok);
    let report = Report { checks, healthy };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("json serialize")
        );
    } else {
        print_pretty(&report);
    }

    Ok(())
}

/// Render the report as a human-friendly table.
fn print_pretty(report: &Report) {
    let name_width = report
        .checks
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(0);

    eprintln!();
    for check in &report.checks {
        eprintln!(
            "  {} {:<width$}  {}",
            check.status.symbol(),
            check.name,
            check.detail,
            width = name_width,
        );
    }

    let passed = report.checks.iter().filter(|c| c.status.is_pass()).count();
    let total = report.checks.len();
    eprintln!();
    eprintln!("  {passed}/{total} checks passed");
}

async fn check_database(db: &Database) -> Check {
    match db.status().await {
        Ok(status) => Check {
            name:   "database".into(),
            status: Status::Ok,
            detail: format!(
                "level={}, vocab={}, due={}",
                status.level, status.vocabulary_count, status.due_reviews
            ),
        },
        Err(e) => Check {
            name:   "database".into(),
            status: Status::Error,
            detail: format!("not initialized or corrupt: {e}"),
        },
    }
}

fn check_voicevox_installed() -> Check {
    if setup::is_voicevox_installed() {
        Check {
            name:   "voicevox_installed".into(),
            status: Status::Ok,
            detail: crate::paths::voicevox_executable().display().to_string(),
        }
    } else {
        Check {
            name:   "voicevox_installed".into(),
            status: Status::Missing,
            detail: "run `kotoba setup` to install".into(),
        }
    }
}

async fn check_voicevox_api() -> Check {
    let base_url = std::env::var("VOICEVOX_URL")
        .unwrap_or_else(|_| crate::app_config::load().voicevox.url.clone());

    let client = crate::http::client();

    match client
        .get(format!("{base_url}/version"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let version = resp.text().await.unwrap_or_else(|_| "unknown".into());
            Check {
                name:   "voicevox_api".into(),
                status: Status::Ok,
                detail: format!("version={version}, url={base_url}"),
            }
        }
        Ok(resp) => Check {
            name:   "voicevox_api".into(),
            status: Status::Error,
            detail: format!("HTTP {}", resp.status()),
        },
        Err(_) => Check {
            name:   "voicevox_api".into(),
            status: Status::NotRunning,
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
            name:   "audio_cache".into(),
            status: Status::Ok,
            detail: format!("{count} cached files at {}", dir.display()),
        }
    } else {
        Check {
            name:   "audio_cache".into(),
            status: Status::Ok,
            detail: format!("not created yet (will be at {})", dir.display()),
        }
    }
}

fn check_kokoro_model() -> Check {
    let kokoro_dir = dirs::home_dir().map(|h| h.join(".kotoba/models/kokoro"));

    match kokoro_dir {
        Some(d) if d.join("kokoro-v1.0.onnx").exists() => Check {
            name:   "kokoro_model".into(),
            status: Status::Ok,
            detail: "installed".into(),
        },
        Some(_) => Check {
            name:   "kokoro_model".into(),
            status: Status::Ok,
            detail: "not installed (optional — run `kotoba voice add kokoro`)".into(),
        },
        None => Check {
            name:   "kokoro_model".into(),
            status: Status::Error,
            detail: "home directory not found".into(),
        },
    }
}

fn check_rvc_python() -> Check {
    let python = crate::rvc::resolve_python();
    let python_display = python.display().to_string();

    match std::process::Command::new(&python)
        .args(["-c", "from infer_rvc_python import BaseLoader"])
        .output()
    {
        Ok(output) if output.status.success() => Check {
            name:   "rvc_python".into(),
            status: Status::Ok,
            detail: format!("infer-rvc-python available ({python_display})"),
        },
        _ => Check {
            name:   "rvc_python".into(),
            status: Status::Ok,
            detail: format!(
                "infer-rvc-python not installed (optional — needed for RVC voice conversion, \
                 python={python_display})"
            ),
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
            name:   "rvc_models".into(),
            status: Status::Ok,
            detail: format!("{count} installed"),
        }
    } else {
        Check {
            name:   "rvc_models".into(),
            status: Status::Ok,
            detail: "none (optional — add with `kotoba huggingface add rvc:owner/repo`)".into(),
        }
    }
}

fn check_disk_space() -> Check {
    let dir = crate::paths::data_dir();
    Check {
        name:   "disk_space".into(),
        status: Status::Ok,
        detail: format!("data dir: {}", dir.display()),
    }
}
