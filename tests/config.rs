//! Integration tests for DB config (runtime state) and TOML config.

mod common;

#[tokio::test]
async fn set_get_roundtrip() {
    let (db, _dir) = common::temp_db().await;
    db.set_config("current_level", "N3")
        .await
        .expect("set_config failed");

    let value = db
        .get_config("current_level")
        .await
        .expect("get_config failed")
        .expect("should have value");
    assert_eq!(value, "N3");
}

#[tokio::test]
async fn get_missing_key_returns_none() {
    let (db, _dir) = common::temp_db().await;
    let value = db
        .get_config("nonexistent_key")
        .await
        .expect("get_config failed");
    assert!(value.is_none(), "missing key should return None");
}

#[tokio::test]
async fn seeded_defaults_present() {
    let (db, _dir) = common::temp_db().await;

    let level = db
        .get_config("current_level")
        .await
        .expect("get_config failed")
        .expect("current_level should be seeded");
    assert_eq!(level, "N5");

    let native = db
        .get_config("native_language")
        .await
        .expect("get_config failed")
        .expect("native_language should be seeded");
    assert_eq!(native, "zh");
}

#[tokio::test]
async fn overwrite_existing_key() {
    let (db, _dir) = common::temp_db().await;
    db.set_config("current_level", "N4")
        .await
        .expect("first set failed");
    db.set_config("current_level", "N3")
        .await
        .expect("second set failed");

    let value = db
        .get_config("current_level")
        .await
        .expect("get_config failed")
        .expect("should have value");
    assert_eq!(value, "N3");
}
