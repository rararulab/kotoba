//! RVC v2 voice conversion via subprocess.
//!
//! Calls `rvc_python` CLI to convert base TTS audio into
//! anime character voices. Users must install it with
//! `pip install rvc-python`.

use std::{path::Path, sync::OnceLock};

use snafu::ResultExt;

use crate::error::{self, Result};

/// Cached result of the `rvc_python` installation check.
static RVC_INSTALLED: OnceLock<bool> = OnceLock::new();

/// Check that `rvc_python` is installed and importable.
///
/// The result is cached after the first call to avoid spawning
/// a Python process on every invocation.
pub async fn check_installed() -> Result<()> {
    if let Some(&installed) = RVC_INSTALLED.get() {
        return if installed {
            Ok(())
        } else {
            Err(error::RvcNotInstalledSnafu.build())
        };
    }

    let output = tokio::process::Command::new("python3")
        .args(["-c", "import rvc_python"])
        .output()
        .await
        .context(error::IoSnafu)?;

    let installed = output.status.success();
    let _ = RVC_INSTALLED.set(installed);

    if !installed {
        return Err(error::RvcNotInstalledSnafu.build());
    }
    Ok(())
}

/// Validate that a model name contains no path traversal characters.
fn validate_model_name(model: &str) -> Result<()> {
    if model.is_empty() || model.contains('/') || model.contains('\\') || model.contains("..") {
        return Err(error::RvcSnafu {
            message: format!("invalid model name: {model}"),
        }
        .build());
    }
    Ok(())
}

/// Convert audio at `input_path` using the given RVC model,
/// writing the result to `output_path`.
///
/// Spawns `rvc_cli infer` as a subprocess with the model located
/// at `~/.kotoba/models/rvc/{model}/model.pth`.
pub async fn convert(input_path: &Path, model: &str, output_path: &Path) -> Result<()> {
    validate_model_name(model)?;

    let model_dir = crate::paths::models_dir().join("rvc").join(model);
    let model_pth = model_dir.join("model.pth");

    if !model_pth.exists() {
        return Err(error::RvcSnafu {
            message: format!(
                "model not found: {model} — download with `kotoba huggingface add rvc:<repo>`"
            ),
        }
        .build());
    }

    let mut cmd = tokio::process::Command::new("rvc_cli");
    cmd.arg("infer")
        .arg("-i")
        .arg(input_path)
        .arg("-o")
        .arg(output_path)
        .arg("-mp")
        .arg(&model_pth);

    // Use index file if available for better voice quality
    let index_path = model_dir.join("model.index");
    if index_path.exists() {
        cmd.arg("-ip").arg(&index_path);
    }

    let output = cmd.output().await.context(error::IoSnafu)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(error::RvcSnafu {
            message: format!("rvc_cli infer failed: {stderr}"),
        }
        .build());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rvc_model_path_resolves() {
        let dir = crate::paths::models_dir().join("rvc").join("test-model");
        assert!(dir.ends_with("rvc/test-model"));
    }

    #[test]
    fn validate_rejects_path_traversal() {
        assert!(validate_model_name("../evil").is_err());
        assert!(validate_model_name("foo/bar").is_err());
        assert!(validate_model_name("foo\\bar").is_err());
        assert!(validate_model_name("").is_err());
    }

    #[test]
    fn validate_accepts_normal_names() {
        assert!(validate_model_name("naruto-rvc-v2").is_ok());
        assert!(validate_model_name("my_model").is_ok());
    }
}
