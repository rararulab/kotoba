//! Vocabulary export in multiple formats.

use snafu::{ensure, ResultExt};

use crate::db::Database;
use crate::error::{self, Result};

/// Export all vocabulary in the specified format to stdout.
pub async fn export(db: &Database, format: &str) -> Result<()> {
    ensure!(
        matches!(format, "json" | "csv" | "anki"),
        error::UnknownFormatSnafu { format }
    );

    let vocab = db.all_vocabulary().await?;

    match format {
        "json" => {
            let json = serde_json::to_string_pretty(&vocab).context(error::JsonSnafu)?;
            println!("{json}");
        }
        "csv" => {
            println!("word,reading,meaning,level");
            for v in &vocab {
                println!("{},{},{},{}", v.word, v.reading, v.meaning, v.level);
            }
        }
        "anki" => {
            for v in &vocab {
                println!("{}\t{}  {}", v.word, v.reading, v.meaning);
            }
        }
        _ => unreachable!(),
    }

    Ok(())
}
