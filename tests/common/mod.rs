use kotoba::db::Database;

/// Create an isolated temporary database for testing.
///
/// Returns the database handle and the temp directory guard.
/// The directory (and database file) is deleted when `TempDir` is dropped.
pub async fn temp_db() -> (Database, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("test.db");
    let db = Database::open_at(path).await.expect("open temp db");
    db.init().await.expect("init db");
    (db, dir)
}
