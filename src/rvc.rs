//! RVC v2 voice conversion via subprocess.
//!
//! Calls `rvc_python` CLI to convert base TTS audio into
//! anime character voices. Users must install it with
//! `pip install rvc-python`.

use std::path::Path;

use snafu::ResultExt;

use crate::error::{self, Result};

/// Check that `rvc_python` is installed and importable.
pub async fn check_installed() -> Result<()> {
    let output = tokio::process::Command::new("python3")
        .args(["-c", "import rvc_python"])
        .output()
        .await
        .context(error::IoSnafu)?;

    if !output.status.success() {
        return Err(error::RvcNotInstalledSnafu.build());
    }
    Ok(())
}

/// Convert audio at `input_path` using the given RVC model,
/// writing the result to `output_path`.
///
/// Spawns `rvc_cli infer` as a subprocess with the model located
/// at `~/.kotoba/models/rvc/{model}/model.pth`.
pub async fn convert(input_path: &Path, model: &str, output_path: &Path) -> Result<()> {
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
    cmd.args(["infer", "-i"])
        .arg(input_path)
        .args(["-o"])
        .arg(output_path)
        .args(["-mp"])
        .arg(&model_pth);

    // Use index file if available for better voice quality
    let index_path = model_dir.join("model.index");
    if index_path.exists() {
        cmd.args(["-ip"]).arg(&index_path);
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
    #[test]
    fn rvc_model_path_resolves() {
        let dir = crate::paths::models_dir().join("rvc").join("test-model");
        assert!(dir.ends_with("rvc/test-model"));
    }
}
