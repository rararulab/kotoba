//! Database store wrapping `SQLite` connection pool.

use sqlx::{Sqlite, SqlitePool};

use super::err::Result;

/// Database store that manages the `SQLite` connection pool.
#[derive(Clone)]
pub struct DBStore {
    pool: SqlitePool,
}

impl DBStore {
    /// Create a new store from an existing pool.
    pub const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Get the underlying `SQLite` pool.
    pub const fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Acquire a connection from the pool.
    pub async fn acquire(&self) -> Result<sqlx::pool::PoolConnection<Sqlite>> {
        Ok(self.pool.acquire().await?)
    }
}

impl From<DBStore> for SqlitePool {
    fn from(store: DBStore) -> Self {
        store.pool
    }
}
