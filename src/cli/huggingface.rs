//! `kotoba huggingface` — download and manage ONNX models from `HuggingFace`.

use std::io::Write;

use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use snafu::ResultExt;

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

        let response = client.get(*url).send().await.context(error::HttpSnafu)?;

        if !response.status().is_success() {
            return Err(error::KokoroSnafu {
                message: format!("failed to download {filename}: HTTP {}", response.status()),
            }
            .build());
        }

        let total_size = response.content_length().unwrap_or(0);

        let pb = ProgressBar::new(total_size);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{bar:40.cyan/blue} {percent}% {bytes}/{total_bytes}  {bytes_per_sec}  ETA \
                     {eta}",
                )
                .expect("valid progress bar template")
                .progress_chars("=>-"),
        );

        let mut stream = response.bytes_stream();
        let mut file = std::fs::File::create(&dest).context(error::IoSnafu)?;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context(error::HttpSnafu)?;
            file.write_all(&chunk).context(error::IoSnafu)?;
            pb.inc(chunk.len() as u64);
        }

        pb.finish_and_clear();

        let file_size = std::fs::metadata(&dest).context(error::IoSnafu)?.len();
        if file_size == 0 {
            let _ = std::fs::remove_file(&dest);
            return Err(error::KokoroSnafu {
                message: format!("{filename} is empty (0 bytes)"),
            }
            .build());
        }
        eprintln!("  downloaded {file_size} bytes");
    }

    eprintln!("kokoro model saved to: {}", model_dir.display());
    eprintln!("use `kotoba voice set kokoro:<voice>` to activate");

    Ok(ModelAddResult {
        model: "kokoro".to_string(),
        path:  model_dir.display().to_string(),
    })
}

/// Download an ONNX model from `HuggingFace`, or the Kokoro model.
pub async fn add(repo_id: &str) -> Result<ModelAddResult> {
    if repo_id == "kokoro" {
        return add_kokoro().await;
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

    let response = client
        .get(&model_url)
        .send()
        .await
        .context(error::HttpSnafu)?;

    if !response.status().is_success() {
        return Err(error::VoicevoxSnafu {
            message: format!("failed to download {model_url}: HTTP {}", response.status()),
        }
        .build());
    }

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

    std::fs::create_dir_all(&model_dir).context(error::IoSnafu)?;

    // Stream the response body to disk in chunks instead of buffering the
    // entire model file in memory.
    let mut stream = response.bytes_stream();
    let model_path = model_dir.join("model.onnx");
    let mut file = std::fs::File::create(&model_path).context(error::IoSnafu)?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context(error::HttpSnafu)?;
        file.write_all(&chunk).context(error::IoSnafu)?;
        pb.inc(chunk.len() as u64);
    }

    pb.finish_and_clear();

    // Basic size validation — HuggingFace doesn't provide standard checksum
    // sidecars
    let file_size = std::fs::metadata(&model_path)
        .context(error::IoSnafu)?
        .len();
    if file_size == 0 {
        // Clean up the empty file
        let _ = std::fs::remove_dir_all(&model_dir);
        return Err(error::VoicevoxSnafu {
            message: "downloaded model file is empty (0 bytes)".to_string(),
        }
        .build());
    }
    eprintln!("  downloaded {file_size} bytes");

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
