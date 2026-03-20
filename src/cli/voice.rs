//! `kotoba voice` — manage TTS voice models.

use std::path::PathBuf;

use serde::Serialize;
use snafu::ResultExt;

use crate::{
    db::Database,
    error::{self, Result},
};

/// A voice entry for display.
#[derive(Debug, Serialize)]
pub struct VoiceInfo {
    pub name:    String,
    pub backend: String,
    pub active:  bool,
}

fn models_dir() -> Result<PathBuf> {
    let dir = dirs::home_dir()
        .ok_or_else(|| error::HomeNotFoundSnafu.build())?
        .join(".kotoba")
        .join("models");
    std::fs::create_dir_all(&dir).context(error::IoSnafu)?;
    Ok(dir)
}

/// List available voices.
pub async fn list(db: &Database) -> Result<()> {
    let current = db
        .get_config("voice")
        .await?
        .unwrap_or_else(|| "voicevox:1".to_string());

    let mut voices: Vec<VoiceInfo> = Vec::new();

    // VOICEVOX built-in speakers (subset of most popular ones)
    let voicevox_speakers = [
        ("1", "四国めたん (normal)"),
        ("2", "四国めたん (あまあま)"),
        ("3", "ずんだもん (normal)"),
        ("4", "ずんだもん (あまあま)"),
        ("8", "春日部つむぎ"),
        ("10", "雨晴はう"),
        ("13", "青山龍星"),
        ("14", "冥鳴ひまり"),
        ("20", "もち子さん"),
        ("23", "WhiteCUL"),
    ];

    for (id, name) in &voicevox_speakers {
        let key = format!("voicevox:{id}");
        voices.push(VoiceInfo {
            name:    format!("{name} [{key}]"),
            backend: "voicevox".to_string(),
            active:  current == key,
        });
    }

    // Downloaded HuggingFace models
    let models_path = models_dir()?;
    if let Ok(entries) = std::fs::read_dir(&models_path) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let dir_name = entry.file_name().to_string_lossy().to_string();
                let key = format!("vits:{dir_name}");
                voices.push(VoiceInfo {
                    name:    format!("{dir_name} [{key}]"),
                    backend: "vits".to_string(),
                    active:  current == key,
                });
            }
        }
    }

    let output = serde_json::to_string_pretty(&voices).context(error::JsonSnafu)?;
    println!("{output}");
    Ok(())
}

/// Set the active voice.
pub async fn set(db: &Database, name: &str) -> Result<()> {
    db.set_config("voice", name).await?;
    println!("voice set to: {name}");
    Ok(())
}

/// Download a voice model from `HuggingFace`.
pub async fn add(repo_id: &str) -> Result<()> {
    let models_path = models_dir()?;
    let model_name = repo_id.split('/').next_back().unwrap_or(repo_id);
    let model_dir = models_path.join(model_name);

    if model_dir.exists() {
        println!("model already downloaded: {}", model_dir.display());
        return Ok(());
    }

    println!("downloading model from huggingface: {repo_id}...");

    // Download model.onnx from HuggingFace
    let client = reqwest::Client::new();
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

    let bytes = response.bytes().await.context(error::HttpSnafu)?;

    std::fs::create_dir_all(&model_dir).context(error::IoSnafu)?;
    std::fs::write(model_dir.join("model.onnx"), &bytes).context(error::IoSnafu)?;

    // Try to download config.json if available
    let config_url = format!("https://huggingface.co/{repo_id}/resolve/main/config.json");
    if let Ok(resp) = client.get(&config_url).send().await
        && resp.status().is_success()
        && let Ok(config_bytes) = resp.bytes().await
    {
        let _ = std::fs::write(model_dir.join("config.json"), &config_bytes);
    }

    println!("model saved to: {}", model_dir.display());
    println!("use `kotoba voice set vits:{model_name}` to activate");

    Ok(())
}
