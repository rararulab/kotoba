mod cli;
mod db;
mod error;
mod srs;
mod store;

use clap::Parser;
use cli::{Cli, Command};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let db = db::Database::open_default().await?;

    match cli.command {
        Command::Init => {
            db.init().await?;
            println!("kotoba initialized at {}", db.path().display());
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
            println!("added: {word}({reading}) = {meaning}");
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
                println!("added grammar: {pattern} = {meaning}");
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
                println!("recorded grammar review: {word} quality={quality}");
            } else {
                srs::record_review(&db, &word, quality).await?;
                println!("recorded review: {word} quality={quality}");
            }
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
        Command::Play { word, backend } => {
            let path = cli::play::play_word(&word, &backend).await?;
            println!("{}", path.display());
        }
        Command::Setup => {
            cli::setup::run(&db).await?;
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
            }
            cli::VoiceAction::Add { repo_id } => {
                cli::voice::add(&repo_id).await?;
            }
        },
        Command::Config { action } => match action {
            cli::ConfigAction::Set { key, value } => {
                db.set_config(&key, &value).await?;
                println!("set {key} = {value}");
            }
            cli::ConfigAction::Get { key } => {
                let value = db.get_config(&key).await?;
                println!("{}", value.as_deref().unwrap_or("(not set)"));
            }
            cli::ConfigAction::List => {
                let entries = db.all_config().await?;
                for (key, value) in entries {
                    println!("{key} = {value}");
                }
            }
        },
        Command::Export { format, grammar } => {
            cli::export::export(&db, &format, grammar).await?;
        }
    }

    Ok(())
}
