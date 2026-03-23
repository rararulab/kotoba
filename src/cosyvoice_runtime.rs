//! `CosyVoice` runtime lifecycle helpers (probe, infer command, autostart).

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use snafu::{ResultExt, prelude::*};

use crate::{
    app_config::CosyvoiceConfig,
    error::{self, Result},
};

const DEFAULT_MODEL_DIR: &str = "iic/CosyVoice2-0.5B";
const HOST_TOKEN: &str = "{host}";
const PORT_TOKEN: &str = "{port}";
const URL_TOKEN: &str = "{url}";

#[derive(Debug, Clone, Copy)]
pub enum CommandSource {
    Env,
    Config,
    Inferred,
}

#[derive(Debug, Clone)]
pub struct ResolvedCommand {
    pub source:   CommandSource,
    pub template: String,
}

pub fn base_url(cfg: &CosyvoiceConfig) -> String {
    std::env::var("COSYVOICE_URL").unwrap_or_else(|_| cfg.url.clone())
}

pub async fn is_api_ready(base_url: &str) -> bool {
    let url = base_url.trim_end_matches('/');
    if url.is_empty() {
        return false;
    }

    crate::http::client()
        .get(url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .is_ok()
}

pub fn parse_bind_addr(base_url: &str) -> Result<(String, u16)> {
    let url = reqwest::Url::parse(base_url).map_err(|source| {
        error::CosyvoiceSnafu {
            message: format!("invalid cosyvoice.url `{base_url}`: {source}"),
        }
        .build()
    })?;

    let host = url.host_str().ok_or_else(|| {
        error::CosyvoiceSnafu {
            message: format!("cosyvoice.url `{base_url}` does not include a host"),
        }
        .build()
    })?;
    let port = url.port_or_known_default().ok_or_else(|| {
        error::CosyvoiceSnafu {
            message: format!("cosyvoice.url `{base_url}` does not include a valid port"),
        }
        .build()
    })?;
    Ok((host.to_string(), port))
}

pub fn resolve_command(cfg: &CosyvoiceConfig) -> Option<ResolvedCommand> {
    if let Ok(cmd) = std::env::var("COSYVOICE_CMD") {
        let trimmed = cmd.trim();
        if !trimmed.is_empty() {
            return Some(ResolvedCommand {
                source:   CommandSource::Env,
                template: trimmed.to_string(),
            });
        }
    }

    let configured = cfg.command.trim();
    if !configured.is_empty() {
        return Some(ResolvedCommand {
            source:   CommandSource::Config,
            template: configured.to_string(),
        });
    }

    infer_command().map(|template| ResolvedCommand {
        source: CommandSource::Inferred,
        template,
    })
}

pub fn infer_command() -> Option<String> {
    let script = infer_server_script()?;
    let model_dir = std::env::var("COSYVOICE_MODEL_DIR")
        .unwrap_or_else(|_| DEFAULT_MODEL_DIR.to_string())
        .trim()
        .to_string();
    if model_dir.is_empty() {
        return None;
    }

    Some(format!(
        "python3 {} --host {HOST_TOKEN} --port {PORT_TOKEN} --model_dir {}",
        shell_quote(script.to_string_lossy().as_ref()),
        shell_quote(&model_dir),
    ))
}

pub fn expand_command(template: &str, host: &str, port: u16, url: &str) -> String {
    template
        .replace(HOST_TOKEN, host)
        .replace(PORT_TOKEN, &port.to_string())
        .replace(URL_TOKEN, url)
}

pub async fn ensure_running(cfg: &CosyvoiceConfig) -> Result<()> {
    let base_url = base_url(cfg);
    let base_url = base_url.trim().to_string();
    ensure!(
        !base_url.is_empty(),
        error::CosyvoiceSnafu {
            message: "cosyvoice.url is empty".to_string(),
        }
    );

    if is_api_ready(&base_url).await {
        return Ok(());
    }

    ensure!(
        cfg.autostart,
        error::CosyvoiceSnafu {
            message: format!(
                "CosyVoice runtime not reachable at {base_url} and cosyvoice.autostart=false"
            ),
        }
    );

    let resolved = resolve_command(cfg).ok_or_else(|| {
        error::CosyvoiceSnafu {
            message: format!(
                "CosyVoice runtime not reachable at {base_url}. No launch command found. Set \
                 cosyvoice.command or COSYVOICE_CMD, or place CosyVoice at a default location \
                 (~/CosyVoice or ~/.kotoba/cosyvoice/CosyVoice)."
            ),
        }
        .build()
    })?;

    if matches!(resolved.source, CommandSource::Inferred) {
        eprintln!("  detected cosyvoice runtime command from local installation");
    }
    eprintln!("  cosyvoice runtime not reachable, starting...");
    start_runtime(&base_url, &resolved.template)?;
    wait_for_ready(&base_url, Duration::from_secs(90)).await?;
    eprintln!("  cosyvoice runtime ready at {base_url}");
    Ok(())
}

fn start_runtime(base_url: &str, command_template: &str) -> Result<()> {
    let (host, port) = parse_bind_addr(base_url)?;
    let command_line = expand_command(command_template, &host, port, base_url);
    let log_path = crate::paths::cosyvoice_log_file();

    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).context(error::IoSnafu)?;
    }
    let stdout_log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .context(error::IoSnafu)?;
    let stderr_log = stdout_log.try_clone().context(error::IoSnafu)?;

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(&command_line);
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut command = {
        let mut c = Command::new("sh");
        c.arg("-lc").arg(&command_line);
        c
    };

    command
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(stderr_log))
        .spawn()
        .context(error::IoSnafu)?;

    eprintln!(
        "  started cosyvoice runtime on {host}:{port} (logs: {})",
        log_path.display()
    );

    Ok(())
}

async fn wait_for_ready(base_url: &str, timeout: Duration) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if is_api_ready(base_url).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    error::CosyvoiceSnafu {
        message: format!("CosyVoice runtime did not become ready at {base_url} in time"),
    }
    .fail()
}

fn infer_server_script() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(home) = std::env::var("COSYVOICE_HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join("runtime/python/fastapi/server.py"));
        candidates.push(home.join("CosyVoice/runtime/python/fastapi/server.py"));
    }

    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("CosyVoice/runtime/python/fastapi/server.py"));
        candidates.push(home.join(".kotoba/cosyvoice/CosyVoice/runtime/python/fastapi/server.py"));
        candidates.push(home.join("code/CosyVoice/runtime/python/fastapi/server.py"));
        candidates.push(home.join("workspace/CosyVoice/runtime/python/fastapi/server.py"));
    }

    candidates.push(PathBuf::from(
        "/opt/CosyVoice/runtime/python/fastapi/server.py",
    ));

    candidates.into_iter().find(|path| path_exists(path))
}

fn path_exists(path: &Path) -> bool { std::fs::metadata(path).is_ok_and(|meta| meta.is_file()) }

fn shell_quote(value: &str) -> String {
    let escaped = value.replace('\'', r#"'"'"'"#);
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_command_replaces_tokens() {
        let template = "python server.py --host {host} --port {port} --url {url}";
        let expanded = expand_command(template, "127.0.0.1", 50000, "http://127.0.0.1:50000");
        assert_eq!(
            expanded,
            "python server.py --host 127.0.0.1 --port 50000 --url http://127.0.0.1:50000"
        );
    }

    #[test]
    fn parse_bind_addr_accepts_explicit_port() {
        let (host, port) = parse_bind_addr("http://127.0.0.1:50000").expect("parse url");
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 50000);
    }
}
