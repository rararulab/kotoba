//! Database configuration.

use sqlx::sqlite::SqlitePoolOptions;

use super::db::DBStore;
use super::err::Result;

/// SQLite database configuration.
#[derive(Debug, Clone, bon::Builder, serde::Serialize, serde::Deserialize)]
pub struct DatabaseConfig {
    /// Maximum number of connections in the pool.
    #[serde(default = "default_max_connections")]
    #[builder(default = 5, getter)]
    pub max_connections: u32,
}

fn default_max_connections() -> u32 {
    5
}

impl DatabaseConfig {
    /// Open a SQLite database at the given URL.
    ///
    /// Sets WAL mode, busy timeout and foreign key pragmas.
    pub async fn open(&self, database_url: &str) -> Result<DBStore> {
        let pool = SqlitePoolOptions::new()
            .max_connections(self.max_connections)
            .connect(database_url)
            .await?;

        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA busy_timeout=5000")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA foreign_keys=ON")
            .execute(&pool)
            .await?;

        Ok(DBStore::new(pool))
    }
}
