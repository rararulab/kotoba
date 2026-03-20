mod cli;
mod db;
mod error;
mod romaji;
mod srs;
mod store;
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
            let path = cli::play::play_word(&db, &word).await?;
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
                cli::voice::list(&db).await?;
            }
            cli::VoiceAction::Set { name } => {
                cli::voice::set(&db, &name).await?;
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "action": "voice_set", "name": name})
                );
            }
            cli::VoiceAction::Add { repo_id } => {
                let result = cli::voice::add(&repo_id).await?;
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "action": "voice_add", "model": result.model, "path": result.path})
                );
            }
        },
        Command::Config { action } => match action {
            cli::ConfigAction::Set { key, value } => {
                db.set_config(&key, &value).await?;
                eprintln!("set {key} = {value}");
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "action": "config_set", "key": key, "value": value})
                );
            }
            cli::ConfigAction::Get { key } => {
                let value = db.get_config(&key).await?;
                let display_value = value.as_deref().unwrap_or("(not set)");
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "action": "config_get", "key": key, "value": display_value})
                );
            }
            cli::ConfigAction::List => {
                let entries = db.all_config().await?;
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
        Command::Export { format, grammar } => {
            cli::export::export(&db, &format, grammar).await?;
        }
    }

    Ok(())
}
