//! RVC v2 voice conversion via a Python subprocess.
//!
//! Spawns `python3 -c <inline_script>` per conversion call.
//! The inline script uses `infer-rvc-python` for inference.

use std::path::{Path, PathBuf};

use snafu::{ResultExt, ensure};

use crate::error::{self, Result};

/// Inline Python script for RVC inference.
///
/// Arguments:
/// `<model.pth> <index_path_or_empty> <input.wav> <output.wav> [pitch]
/// [pitch_algo] [index_influence]`
const CONVERT_PY: &str = r"
import torch, io, sys
_orig = torch.load
def _p(*a,**k): k.setdefault('weights_only',False); return _orig(*a,**k)
torch.load = _p
import soundfile as sf
from infer_rvc_python import BaseLoader
from pathlib import Path

model_pth = sys.argv[1]
index_path = sys.argv[2]
input_path = sys.argv[3]
output_path = sys.argv[4]
pitch = int(sys.argv[5]) if len(sys.argv) > 5 else 0
pitch_algo = sys.argv[6] if len(sys.argv) > 6 else 'rmvpe'
index_influence = float(sys.argv[7]) if len(sys.argv) > 7 else 0.66

c = BaseLoader(only_cpu=True)
c.apply_conf(
    tag='m', file_model=model_pth, pitch_algo=pitch_algo, pitch_lvl=pitch,
    file_index=index_path if index_path else '',
    index_influence=index_influence if index_path else 0.0,
)
r = c.generate_from_cache(audio_data=input_path, tag='m')
buf = io.BytesIO()
sf.write(buf, r[0], r[1], format='WAV')
buf.seek(0)
Path(output_path).write_bytes(buf.read())
";

/// Resolve the Python executable for RVC inference.
///
/// Priority: `RVC_PYTHON` env var > `rvc.python` config > auto-detect
/// venv at `~/.kotoba/venvs/rvc/bin/python3` > system `python3`.
pub fn resolve_python() -> PathBuf {
    // 1. Environment variable
    if let Ok(p) = std::env::var("RVC_PYTHON") {
        return PathBuf::from(p);
    }

    // 2. Config file
    let cfg = crate::app_config::load().rvc.python.clone();
    if !cfg.is_empty() {
        return PathBuf::from(cfg);
    }

    // 3. Auto-detect managed venv
    let venv_python = crate::paths::rvc_venv_dir().join("bin").join("python3");
    if venv_python.exists() {
        return venv_python;
    }

    // 4. System fallback
    PathBuf::from("python3")
}

/// Validate that a model name contains no path traversal characters.
fn validate_model_name(model: &str) -> Result<()> {
    ensure!(
        !model.is_empty() && !model.contains('/') && !model.contains('\\') && !model.contains(".."),
        error::RvcSnafu {
            message: format!("invalid model name: {model}"),
        }
    );
    Ok(())
}

/// Find the first file with the given extension in a directory.
fn find_file_by_ext(dir: &Path, ext: &str) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
}

/// Convert audio at `input_path` using the given RVC model and pitch shift
/// (in semitones), writing the result to `output_path`.
///
/// Spawns a `python3 -c` subprocess that loads the model and
/// runs inference via `infer-rvc-python`.
pub async fn convert(
    input_path: &Path,
    model: &str,
    pitch: i32,
    pitch_algo: &str,
    index_influence: f32,
    output_path: &Path,
) -> Result<()> {
    validate_model_name(model)?;

    let model_dir = crate::paths::models_dir().join("rvc").join(model);
    let pth_path = find_file_by_ext(&model_dir, "pth").ok_or_else(|| {
        error::RvcSnafu {
            message: format!("no .pth file in {}", model_dir.display()),
        }
        .build()
    })?;

    let index_path = find_file_by_ext(&model_dir, "index")
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let python = resolve_python();
    let output = tokio::process::Command::new(&python)
        .arg("-c")
        .arg(CONVERT_PY)
        .arg(pth_path.display().to_string())
        .arg(&index_path)
        .arg(input_path.display().to_string())
        .arg(output_path.display().to_string())
        .arg(pitch.to_string())
        .arg(pitch_algo)
        .arg(index_influence.clamp(0.0, 1.0).to_string())
        .env("OMP_NUM_THREADS", "1")
        .env("MKL_NUM_THREADS", "1")
        .output()
        .await
        .context(error::IoSnafu)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return error::RvcSnafu {
            message: format!(
                "{} exited with {}: {}",
                python.display(),
                output.status,
                stderr.chars().take(500).collect::<String>()
            ),
        }
        .fail();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn convert_script_contains_baseloader() {
        assert!(CONVERT_PY.contains("BaseLoader"));
    }
}
