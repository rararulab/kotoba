mod app_config;
mod cli;
mod db;
mod error;
mod http;
mod paths;
mod romaji;
mod srs;
mod store;
mod tts;
mod vits;

use clap::Parser;
use cli::{Cli, Command};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .init();

    if let Err(e) = run().await {
        eprintln!("Error: {e}");
        println!(
            "{}",
            serde_json::json!({"ok": false, "error": e.to_string()})
        );
        std::process::exit(1);
    }
}

#[allow(clippy::too_many_lines)]
async fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let db = db::Database::open_default().await?;

    // Ensure DB is initialized for all commands except Init and Setup
    if !matches!(cli.command, Command::Init | Command::Setup) {
        db.ensure_initialized().await?;
    }

    match cli.command {
        Command::Init => {
            db.init().await?;
            eprintln!("kotoba initialized at {}", db.path().display());
            println!(
                "{}",
                serde_json::json!({"ok": true, "action": "init", "path": db.path().display().to_string()})
            );
        }
        Command::Status => {
            let status = db.status().await?;
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        Command::Add {
            word,
            reading,
            meaning,
            level,
        } => {
            db.add_vocabulary(&word, &reading, &meaning, &level).await?;
            eprintln!("added: {word}({reading}) = {meaning}");
            println!(
                "{}",
                serde_json::json!({"ok": true, "action": "add", "word": word, "reading": reading, "meaning": meaning})
            );
        }
        Command::Grammar { action } => match action {
            cli::GrammarAction::Add {
                pattern,
                meaning,
                level,
                example,
            } => {
                db.add_grammar(&pattern, &meaning, &level, example.as_deref())
                    .await?;
                eprintln!("added grammar: {pattern} = {meaning}");
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "action": "grammar_add", "pattern": pattern, "meaning": meaning})
                );
            }
            cli::GrammarAction::List { level } => {
                let items = db.all_grammar(level.as_deref()).await?;
                println!("{}", serde_json::to_string_pretty(&items)?);
            }
        },
        Command::Seen {
            word,
            quality,
            grammar,
        } => {
            if grammar {
                srs::record_grammar_review(&db, &word, quality).await?;
                eprintln!("recorded grammar review: {word} quality={quality}");
            } else {
                srs::record_review(&db, &word, quality).await?;
                eprintln!("recorded review: {word} quality={quality}");
            }
            println!(
                "{}",
                serde_json::json!({"ok": true, "action": "seen", "word": word, "quality": quality, "grammar": grammar})
            );
        }
        Command::Review { grammar } => {
            let items = if grammar {
                db.due_grammar().await?
            } else {
                db.due_vocabulary().await?
            };
            println!("{}", serde_json::to_string_pretty(&items)?);
        }
        Command::Progress { weekly } => {
            let progress = db.progress(weekly).await?;
            println!("{}", serde_json::to_string_pretty(&progress)?);
        }
        Command::Play { word } => {
            let path = cli::play::play_word(&word).await?;
            println!(
                "{}",
                serde_json::json!({"ok": true, "action": "play", "path": path.display().to_string()})
            );
        }
        Command::Setup => {
            let result = cli::setup::run(&db).await?;
            println!(
                "{}",
                serde_json::json!({"ok": true, "action": "setup", "db_path": result.db_path, "voicevox_installed": result.voicevox_installed})
            );
        }
        Command::Doctor => {
            cli::doctor::run(&db).await?;
        }
        Command::Voice { action } => match action {
            cli::VoiceAction::List => {
                cli::voice::list()?;
            }
            cli::VoiceAction::Set { name } => {
                cli::voice::set(&name)?;
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "action": "voice_set", "name": name})
                );
            }
        },
        Command::Huggingface { action } => match action {
            cli::HuggingFaceAction::Add { repo_id } => {
                let result = cli::huggingface::add(&repo_id).await?;
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "action": "huggingface_add", "model": result.model, "path": result.path})
                );
            }
            cli::HuggingFaceAction::List => {
                cli::huggingface::list()?;
            }
        },
        Command::Config { action } => match action {
            cli::ConfigAction::Set { key, value } => {
                let mut cfg = app_config::load().clone();
                set_config_field(&mut cfg, &key, &value);
                app_config::save(&cfg)?;
                eprintln!("set {key} = {value}");
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "action": "config_set", "key": key, "value": value})
                );
            }
            cli::ConfigAction::Get { key } => {
                let cfg = app_config::load();
                let value = get_config_field(cfg, &key);
                let display_value = value.as_deref().unwrap_or("(not set)");
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "action": "config_get", "key": key, "value": display_value})
                );
            }
            cli::ConfigAction::List => {
                let cfg = app_config::load();
                let entries = config_as_map(cfg);
                let map: serde_json::Map<String, serde_json::Value> = entries
                    .into_iter()
                    .map(|(k, v)| (k, serde_json::Value::String(v)))
                    .collect();
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "action": "config_list", "entries": map})
                );
            }
        },
        Command::List { grammar, level } => {
            if grammar {
                let items = db.all_grammar(level.as_deref()).await?;
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                let items = db.all_vocabulary(level.as_deref()).await?;
                println!("{}", serde_json::to_string_pretty(&items)?);
            }
        }
        Command::Export { format, grammar } => {
            cli::export::export(&db, &format, grammar).await?;
        }
    }

    Ok(())
}

/// Set a config field by dotted key path.
fn set_config_field(cfg: &mut app_config::AppConfig, key: &str, value: &str) {
    match key {
        "voice.active" => cfg.voice.active = value.to_string(),
        "voicevox.version" => cfg.voicevox.version = value.to_string(),
        "voicevox.url" => cfg.voicevox.url = value.to_string(),
        "voicevox.speaker" => cfg.voicevox.speaker = value.to_string(),
        _ => eprintln!("warning: unknown config key: {key}"),
    }
}

/// Get a config field by dotted key path.
fn get_config_field(cfg: &app_config::AppConfig, key: &str) -> Option<String> {
    match key {
        "voice.active" => Some(cfg.voice.active.clone()),
        "voicevox.version" => Some(cfg.voicevox.version.clone()),
        "voicevox.url" => Some(cfg.voicevox.url.clone()),
        "voicevox.speaker" => Some(cfg.voicevox.speaker.clone()),
        _ => None,
    }
}

/// Flatten config into key-value pairs for listing.
fn config_as_map(cfg: &app_config::AppConfig) -> Vec<(String, String)> {
    vec![
        ("voice.active".to_string(), cfg.voice.active.clone()),
        ("voicevox.version".to_string(), cfg.voicevox.version.clone()),
        ("voicevox.url".to_string(), cfg.voicevox.url.clone()),
        ("voicevox.speaker".to_string(), cfg.voicevox.speaker.clone()),
    ]
}
