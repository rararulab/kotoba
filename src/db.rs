//! Application-level database operations for vocabulary, grammar, and reviews.

use std::path::{Path, PathBuf};

use serde::Serialize;
use snafu::ResultExt;

/// Convert a SQL COUNT result (`i64`) to `usize`.
///
/// SQL COUNT(*) is always non-negative and fits in usize on all platforms.
fn count_as_usize(n: i64) -> usize {
    usize::try_from(n).expect("SQL COUNT is non-negative and fits in usize")
}

use crate::{
    error::{self, Result},
    store::{DBStore, DatabaseConfig},
};

/// Application database wrapping the store layer.
pub struct Database {
    store: DBStore,
    path: PathBuf,
}

/// Current learning status returned by `kotoba status`.
#[derive(Debug, Serialize, bon::Builder)]
pub struct Status {
    pub level: String,
    pub vocabulary_count: usize,
    pub grammar_count: usize,
    pub due_reviews: usize,
}

/// A vocabulary entry for display or export.
#[derive(Debug, Serialize)]
pub struct VocabularyItem {
    pub word: String,
    pub reading: String,
    pub meaning: String,
    pub level: String,
}

/// An item due for SRS review.
#[derive(Debug, Serialize)]
pub struct ReviewItem {
    pub word: String,
    pub reading: String,
    pub meaning: String,
    pub item_type: String,
    pub due_at: String,
}

/// Learning progress statistics.
#[derive(Debug, Serialize, bon::Builder)]
pub struct Progress {
    pub total_vocabulary: usize,
    pub total_grammar: usize,
    pub mastered: usize,
    pub learning: usize,
    pub new: usize,
    pub reviews_count: usize,
}

fn default_db_dir() -> Result<PathBuf> {
    let dir = dirs::home_dir()
        .ok_or_else(|| error::HomeNotFoundSnafu.build())?
        .join(".kotoba");
    std::fs::create_dir_all(&dir).context(error::IoSnafu)?;
    Ok(dir)
}

impl Database {
    /// Open the default database at `~/.kotoba/kotoba.db`.
    pub async fn open_default() -> Result<Self> {
        let dir = default_db_dir()?;
        let path = dir.join("kotoba.db");
        let url = format!("sqlite:{}?mode=rwc", path.display());

        let config = DatabaseConfig::builder().build();
        let store = config.open(&url).await.context(error::StoreSnafu)?;

        Ok(Self { store, path })
    }

    /// Return the database file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    const fn pool(&self) -> &sqlx::SqlitePool {
        self.store.pool()
    }

    /// Create all tables and seed default profile values.
    pub async fn init(&self) -> Result<()> {
        sqlx::raw_sql(include_str!("schema.sql"))
            .execute(self.pool())
            .await
            .context(error::SqlxSnafu)?;
        Ok(())
    }

    /// Return current learning status.
    pub async fn status(&self) -> Result<Status> {
        let level = self
            .get_config("current_level")
            .await?
            .unwrap_or_else(|| "N5".to_string());
        let vocabulary_count = self.count("vocabulary").await?;
        let grammar_count = self.count("grammar").await?;
        let due_reviews = self.due_vocabulary().await?.len() + self.due_grammar().await?.len();

        Ok(Status::builder()
            .level(level)
            .vocabulary_count(vocabulary_count)
            .grammar_count(grammar_count)
            .due_reviews(due_reviews)
            .build())
    }

    /// Insert or update a vocabulary entry.
    pub async fn add_vocabulary(
        &self,
        word: &str,
        reading: &str,
        meaning: &str,
        level: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO vocabulary (word, reading, meaning, level) VALUES (?, ?, ?, ?)",
        )
        .bind(word)
        .bind(reading)
        .bind(meaning)
        .bind(level)
        .execute(self.pool())
        .await
        .context(error::SqlxSnafu)?;
        Ok(())
    }

    /// Look up a vocabulary item's database ID by word.
    pub async fn get_vocabulary_id(&self, word: &str) -> Result<i64> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM vocabulary WHERE word = ?")
            .bind(word)
            .fetch_optional(self.pool())
            .await
            .context(error::SqlxSnafu)?;

        row.map(|(id,)| id).ok_or_else(|| {
            error::WordNotFoundSnafu {
                word: word.to_string(),
            }
            .build()
        })
    }

    /// Fetch the most recent review state for an item.
    pub async fn get_latest_review(
        &self,
        item_id: i64,
        item_type: &str,
    ) -> Result<Option<(f64, f64, i32)>> {
        let row: Option<(f64, f64, i32)> = sqlx::query_as(
            "SELECT interval_days, ease, reps FROM reviews WHERE item_id = ? AND item_type = ? \
             ORDER BY reviewed_at DESC LIMIT 1",
        )
        .bind(item_id)
        .bind(item_type)
        .fetch_optional(self.pool())
        .await
        .context(error::SqlxSnafu)?;

        Ok(row)
    }

    /// Record a new review event.
    pub async fn insert_review(
        &self,
        item_id: i64,
        item_type: &str,
        quality: u8,
        interval: f64,
        ease: f64,
        reps: i32,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO reviews (item_id, item_type, quality, interval_days, ease, reps) VALUES \
             (?, ?, ?, ?, ?, ?)",
        )
        .bind(item_id)
        .bind(item_type)
        .bind(quality)
        .bind(interval)
        .bind(ease)
        .bind(reps)
        .execute(self.pool())
        .await
        .context(error::SqlxSnafu)?;
        Ok(())
    }

    /// Return vocabulary items due for review.
    pub async fn due_vocabulary(&self) -> Result<Vec<ReviewItem>> {
        let now = chrono::Utc::now()
            .naive_utc()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        let rows: Vec<VocabDueRow> = sqlx::query_as(
            "SELECT v.word, v.reading, v.meaning, r.reviewed_at, r.interval_days FROM vocabulary \
             v LEFT JOIN ( SELECT item_id, reviewed_at, interval_days, ROW_NUMBER() OVER \
             (PARTITION BY item_id ORDER BY reviewed_at DESC) as rn FROM reviews WHERE item_type \
             = 'vocabulary' ) r ON v.id = r.item_id AND r.rn = 1 WHERE r.reviewed_at IS NULL OR \
             datetime(r.reviewed_at, '+' || CAST(r.interval_days AS INTEGER) || ' days') <= ?",
        )
        .bind(&now)
        .fetch_all(self.pool())
        .await
        .context(error::SqlxSnafu)?;

        let items = rows
            .into_iter()
            .map(
                |(word, reading, meaning, reviewed_at, interval)| ReviewItem {
                    word,
                    reading,
                    meaning,
                    item_type: "vocabulary".to_string(),
                    due_at: format_due_at(reviewed_at.as_deref(), interval),
                },
            )
            .collect();

        Ok(items)
    }

    /// Return grammar items due for review.
    pub async fn due_grammar(&self) -> Result<Vec<ReviewItem>> {
        let now = chrono::Utc::now()
            .naive_utc()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        let rows: Vec<(String, String, Option<String>, Option<f64>)> = sqlx::query_as(
            "SELECT g.pattern, g.meaning, r.reviewed_at, r.interval_days FROM grammar g LEFT JOIN \
             ( SELECT item_id, reviewed_at, interval_days, ROW_NUMBER() OVER (PARTITION BY \
             item_id ORDER BY reviewed_at DESC) as rn FROM reviews WHERE item_type = 'grammar' ) \
             r ON g.id = r.item_id AND r.rn = 1 WHERE r.reviewed_at IS NULL OR \
             datetime(r.reviewed_at, '+' || CAST(r.interval_days AS INTEGER) || ' days') <= ?",
        )
        .bind(&now)
        .fetch_all(self.pool())
        .await
        .context(error::SqlxSnafu)?;

        let items = rows
            .into_iter()
            .map(|(pattern, meaning, reviewed_at, interval)| ReviewItem {
                word: pattern,
                reading: String::new(),
                meaning,
                item_type: "grammar".to_string(),
                due_at: format_due_at(reviewed_at.as_deref(), interval),
            })
            .collect();

        Ok(items)
    }

    /// Compute learning progress statistics.
    pub async fn progress(&self, weekly: bool) -> Result<Progress> {
        let total_vocabulary = self.count("vocabulary").await?;
        let total_grammar = self.count("grammar").await?;
        let mastered = self.count_by_mastery("reps >= 5").await?;
        let learning = self.count_by_mastery("reps < 5 AND reps > 0").await?;

        let reviewed_items: (i64,) = sqlx::query_as("SELECT COUNT(DISTINCT item_id) FROM reviews")
            .fetch_one(self.pool())
            .await
            .context(error::SqlxSnafu)?;
        let new =
            (total_vocabulary + total_grammar).saturating_sub(count_as_usize(reviewed_items.0));

        let date_filter = if weekly {
            "reviewed_at >= datetime('now', '-7 days')"
        } else {
            "reviewed_at >= datetime('now', 'start of day')"
        };
        let reviews_count: (i64,) =
            sqlx::query_as(&format!("SELECT COUNT(*) FROM reviews WHERE {date_filter}"))
                .fetch_one(self.pool())
                .await
                .context(error::SqlxSnafu)?;

        Ok(Progress::builder()
            .total_vocabulary(total_vocabulary)
            .total_grammar(total_grammar)
            .mastered(mastered)
            .learning(learning)
            .new(new)
            .reviews_count(count_as_usize(reviews_count.0))
            .build())
    }

    /// Return all vocabulary items for export.
    pub async fn all_vocabulary(&self) -> Result<Vec<VocabularyItem>> {
        let rows: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT word, reading, meaning, level FROM vocabulary ORDER BY created_at",
        )
        .fetch_all(self.pool())
        .await
        .context(error::SqlxSnafu)?;

        Ok(rows
            .into_iter()
            .map(|(word, reading, meaning, level)| VocabularyItem {
                word,
                reading,
                meaning,
                level,
            })
            .collect())
    }

    /// Set a user profile config value.
    pub async fn set_config(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query("INSERT OR REPLACE INTO user_profile (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(self.pool())
            .await
            .context(error::SqlxSnafu)?;
        Ok(())
    }

    /// Get a user profile config value.
    pub async fn get_config(&self, key: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM user_profile WHERE key = ?")
            .bind(key)
            .fetch_optional(self.pool())
            .await
            .context(error::SqlxSnafu)?;
        Ok(row.map(|(v,)| v))
    }

    /// Return all user profile config entries, sorted by key.
    pub async fn all_config(&self) -> Result<Vec<(String, String)>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM user_profile ORDER BY key")
                .fetch_all(self.pool())
                .await
                .context(error::SqlxSnafu)?;
        Ok(rows)
    }

    async fn count(&self, table: &str) -> Result<usize> {
        let row: (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(self.pool())
            .await
            .context(error::SqlxSnafu)?;
        Ok(count_as_usize(row.0))
    }

    async fn count_by_mastery(&self, condition: &str) -> Result<usize> {
        let row: (i64,) = sqlx::query_as(&format!(
            "SELECT COUNT(DISTINCT item_id) FROM reviews r1 WHERE {condition} AND reviewed_at = \
             (SELECT MAX(reviewed_at) FROM reviews r2 WHERE r2.item_id = r1.item_id AND \
             r2.item_type = r1.item_type)"
        ))
        .fetch_one(self.pool())
        .await
        .context(error::SqlxSnafu)?;
        Ok(count_as_usize(row.0))
    }
}

/// Row shape returned by the due-vocabulary query.
type VocabDueRow = (String, String, String, Option<String>, Option<f64>);

fn format_due_at(reviewed_at: Option<&str>, interval: Option<f64>) -> String {
    match (reviewed_at, interval) {
        (Some(ra), Some(iv)) => chrono::NaiveDateTime::parse_from_str(ra, "%Y-%m-%d %H:%M:%S")
            .map_or_else(
                |_| "now".to_string(),
                |dt| {
                    // Interval days are small positive values from SRS; truncation is intentional
                    #[allow(clippy::cast_possible_truncation)]
                    let days = iv as i64;
                    (dt + chrono::Duration::days(days))
                        .format("%Y-%m-%d")
                        .to_string()
                },
            ),
        _ => "now".to_string(),
    }
}
