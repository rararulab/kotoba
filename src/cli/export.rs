//! Vocabulary and grammar export in multiple formats.

use snafu::{ResultExt, ensure};

use crate::{
    db::Database,
    error::{self, Result},
};

/// Export vocabulary or grammar in the specified format to stdout.
pub async fn export(db: &Database, format: &str, grammar: bool) -> Result<()> {
    ensure!(
        matches!(format, "json" | "csv" | "anki"),
        error::UnknownFormatSnafu { format }
    );

    if grammar {
        export_grammar(db, format).await
    } else {
        export_vocabulary(db, format).await
    }
}

async fn export_vocabulary(db: &Database, format: &str) -> Result<()> {
    let vocab = db.all_vocabulary().await?;

    match format {
        "json" => {
            let json = serde_json::to_string_pretty(&vocab).context(error::JsonSnafu)?;
            println!("{json}");
        }
        "csv" => {
            println!("word,reading,romaji,meaning,level");
            for v in &vocab {
                println!(
                    "{},{},{},{},{}",
                    v.word, v.reading, v.romaji, v.meaning, v.level
                );
            }
        }
        "anki" => {
            for v in &vocab {
                println!("{}\t{} ({})  {}", v.word, v.reading, v.romaji, v.meaning);
            }
        }
        _ => unreachable!(),
    }

    Ok(())
}

async fn export_grammar(db: &Database, format: &str) -> Result<()> {
    let items = db.all_grammar(None).await?;

    match format {
        "json" => {
            let json = serde_json::to_string_pretty(&items).context(error::JsonSnafu)?;
            println!("{json}");
        }
        "csv" => {
            println!("pattern,meaning,level,example");
            for g in &items {
                println!(
                    "{},{},{},{}",
                    g.pattern,
                    g.meaning,
                    g.level,
                    g.example.as_deref().unwrap_or("")
                );
            }
        }
        "anki" => {
            for g in &items {
                println!("{}\t{}", g.pattern, g.meaning);
            }
        }
        _ => unreachable!(),
    }

    Ok(())
}
