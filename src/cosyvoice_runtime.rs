//! `CosyVoice` runtime lifecycle helpers (bootstrap, probe, autostart).

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

use snafu::{ResultExt, ensure};

use crate::{
    app_config::CosyvoiceConfig,
    error::{self, Result},
};

const COSYVOICE_REPO_URL: &str = "https://github.com/FunAudioLLM/CosyVoice.git";
const DEFAULT_MODEL_DIR: &str = "iic/CosyVoice2-0.5B";
const DEFAULT_STARTUP_TIMEOUT_SECS: u64 = 1_800;
const HOST_TOKEN: &str = "{host}";
const PORT_TOKEN: &str = "{port}";
const URL_TOKEN: &str = "{url}";

#[derive(Debug, Clone, Copy)]
pub enum CommandSource {
    Env,
    Config,
    Inferred,
    Bootstrapped,
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
                template: normalize_command_template(trimmed),
            });
        }
    }

    let configured = cfg.command.trim();
    if !configured.is_empty() {
        return Some(ResolvedCommand {
            source:   CommandSource::Config,
            template: normalize_command_template(configured),
        });
    }

    infer_command().map(|template| ResolvedCommand {
        source: CommandSource::Inferred,
        template,
    })
}

pub fn infer_command() -> Option<String> {
    let script = infer_server_script()?;
    let model_dir = resolved_model_dir()?;
    let python_bin = inferred_python_bin(&script);

    Some(normalize_command_template(&build_command_template(
        &python_bin,
        &script,
        &model_dir,
    )))
}

pub fn bootstrap_managed_install() -> Result<String> {
    let repo_dir = crate::paths::cosyvoice_repo_dir();
    ensure_repo_checkout(&repo_dir)?;
    let python = ensure_managed_python_env(&repo_dir)?;
    let script = crate::paths::cosyvoice_server_script();
    ensure!(
        script.is_file(),
        error::CosyvoiceSnafu {
            message: format!(
                "CosyVoice server script missing after bootstrap: {}",
                script.display()
            ),
        }
    );
    let model_dir = resolved_model_dir().ok_or_else(|| {
        error::CosyvoiceSnafu {
            message: "COSYVOICE_MODEL_DIR is empty".to_string(),
        }
        .build()
    })?;

    Ok(normalize_command_template(&build_command_template(
        &python, &script, &model_dir,
    )))
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

    let mut resolved = if let Some(command) = resolve_command(cfg) {
        command
    } else {
        eprintln!("  no cosyvoice command found, bootstrapping managed runtime...");
        ResolvedCommand {
            source:   CommandSource::Bootstrapped,
            template: bootstrap_managed_install()?,
        }
    };

    if matches!(resolved.source, CommandSource::Inferred) {
        eprintln!("  detected cosyvoice runtime command from local installation");
    }

    let mut first_error: Option<error::KotobaError> = None;
    for attempt in 0..2 {
        eprintln!("  cosyvoice runtime not reachable, starting...");
        match start_and_wait(&base_url, &resolved.template).await {
            Ok(()) => {
                eprintln!("  cosyvoice runtime ready at {base_url}");
                return Ok(());
            }
            Err(err) => {
                if first_error.is_none() {
                    first_error = Some(err);
                }

                if attempt == 0 && matches!(resolved.source, CommandSource::Inferred) {
                    eprintln!(
                        "  inferred runtime failed to start; trying managed bootstrap fallback..."
                    );
                    resolved = ResolvedCommand {
                        source:   CommandSource::Bootstrapped,
                        template: bootstrap_managed_install()?,
                    };
                    continue;
                }

                break;
            }
        }
    }

    Err(first_error.unwrap_or_else(|| {
        error::CosyvoiceSnafu {
            message: format!("failed to start CosyVoice runtime at {base_url}"),
        }
        .build()
    }))
}

async fn start_and_wait(base_url: &str, command_template: &str) -> Result<()> {
    let mut child = start_runtime(base_url, command_template)?;
    wait_for_ready(base_url, startup_timeout(), Some(&mut child)).await
}

fn start_runtime(base_url: &str, command_template: &str) -> Result<Child> {
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

    let child = command
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(stderr_log))
        .spawn()
        .context(error::IoSnafu)?;

    eprintln!(
        "  started cosyvoice runtime on {host}:{port} (logs: {})",
        log_path.display()
    );

    Ok(child)
}

async fn wait_for_ready(
    base_url: &str,
    timeout: Duration,
    mut child: Option<&mut Child>,
) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if is_api_ready(base_url).await {
            return Ok(());
        }

        if let Some(proc) = child.as_deref_mut()
            && let Some(status) = proc.try_wait().context(error::IoSnafu)?
        {
            return error::CosyvoiceSnafu {
                message: format!(
                    "CosyVoice runtime exited early with status {status}; see logs: {}",
                    crate::paths::cosyvoice_log_file().display()
                ),
            }
            .fail();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    error::CosyvoiceSnafu {
        message: format!("CosyVoice runtime did not become ready at {base_url} in time"),
    }
    .fail()
}

fn ensure_repo_checkout(repo_dir: &Path) -> Result<()> {
    let server_script = crate::paths::cosyvoice_server_script();
    if server_script.is_file() {
        return Ok(());
    }

    ensure!(
        command_exists("git"),
        error::CosyvoiceSnafu {
            message: "git is required to bootstrap CosyVoice runtime".to_string(),
        }
    );

    let base_dir = crate::paths::cosyvoice_dir();
    std::fs::create_dir_all(&base_dir).context(error::IoSnafu)?;

    if !repo_dir.exists() {
        run_checked(
            Command::new("git")
                .arg("clone")
                .arg("--depth")
                .arg("1")
                .arg("--recursive")
                .arg(COSYVOICE_REPO_URL)
                .arg(repo_dir),
            "clone CosyVoice repository",
        )?;
    } else if repo_dir.join(".git").is_dir() {
        run_checked(
            Command::new("git")
                .arg("-C")
                .arg(repo_dir)
                .arg("submodule")
                .arg("update")
                .arg("--init")
                .arg("--recursive"),
            "initialize CosyVoice submodules",
        )?;
    }

    ensure!(
        server_script.is_file(),
        error::CosyvoiceSnafu {
            message: format!(
                "CosyVoice server script not found after clone: {}",
                server_script.display()
            ),
        }
    );

    Ok(())
}

fn ensure_managed_python_env(repo_dir: &Path) -> Result<String> {
    let venv_dir = crate::paths::cosyvoice_venv_dir();
    if resolve_venv_python().is_none() {
        if let Some(parent) = venv_dir.parent() {
            std::fs::create_dir_all(parent).context(error::IoSnafu)?;
        }

        if command_exists("uv") {
            run_checked(
                Command::new("uv")
                    .arg("venv")
                    .arg("--python")
                    .arg("3.10")
                    .arg(&venv_dir),
                "create CosyVoice venv with uv",
            )?;
        } else {
            ensure!(
                command_exists("python3"),
                error::CosyvoiceSnafu {
                    message: "python3 is required to bootstrap CosyVoice runtime".to_string(),
                }
            );
            run_checked(
                Command::new("python3").arg("-m").arg("venv").arg(&venv_dir),
                "create CosyVoice venv",
            )?;
        }
    }

    let python = resolve_venv_python().ok_or_else(|| {
        error::CosyvoiceSnafu {
            message: format!(
                "managed CosyVoice venv is missing python executable at {}",
                crate::paths::cosyvoice_venv_dir().display()
            ),
        }
        .build()
    })?;
    ensure_pip_available(&python)?;

    let requirements = repo_dir.join("requirements.txt");
    ensure!(
        requirements.is_file(),
        error::CosyvoiceSnafu {
            message: format!(
                "CosyVoice requirements file missing: {}",
                requirements.display()
            ),
        }
    );

    let stamp = crate::paths::cosyvoice_requirements_stamp();
    if !stamp.is_file() {
        if let Err(err) = run_checked(
            Command::new(&python)
                .arg("-m")
                .arg("pip")
                .arg("install")
                .arg("--upgrade")
                .arg("pip"),
            "upgrade pip for CosyVoice venv",
        ) {
            eprintln!("  warning: {err}");
        }

        install_requirements_with_fallback(&python, &requirements)?;

        if let Some(parent) = stamp.parent() {
            std::fs::create_dir_all(parent).context(error::IoSnafu)?;
        }
        std::fs::write(&stamp, "ok\n").context(error::IoSnafu)?;
    }

    Ok(python.to_string_lossy().into_owned())
}

fn resolve_venv_python() -> Option<PathBuf> {
    let primary = crate::paths::cosyvoice_venv_python();
    if path_exists(&primary) {
        return Some(primary);
    }

    if cfg!(windows) {
        None
    } else {
        let fallback = crate::paths::cosyvoice_venv_dir()
            .join("bin")
            .join("python");
        path_exists(&fallback).then_some(fallback)
    }
}

fn infer_server_script() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    candidates.push(crate::paths::cosyvoice_server_script());

    if let Ok(home) = std::env::var("COSYVOICE_HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join("runtime/python/fastapi/server.py"));
        candidates.push(home.join("CosyVoice/runtime/python/fastapi/server.py"));
    }

    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("CosyVoice/runtime/python/fastapi/server.py"));
        candidates.push(home.join("code/CosyVoice/runtime/python/fastapi/server.py"));
        candidates.push(home.join("workspace/CosyVoice/runtime/python/fastapi/server.py"));
    }

    candidates.push(PathBuf::from(
        "/opt/CosyVoice/runtime/python/fastapi/server.py",
    ));

    candidates.into_iter().find(|path| path_exists(path))
}

fn inferred_python_bin(script: &Path) -> String {
    if let Ok(python) = std::env::var("COSYVOICE_PYTHON") {
        let python = python.trim();
        if !python.is_empty() {
            return python.to_string();
        }
    }

    let managed_repo = crate::paths::cosyvoice_repo_dir();
    if script.starts_with(&managed_repo)
        && let Some(managed_python) = resolve_venv_python()
    {
        return managed_python.to_string_lossy().into_owned();
    }

    if cfg!(windows) {
        "python".to_string()
    } else {
        "python3".to_string()
    }
}

fn resolved_model_dir() -> Option<String> {
    let model_dir = std::env::var("COSYVOICE_MODEL_DIR")
        .unwrap_or_else(|_| DEFAULT_MODEL_DIR.to_string())
        .trim()
        .to_string();
    (!model_dir.is_empty()).then_some(model_dir)
}

fn build_command_template(python_bin: &str, server_script: &Path, model_dir: &str) -> String {
    format!(
        "{} {} --port {PORT_TOKEN} --model_dir {}",
        shell_quote(python_bin),
        shell_quote(server_script.to_string_lossy().as_ref()),
        shell_quote(model_dir),
    )
}

fn normalize_command_template(template: &str) -> String {
    let trimmed = template.trim();
    if !trimmed.contains("runtime/python/fastapi/server.py") {
        return trimmed.to_string();
    }

    let mut migrated = trimmed.replace("--host {host} --port {port}", "--port {port}");
    migrated = migrated.replace("--host {host}", "");
    while migrated.contains("  ") {
        migrated = migrated.replace("  ", " ");
    }
    migrated.trim().to_string()
}

fn startup_timeout() -> Duration {
    let raw = std::env::var("COSYVOICE_STARTUP_TIMEOUT_SECS").ok();
    let secs = startup_timeout_secs_from(raw.as_deref());
    Duration::from_secs(secs)
}

fn startup_timeout_secs_from(raw: Option<&str>) -> u64 {
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(DEFAULT_STARTUP_TIMEOUT_SECS)
}

#[cfg(test)]
fn startup_timeout_from(raw: Option<&str>) -> Duration {
    let secs = startup_timeout_secs_from(raw);
    Duration::from_secs(secs)
}

fn ensure_pip_available(python: &Path) -> Result<()> {
    let has_pip = Command::new(python)
        .arg("-m")
        .arg("pip")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if has_pip {
        return Ok(());
    }

    run_checked(
        Command::new(python)
            .arg("-m")
            .arg("ensurepip")
            .arg("--upgrade"),
        "bootstrap pip for CosyVoice venv",
    )?;

    let has_pip_after = Command::new(python)
        .arg("-m")
        .arg("pip")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    ensure!(
        has_pip_after,
        error::CosyvoiceSnafu {
            message: format!(
                "pip is unavailable in CosyVoice venv ({})",
                python.display()
            ),
        }
    );

    Ok(())
}

fn install_requirements_with_fallback(python: &Path, requirements: &Path) -> Result<()> {
    let install_all = run_checked(
        Command::new(python)
            .arg("-m")
            .arg("pip")
            .arg("install")
            .arg("-r")
            .arg(requirements),
        "install CosyVoice runtime dependencies",
    );
    if install_all.is_ok() {
        return Ok(());
    }

    let content = std::fs::read_to_string(requirements).context(error::IoSnafu)?;
    let whisper_spec = content.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with("openai-whisper") {
            Some(
                trimmed
                    .split(';')
                    .next()
                    .unwrap_or(trimmed)
                    .trim()
                    .to_string(),
            )
        } else {
            None
        }
    });
    let Some(whisper_spec) = whisper_spec else {
        return install_all;
    };

    eprintln!("  pip install -r requirements failed; retrying with dedicated whisper fallback...");
    let filtered = content
        .lines()
        .filter(|line| !line.trim().starts_with("openai-whisper"))
        .collect::<Vec<_>>()
        .join("\n");

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let temp_req = std::env::temp_dir().join(format!(
        "kotoba-cosyvoice-requirements-{}-{nonce}.txt",
        std::process::id()
    ));
    std::fs::write(&temp_req, filtered).context(error::IoSnafu)?;

    let retry = (|| -> Result<()> {
        run_checked(
            Command::new(python)
                .arg("-m")
                .arg("pip")
                .arg("install")
                .arg("-r")
                .arg(&temp_req),
            "install CosyVoice dependencies (without openai-whisper)",
        )?;
        run_checked(
            Command::new(python)
                .arg("-m")
                .arg("pip")
                .arg("install")
                .arg(&whisper_spec)
                .arg("--no-build-isolation"),
            "install openai-whisper with no-build-isolation",
        )
    })();

    let _ = std::fs::remove_file(&temp_req);
    retry
}

fn run_checked(command: &mut Command, label: &str) -> Result<()> {
    let output = command.output().context(error::IoSnafu)?;
    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut detail = stderr.trim().to_string();
    if detail.is_empty() {
        detail = stdout.trim().to_string();
    }
    if detail.chars().count() > 600 {
        detail = detail.chars().take(600).collect();
    }

    error::CosyvoiceSnafu {
        message: format!(
            "{label} failed (status {}): {}",
            output.status,
            if detail.is_empty() {
                "(no output)".to_string()
            } else {
                detail
            }
        ),
    }
    .fail()
}

fn command_exists(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
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

    #[test]
    fn normalize_command_template_migrates_legacy_host_arg() {
        let legacy = "'python3' '/tmp/CosyVoice/runtime/python/fastapi/server.py' --host {host} \
                      --port {port} --model_dir 'iic/CosyVoice2-0.5B'";
        let normalized = normalize_command_template(legacy);
        assert!(!normalized.contains("--host {host}"));
        assert!(normalized.contains("--port {port}"));
    }

    #[test]
    fn startup_timeout_reads_env_override() {
        assert_eq!(startup_timeout_from(Some("123")), Duration::from_secs(123));
        assert_eq!(
            startup_timeout_from(Some("invalid")),
            Duration::from_secs(DEFAULT_STARTUP_TIMEOUT_SECS)
        );
    }
}
