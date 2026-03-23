mod app_config;
mod cli;
mod cosyvoice_runtime;
mod db;
mod error;
mod http;
pub(crate) mod kokoro;
mod paths;
mod romaji;
mod rvc;
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
            let (reading, meaning) = match (reading, meaning) {
                (Some(reading), Some(meaning)) => (reading, meaning),
                (None, None) => {
                    let auto = cli::dictionary::lookup(&word).await?;
                    eprintln!(
                        "auto-filled from dictionary: {word}({}) = {}",
                        auto.reading, auto.meaning
                    );
                    (auto.reading, auto.meaning)
                }
                _ => {
                    return Err(Box::new(error::KotobaError::WordLookup {
                        word:    word.clone(),
                        message: "provide both <reading> and <meaning>, or omit both for auto-fill"
                            .to_string(),
                    }));
                }
            };

            let level_str = level.to_string();
            db.add_vocabulary(&word, &reading, &meaning, &level_str)
                .await?;
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
                let level_str = level.to_string();
                db.add_grammar(&pattern, &meaning, &level_str, example.as_deref())
                    .await?;
                eprintln!("added grammar: {pattern} = {meaning}");
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "action": "grammar_add", "pattern": pattern, "meaning": meaning})
                );
            }
            cli::GrammarAction::List { level } => {
                let level_str = level.as_ref().map(std::string::ToString::to_string);
                let items = db.all_grammar(level_str.as_deref()).await?;
                println!("{}", serde_json::to_string_pretty(&items)?);
            }
        },
        Command::Seen {
            word,
            quality,
            grammar,
        } => {
            let q = quality.as_u8();
            if grammar {
                srs::record_grammar_review(&db, &word, q).await?;
                eprintln!("recorded grammar review: {word} quality={q}");
            } else {
                srs::record_review(&db, &word, q).await?;
                eprintln!("recorded review: {word} quality={q}");
            }
            println!(
                "{}",
                serde_json::json!({"ok": true, "action": "seen", "word": word, "quality": q, "grammar": grammar})
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
        Command::Play {
            word,
            enable,
            style,
        } => {
            let romaji = romaji::to_romaji_natural(&word).await;
            let path = cli::play::play_word(&word, enable, style).await?;
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "action": "play",
                    "path": path.display().to_string(),
                    "style": format!("{style:?}").to_lowercase(),
                    "romaji": romaji
                })
            );
        }
        Command::Setup => {
            let result = cli::setup::run(&db).await?;
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "action": "setup",
                    "db_path": result.db_path,
                    "voicevox_installed": result.voicevox_installed,
                    "voicevox_running": result.voicevox_running,
                    "cosyvoice_configured": result.cosyvoice_configured,
                    "cosyvoice_running": result.cosyvoice_running
                })
            );
        }
        Command::Doctor { json } => {
            cli::doctor::run(&db, json).await?;
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
            cli::VoiceAction::Tone { action } => match action {
                cli::VoiceToneAction::List => {
                    cli::voice::list_tones()?;
                }
                cli::VoiceToneAction::Set { name } => {
                    let applied = cli::voice::set_tone(&name)?;
                    println!(
                        "{}",
                        serde_json::json!({
                            "ok": true,
                            "action": "voice_tone_set",
                            "preset": applied.name,
                            "voice_speed": applied.voice_speed,
                            "rvc_pitch": applied.rvc_pitch,
                            "rvc_pitch_algo": applied.rvc_pitch_algo,
                            "rvc_index_influence": applied.rvc_index_influence
                        })
                    );
                }
            },
            cli::VoiceAction::Rvc { action } => match action {
                cli::VoiceRvcAction::List => {
                    cli::voice::list_rvc()?;
                }
                cli::VoiceRvcAction::Set { name } => {
                    let resolved = cli::voice::set_rvc(&name)?;
                    println!(
                        "{}",
                        serde_json::json!({"ok": true, "action": "rvc_set", "model": resolved})
                    );
                }
                cli::VoiceRvcAction::Off => {
                    cli::voice::off_rvc()?;
                    println!("{}", serde_json::json!({"ok": true, "action": "rvc_off"}));
                }
            },
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
            let level_str = level.as_ref().map(std::string::ToString::to_string);
            if grammar {
                let items = db.all_grammar(level_str.as_deref()).await?;
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                let items = db.all_vocabulary(level_str.as_deref()).await?;
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
        "voice.speed" => match value.parse::<f64>() {
            Ok(v) => cfg.voice.speed = v,
            Err(_) => eprintln!("warning: invalid float for voice.speed: {value}"),
        },
        "voicevox.version" => cfg.voicevox.version = value.to_string(),
        "voicevox.url" => cfg.voicevox.url = value.to_string(),
        "voicevox.speaker" => cfg.voicevox.speaker = value.to_string(),
        "cosyvoice.url" => cfg.cosyvoice.url = value.to_string(),
        "cosyvoice.autostart" => match parse_bool_value(value) {
            Some(v) => cfg.cosyvoice.autostart = v,
            None => eprintln!("warning: invalid bool for cosyvoice.autostart: {value}"),
        },
        "cosyvoice.command" => cfg.cosyvoice.command = value.to_string(),
        "cosyvoice.mode" => cfg.cosyvoice.mode = value.to_string(),
        "cosyvoice.prompt_text" => cfg.cosyvoice.prompt_text = value.to_string(),
        "cosyvoice.prompt_wav" => cfg.cosyvoice.prompt_wav = value.to_string(),
        "cosyvoice.instruct_text" => cfg.cosyvoice.instruct_text = value.to_string(),
        "rvc.model" => cfg.rvc.model = value.to_string(),
        "rvc.python" => cfg.rvc.python = value.to_string(),
        "rvc.pitch" => match value.parse::<i32>() {
            Ok(v) => cfg.rvc.pitch = v,
            Err(_) => eprintln!("warning: invalid integer for rvc.pitch: {value}"),
        },
        "rvc.pitch_algo" => cfg.rvc.pitch_algo = value.to_string(),
        "rvc.index_influence" => match value.parse::<f64>() {
            Ok(v) => cfg.rvc.index_influence = v.clamp(0.0, 1.0),
            Err(_) => eprintln!("warning: invalid float for rvc.index_influence: {value}"),
        },
        _ => eprintln!("warning: unknown config key: {key}"),
    }
}

/// Get a config field by dotted key path.
fn get_config_field(cfg: &app_config::AppConfig, key: &str) -> Option<String> {
    match key {
        "voice.active" => Some(cfg.voice.active.clone()),
        "voice.speed" => Some(cfg.voice.speed.to_string()),
        "voicevox.version" => Some(cfg.voicevox.version.clone()),
        "voicevox.url" => Some(cfg.voicevox.url.clone()),
        "voicevox.speaker" => Some(cfg.voicevox.speaker.clone()),
        "cosyvoice.url" => Some(cfg.cosyvoice.url.clone()),
        "cosyvoice.autostart" => Some(cfg.cosyvoice.autostart.to_string()),
        "cosyvoice.command" => Some(cfg.cosyvoice.command.clone()),
        "cosyvoice.mode" => Some(cfg.cosyvoice.mode.clone()),
        "cosyvoice.prompt_text" => Some(cfg.cosyvoice.prompt_text.clone()),
        "cosyvoice.prompt_wav" => Some(cfg.cosyvoice.prompt_wav.clone()),
        "cosyvoice.instruct_text" => Some(cfg.cosyvoice.instruct_text.clone()),
        "rvc.model" => Some(cfg.rvc.model.clone()),
        "rvc.python" => Some(cfg.rvc.python.clone()),
        "rvc.pitch" => Some(cfg.rvc.pitch.to_string()),
        "rvc.pitch_algo" => Some(cfg.rvc.pitch_algo.clone()),
        "rvc.index_influence" => Some(cfg.rvc.index_influence.to_string()),
        _ => None,
    }
}

/// Flatten config into key-value pairs for listing.
fn config_as_map(cfg: &app_config::AppConfig) -> Vec<(String, String)> {
    vec![
        ("voice.active".to_string(), cfg.voice.active.clone()),
        ("voice.speed".to_string(), cfg.voice.speed.to_string()),
        ("voicevox.version".to_string(), cfg.voicevox.version.clone()),
        ("voicevox.url".to_string(), cfg.voicevox.url.clone()),
        ("voicevox.speaker".to_string(), cfg.voicevox.speaker.clone()),
        ("cosyvoice.url".to_string(), cfg.cosyvoice.url.clone()),
        (
            "cosyvoice.autostart".to_string(),
            cfg.cosyvoice.autostart.to_string(),
        ),
        (
            "cosyvoice.command".to_string(),
            cfg.cosyvoice.command.clone(),
        ),
        ("cosyvoice.mode".to_string(), cfg.cosyvoice.mode.clone()),
        (
            "cosyvoice.prompt_text".to_string(),
            cfg.cosyvoice.prompt_text.clone(),
        ),
        (
            "cosyvoice.prompt_wav".to_string(),
            cfg.cosyvoice.prompt_wav.clone(),
        ),
        (
            "cosyvoice.instruct_text".to_string(),
            cfg.cosyvoice.instruct_text.clone(),
        ),
        ("rvc.model".to_string(), cfg.rvc.model.clone()),
        ("rvc.python".to_string(), cfg.rvc.python.clone()),
        ("rvc.pitch".to_string(), cfg.rvc.pitch.to_string()),
        ("rvc.pitch_algo".to_string(), cfg.rvc.pitch_algo.clone()),
        (
            "rvc.index_influence".to_string(),
            cfg.rvc.index_influence.to_string(),
        ),
    ]
}

fn parse_bool_value(value: &str) -> Option<bool> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}
