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
            println!("added: {}({}) = {}", word, reading, meaning);
        }
        Command::Seen { word, quality } => {
            srs::record_review(&db, &word, quality).await?;
            println!("recorded review: {} quality={}", word, quality);
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
            println!("{}", path.display());
        }
        Command::Config { key, value } => {
            db.set_config(&key, &value).await?;
            println!("set {} = {}", key, value);
        }
        Command::Export { format } => {
            cli::export::export(&db, &format).await?;
        }
    }

    Ok(())
}
