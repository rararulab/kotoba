mod common;

use kotoba::error::KotobaError;

#[tokio::test]
async fn add_vocabulary_appears_in_all() {
    let (db, _dir) = common::temp_db().await;

    db.add_vocabulary("食べる", "たべる", "to eat", "N5")
        .await
        .expect("add vocab");

    let items = db.all_vocabulary().await.expect("all vocabulary");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].word, "食べる");
    assert_eq!(items[0].reading, "たべる");
    assert_eq!(items[0].meaning, "to eat");
    assert_eq!(items[0].level, "N5");
}

#[tokio::test]
async fn romaji_auto_generated_from_reading() {
    let (db, _dir) = common::temp_db().await;

    db.add_vocabulary("飲む", "のむ", "to drink", "N5")
        .await
        .expect("add vocab");

    let items = db.all_vocabulary().await.expect("all vocabulary");
    assert_eq!(items[0].romaji, "nomu");
}

#[tokio::test]
async fn duplicate_word_replaces() {
    let (db, _dir) = common::temp_db().await;

    db.add_vocabulary("見る", "みる", "to look", "N5")
        .await
        .expect("add first");
    db.add_vocabulary("見る", "みる", "to see", "N4")
        .await
        .expect("add duplicate");

    let items = db.all_vocabulary().await.expect("all vocabulary");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].meaning, "to see");
    assert_eq!(items[0].level, "N4");
}

#[tokio::test]
async fn get_vocabulary_id_returns_correct_id() {
    let (db, _dir) = common::temp_db().await;

    db.add_vocabulary("走る", "はしる", "to run", "N5")
        .await
        .expect("add vocab");

    let id = db.get_vocabulary_id("走る").await.expect("get id");
    assert!(id > 0);
}

#[tokio::test]
async fn get_vocabulary_id_returns_error_for_missing_word() {
    let (db, _dir) = common::temp_db().await;

    let result = db.get_vocabulary_id("存在しない").await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        KotobaError::WordNotFound { .. }
    ));
}
