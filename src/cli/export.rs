//! Vocabulary and grammar export in multiple formats.

use snafu::ResultExt;

use crate::{
    cli::ExportFormat,
    db::Database,
    error::{self, Result},
};

/// Escape a field for RFC 4180 CSV output by wrapping in double quotes and
/// doubling any internal quote characters.
fn csv_field(s: &str) -> String { format!("\"{}\"", s.replace('"', "\"\"")) }

/// Export vocabulary or grammar in the specified format to stdout.
pub async fn export(db: &Database, format: &ExportFormat, grammar: bool) -> Result<()> {
    if grammar {
        export_grammar(db, format).await
    } else {
        export_vocabulary(db, format).await
    }
}

async fn export_vocabulary(db: &Database, format: &ExportFormat) -> Result<()> {
    let vocab = db.all_vocabulary(None).await?;

    match format {
        ExportFormat::Json => {
            let json = serde_json::to_string_pretty(&vocab).context(error::JsonSnafu)?;
            println!("{json}");
        }
        ExportFormat::Csv => {
            println!("word,reading,romaji,meaning,level");
            for v in &vocab {
                println!(
                    "{},{},{},{},{}",
                    csv_field(&v.word),
                    csv_field(&v.reading),
                    csv_field(&v.romaji),
                    csv_field(&v.meaning),
                    csv_field(&v.level),
                );
            }
        }
        ExportFormat::Anki => {
            for v in &vocab {
                println!("{}\t{} ({})  {}", v.word, v.reading, v.romaji, v.meaning);
            }
        }
    }

    Ok(())
}

async fn export_grammar(db: &Database, format: &ExportFormat) -> Result<()> {
    let items = db.all_grammar(None).await?;

    match format {
        ExportFormat::Json => {
            let json = serde_json::to_string_pretty(&items).context(error::JsonSnafu)?;
            println!("{json}");
        }
        ExportFormat::Csv => {
            println!("pattern,meaning,level,example");
            for g in &items {
                println!(
                    "{},{},{},{}",
                    csv_field(&g.pattern),
                    csv_field(&g.meaning),
                    csv_field(&g.level),
                    csv_field(g.example.as_deref().unwrap_or("")),
                );
            }
        }
        ExportFormat::Anki => {
            for g in &items {
                println!("{}\t{}", g.pattern, g.meaning);
            }
        }
    }

    Ok(())
}
