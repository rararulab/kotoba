//! `kotoba huggingface` — download and manage ONNX models from `HuggingFace`.

use std::{io::Write, path::Path};

use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use snafu::{ResultExt, ensure};

use crate::error::{self, Result};

/// Result of downloading a model.
#[derive(Debug, Serialize)]
pub struct ModelAddResult {
    /// The model identifier (directory name).
    pub model: String,
    /// Filesystem path where the model was saved.
    pub path:  String,
}

/// Info about a downloaded model.
#[derive(Debug, Serialize)]
pub struct ModelInfo {
    /// Model directory name.
    pub name:       String,
    /// Filesystem path.
    pub path:       String,
    /// Whether a config.json exists alongside the model.
    pub has_config: bool,
}

/// Download a file from `url` to `dest` with a progress bar.
///
/// Returns the number of bytes written. Removes the destination file and
/// returns an error when the download produces an empty file.
async fn download_file(client: &reqwest::Client, url: &str, dest: &Path) -> Result<u64> {
    let response = client.get(url).send().await.context(error::HttpSnafu)?;

    ensure!(
        response.status().is_success(),
        error::DownloadFailedSnafu {
            url:    url.to_string(),
            status: response.status().to_string(),
        }
    );

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

    let mut stream = response.bytes_stream();
    let mut file = std::fs::File::create(dest).context(error::IoSnafu)?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context(error::HttpSnafu)?;
        file.write_all(&chunk).context(error::IoSnafu)?;
        pb.inc(chunk.len() as u64);
    }

    pb.finish_and_clear();

    let file_size = std::fs::metadata(dest).context(error::IoSnafu)?.len();
    if file_size == 0 {
        let _ = std::fs::remove_file(dest);
        return error::DownloadFailedSnafu {
            url:    url.to_string(),
            status: "downloaded file is empty (0 bytes)",
        }
        .fail();
    }

    eprintln!("  downloaded {file_size} bytes");
    Ok(file_size)
}

/// List downloaded `HuggingFace` models.
pub fn list() -> Result<()> {
    let models_path = crate::paths::models_dir();
    std::fs::create_dir_all(&models_path).context(error::IoSnafu)?;
    let mut models: Vec<ModelInfo> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&models_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("model.onnx").exists() {
                let name = entry.file_name().to_string_lossy().to_string();
                models.push(ModelInfo {
                    name,
                    path: path.display().to_string(),
                    has_config: path.join("config.json").exists(),
                });
            }
        }
    }

    let output = serde_json::to_string_pretty(&models).context(error::JsonSnafu)?;
    println!("{output}");
    Ok(())
}

/// Download Kokoro ONNX model files from GitHub releases.
///
/// Downloads `kokoro-v1.0.onnx` and `voices-v1.0.bin` into
/// `~/.kotoba/models/kokoro/`. Returns early if both files already exist.
async fn add_kokoro() -> Result<ModelAddResult> {
    let model_dir = crate::paths::models_dir().join("kokoro");
    let onnx_path = model_dir.join("kokoro-v1.0.onnx");
    let voices_path = model_dir.join("voices-v1.0.bin");

    if onnx_path.exists() && voices_path.exists() {
        eprintln!("kokoro model already downloaded: {}", model_dir.display());
        return Ok(ModelAddResult {
            model: "kokoro".to_string(),
            path:  model_dir.display().to_string(),
        });
    }

    std::fs::create_dir_all(&model_dir).context(error::IoSnafu)?;

    let client = crate::http::client();

    let files = [
        (
            "kokoro-v1.0.onnx",
            "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx",
        ),
        (
            "voices-v1.0.bin",
            "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin",
        ),
    ];

    for (filename, url) in &files {
        let dest = model_dir.join(filename);
        if dest.exists() {
            eprintln!("{filename} already exists, skipping");
            continue;
        }

        eprintln!("downloading {filename}...");
        download_file(client, url, &dest).await?;
    }

    eprintln!("kokoro model saved to: {}", model_dir.display());
    eprintln!("use `kotoba voice set kokoro:<voice>` to activate");

    Ok(ModelAddResult {
        model: "kokoro".to_string(),
        path:  model_dir.display().to_string(),
    })
}

/// Download an RVC voice model from `HuggingFace`.
///
/// Downloads `model.pth` (required) and `model.index` (optional) into
/// `~/.kotoba/models/rvc/{model_name}/`. Skips if `model.pth` already exists.
async fn add_rvc(repo_id: &str) -> Result<ModelAddResult> {
    let model_name = repo_id.split('/').next_back().unwrap_or(repo_id);
    let model_dir = crate::paths::models_dir().join("rvc").join(model_name);
    let pth_path = model_dir.join("model.pth");

    if pth_path.exists() {
        eprintln!("rvc model already downloaded: {}", model_dir.display());
        return Ok(ModelAddResult {
            model: model_name.to_string(),
            path:  model_dir.display().to_string(),
        });
    }

    std::fs::create_dir_all(&model_dir).context(error::IoSnafu)?;

    let client = crate::http::client();
    let pth_url = format!("https://huggingface.co/{repo_id}/resolve/main/model.pth");

    eprintln!("downloading rvc model from huggingface: {repo_id}...");

    if let Err(e) = download_file(client, &pth_url, &pth_path).await {
        // Clean up empty directory on failure
        let _ = std::fs::remove_dir_all(&model_dir);
        return Err(e);
    }

    // Try to download model.index if available (optional for RVC)
    let index_url = format!("https://huggingface.co/{repo_id}/resolve/main/model.index");
    if let Ok(resp) = client.get(&index_url).send().await
        && resp.status().is_success()
        && let Ok(index_bytes) = resp.bytes().await
    {
        let _ = std::fs::write(model_dir.join("model.index"), &index_bytes);
    }

    eprintln!("rvc model saved to: {}", model_dir.display());
    eprintln!("use `kotoba voice set kokoro:<voice>+rvc:{model_name}` to activate");

    Ok(ModelAddResult {
        model: model_name.to_string(),
        path:  model_dir.display().to_string(),
    })
}

/// Download an ONNX model from `HuggingFace`, the Kokoro model, or an RVC
/// model.
pub async fn add(repo_id: &str) -> Result<ModelAddResult> {
    if repo_id == "kokoro" {
        return add_kokoro().await;
    }
    if let Some(rvc_repo) = repo_id.strip_prefix("rvc:") {
        return add_rvc(rvc_repo).await;
    }

    let models_path = crate::paths::models_dir();
    std::fs::create_dir_all(&models_path).context(error::IoSnafu)?;
    let model_name = repo_id.split('/').next_back().unwrap_or(repo_id);
    let model_dir = models_path.join(model_name);

    if model_dir.exists() {
        eprintln!("model already downloaded: {}", model_dir.display());
        return Ok(ModelAddResult {
            model: model_name.to_string(),
            path:  model_dir.display().to_string(),
        });
    }

    eprintln!("downloading model from huggingface: {repo_id}...");

    let client = crate::http::client();
    let model_url = format!("https://huggingface.co/{repo_id}/resolve/main/model.onnx");

    std::fs::create_dir_all(&model_dir).context(error::IoSnafu)?;

    let model_path = model_dir.join("model.onnx");
    if let Err(e) = download_file(client, &model_url, &model_path).await {
        // Clean up the directory on failure
        let _ = std::fs::remove_dir_all(&model_dir);
        return Err(e);
    }

    // Try to download config.json if available
    let config_url = format!("https://huggingface.co/{repo_id}/resolve/main/config.json");
    if let Ok(resp) = client.get(&config_url).send().await
        && resp.status().is_success()
        && let Ok(config_bytes) = resp.bytes().await
    {
        let _ = std::fs::write(model_dir.join("config.json"), &config_bytes);
    }

    eprintln!("model saved to: {}", model_dir.display());
    eprintln!("use `kotoba voice set vits:{model_name}` to activate");

    Ok(ModelAddResult {
        model: model_name.to_string(),
        path:  model_dir.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn rvc_model_path_uses_last_segment() {
        let repo_id = "someone/naruto-rvc-v2";
        let model_name = repo_id.split('/').next_back().unwrap_or(repo_id);
        let model_dir = crate::paths::models_dir().join("rvc").join(model_name);

        assert_eq!(model_name, "naruto-rvc-v2");
        assert!(model_dir.ends_with("models/rvc/naruto-rvc-v2"));
    }

    #[test]
    fn rvc_model_path_handles_bare_name() {
        let repo_id = "my-model";
        let model_name = repo_id.split('/').next_back().unwrap_or(repo_id);

        assert_eq!(model_name, "my-model");
    }

    #[test]
    fn add_dispatches_rvc_prefix() {
        let input = "rvc:someone/naruto-rvc-v2";
        let stripped = input.strip_prefix("rvc:");

        assert_eq!(stripped, Some("someone/naruto-rvc-v2"));
    }
}
