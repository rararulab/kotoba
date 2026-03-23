//! `kotoba huggingface` — download and manage ONNX models from `HuggingFace`.

use std::{io::Write, path::Path};

use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, ensure};

use crate::error::{self, Result};

/// A single file entry returned by the `HuggingFace` model API.
#[derive(Debug, Deserialize)]
struct HfSibling {
    /// Repository-relative file path (e.g. `weights/model.pth`).
    rfilename: String,
}

/// Top-level response from `https://huggingface.co/api/models/{repo_id}`.
#[derive(Debug, Deserialize)]
struct HfModelInfo {
    /// List of files in the repository.
    siblings: Vec<HfSibling>,
}

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

/// Query the `HuggingFace` API for the file listing of a repository.
async fn query_repo_files(repo_id: &str) -> Result<Vec<HfSibling>> {
    let client = crate::http::client();
    let url = format!("https://huggingface.co/api/models/{repo_id}");
    let resp = client.get(&url).send().await.context(error::HttpSnafu)?;

    ensure!(
        resp.status().is_success(),
        error::DownloadFailedSnafu {
            url:    url.clone(),
            status: resp.status().to_string(),
        }
    );

    let info: HfModelInfo = resp.json().await.context(error::HttpSnafu)?;
    Ok(info.siblings)
}

/// Locate `.pth` and `.index` files from `HuggingFace` sibling entries.
///
/// When `subpath` is `Some`, only files under that prefix are considered.
/// When `None`, all files are searched and the shallowest `.pth` is preferred
/// (fewest path separators).
///
/// Returns `(pth_path, index_path)` as repository-relative strings.
fn find_rvc_files(
    siblings: &[HfSibling],
    subpath: Option<&str>,
) -> (Option<String>, Option<String>) {
    let candidates: Vec<&str> = siblings
        .iter()
        .map(|s| s.rfilename.as_str())
        .filter(|name| {
            subpath.is_none_or(|prefix| {
                let normalized = prefix.strip_suffix('/').unwrap_or(prefix);
                name.starts_with(normalized) && name.as_bytes().get(normalized.len()) == Some(&b'/')
            })
        })
        .collect();

    // Find the shallowest .pth file (fewest '/' separators)
    let pth = candidates
        .iter()
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pth"))
        })
        .min_by_key(|name| name.matches('/').count())
        .map(|s| (*s).to_string());

    let index = candidates
        .iter()
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("index"))
        })
        .min_by_key(|name| name.matches('/').count())
        .map(|s| (*s).to_string());

    (pth, index)
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
/// Accepts `owner/repo` or `owner/repo:subpath` syntax. Queries the HF API to
/// discover `.pth` and `.index` files, then downloads them as `model.pth` /
/// `model.index` into `~/.kotoba/models/rvc/{model_name}/`.
async fn add_rvc(input: &str) -> Result<ModelAddResult> {
    // Parse optional subpath: "owner/repo:subpath" or just "owner/repo"
    let (repo_id, subpath) = match input.find(':') {
        // Only split on ':' that appears after 'owner/repo' (i.e. after a '/')
        Some(pos) if input[..pos].contains('/') => (&input[..pos], Some(&input[pos + 1..])),
        _ => (input, None),
    };

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

    eprintln!("querying huggingface for rvc model files: {repo_id}...");
    let siblings = query_repo_files(repo_id).await?;
    let (pth_file, index_file) = find_rvc_files(&siblings, subpath);

    let pth_file = pth_file.ok_or_else(|| {
        error::RvcModelNotFoundSnafu {
            repo_id: repo_id.to_string(),
            subpath: subpath.unwrap_or("subpath").to_string(),
        }
        .build()
    })?;

    std::fs::create_dir_all(&model_dir).context(error::IoSnafu)?;

    let client = crate::http::download_client();
    let pth_url = format!("https://huggingface.co/{repo_id}/resolve/main/{pth_file}");

    eprintln!("downloading rvc model: {pth_file}...");
    if let Err(e) = download_file(client, &pth_url, &pth_path).await {
        let _ = std::fs::remove_dir_all(&model_dir);
        return Err(e);
    }

    // Download .index file if discovered (optional for RVC)
    if let Some(index_file) = &index_file {
        let index_url = format!("https://huggingface.co/{repo_id}/resolve/main/{index_file}");
        let index_dest = model_dir.join("model.index");
        eprintln!("downloading rvc index: {index_file}...");
        // Index is optional — don't fail the whole operation if it errors
        let _ = download_file(client, &index_url, &index_dest).await;
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
    use super::*;

    /// Helper to build a vec of `HfSibling` from string slices.
    fn siblings(names: &[&str]) -> Vec<HfSibling> {
        names
            .iter()
            .map(|n| HfSibling {
                rfilename: (*n).to_string(),
            })
            .collect()
    }

    #[test]
    fn find_pth_in_weights_dir() {
        let sibs = siblings(&["README.md", "weights/model.pth", "weights/model.index"]);
        let (pth, idx) = find_rvc_files(&sibs, None);
        assert_eq!(pth.as_deref(), Some("weights/model.pth"));
        assert_eq!(idx.as_deref(), Some("weights/model.index"));
    }

    #[test]
    fn find_pth_with_subpath_filter() {
        let sibs = siblings(&[
            "models/A/modelA.pth",
            "models/A/modelA.index",
            "models/B/modelB.pth",
            "models/B/modelB.index",
        ]);
        let (pth, idx) = find_rvc_files(&sibs, Some("models/A"));
        assert_eq!(pth.as_deref(), Some("models/A/modelA.pth"));
        assert_eq!(idx.as_deref(), Some("models/A/modelA.index"));
    }

    #[test]
    fn find_pth_at_root() {
        let sibs = siblings(&["README.md", "voice.pth", "config.json"]);
        let (pth, idx) = find_rvc_files(&sibs, None);
        assert_eq!(pth.as_deref(), Some("voice.pth"));
        assert_eq!(idx, None);
    }

    #[test]
    fn returns_none_when_no_pth_found() {
        let sibs = siblings(&["README.md", "config.json", "data.bin"]);
        let (pth, idx) = find_rvc_files(&sibs, None);
        assert_eq!(pth, None);
        assert_eq!(idx, None);
    }

    #[test]
    fn prefers_shallowest_pth() {
        let sibs = siblings(&["deep/nested/dir/model.pth", "shallow/model.pth", "root.pth"]);
        let (pth, _) = find_rvc_files(&sibs, None);
        assert_eq!(pth.as_deref(), Some("root.pth"));
    }

    #[test]
    fn subpath_does_not_match_prefix_overlap() {
        // "models/AB/x.pth" should NOT match subpath "models/A"
        let sibs = siblings(&["models/AB/x.pth", "models/A/y.pth"]);
        let (pth, _) = find_rvc_files(&sibs, Some("models/A"));
        assert_eq!(pth.as_deref(), Some("models/A/y.pth"));
    }

    #[test]
    fn parse_rvc_input_without_subpath() {
        let input = "someone/naruto-rvc-v2";
        let (repo_id, subpath) = match input.find(':') {
            Some(pos) if input[..pos].contains('/') => (&input[..pos], Some(&input[pos + 1..])),
            _ => (input, None),
        };
        assert_eq!(repo_id, "someone/naruto-rvc-v2");
        assert_eq!(subpath, None);
    }

    #[test]
    fn parse_rvc_input_with_subpath() {
        let input = "ttttdiva/rvc_okiba:models/miku";
        let (repo_id, subpath) = match input.find(':') {
            Some(pos) if input[..pos].contains('/') => (&input[..pos], Some(&input[pos + 1..])),
            _ => (input, None),
        };
        assert_eq!(repo_id, "ttttdiva/rvc_okiba");
        assert_eq!(subpath, Some("models/miku"));
    }
}
