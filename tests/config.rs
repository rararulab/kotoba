mod common;

#[tokio::test]
async fn set_and_get_config_roundtrip() {
    let (db, _dir) = common::temp_db().await;

    db.set_config("test_key", "test_value")
        .await
        .expect("set config");

    let value = db.get_config("test_key").await.expect("get config");
    assert_eq!(value.as_deref(), Some("test_value"));
}

#[tokio::test]
async fn get_config_returns_none_for_missing_key() {
    let (db, _dir) = common::temp_db().await;

    let value = db.get_config("nonexistent_key").await.expect("get config");
    assert!(value.is_none());
}

#[tokio::test]
async fn all_config_returns_seeded_defaults() {
    let (db, _dir) = common::temp_db().await;

    let entries = db.all_config().await.expect("all config");

    // Schema seeds current_level, native_language, target_language
    assert!(entries.len() >= 3);

    let keys: Vec<&str> = entries.iter().map(|(k, _)| k.as_str()).collect();
    assert!(keys.contains(&"current_level"));
    assert!(keys.contains(&"native_language"));
    assert!(keys.contains(&"target_language"));

    // Verify default values
    let find = |key: &str| {
        entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(find("current_level"), Some("N5"));
    assert_eq!(find("native_language"), Some("zh"));
    assert_eq!(find("target_language"), Some("ja"));
}

#[tokio::test]
async fn overwriting_config_value() {
    let (db, _dir) = common::temp_db().await;

    db.set_config("current_level", "N3")
        .await
        .expect("set config");

    let value = db.get_config("current_level").await.expect("get config");
    assert_eq!(value.as_deref(), Some("N3"));
}
