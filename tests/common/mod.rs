//! Shared test helpers for integration tests.

use std::path::PathBuf;

use kotoba::db::Database;

/// Create a temporary `SQLite` database for test isolation.
///
/// Returns the database handle and the temp directory guard. The directory
/// is automatically cleaned up when the guard is dropped.
pub async fn temp_db() -> (Database, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let path: PathBuf = dir.path().join("test.db");
    let db = Database::open_at(path)
        .await
        .expect("failed to open temp db");
    db.init().await.expect("failed to init temp db");
    (db, dir)
}
