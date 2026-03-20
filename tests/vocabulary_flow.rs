//! Integration tests for vocabulary management flow.

mod common;

use kotoba::romaji;

#[tokio::test]
async fn add_and_retrieve_vocabulary() {
    let (db, _dir) = common::temp_db().await;
    db.add_vocabulary("食べる", "たべる", "to eat", "N5")
        .await
        .expect("add_vocabulary failed");

    let items = db.all_vocabulary().await.expect("all_vocabulary failed");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].word, "食べる");
    assert_eq!(items[0].reading, "たべる");
    assert_eq!(items[0].meaning, "to eat");
    assert_eq!(items[0].level, "N5");
}

#[tokio::test]
async fn romaji_auto_generated_on_add() {
    let (db, _dir) = common::temp_db().await;
    db.add_vocabulary("飲む", "のむ", "to drink", "N5")
        .await
        .expect("add_vocabulary failed");

    let items = db.all_vocabulary().await.expect("all_vocabulary failed");
    assert_eq!(items[0].romaji, romaji::to_romaji("のむ"));
    assert_eq!(items[0].romaji, "nomu");
}

#[tokio::test]
async fn duplicate_word_replaces_entry() {
    let (db, _dir) = common::temp_db().await;
    db.add_vocabulary("犬", "いぬ", "dog", "N5")
        .await
        .expect("first add failed");
    db.add_vocabulary("犬", "いぬ", "dog (pet)", "N4")
        .await
        .expect("second add failed");

    let items = db.all_vocabulary().await.expect("all_vocabulary failed");
    assert_eq!(items.len(), 1, "duplicate should replace, not insert");
    assert_eq!(items[0].meaning, "dog (pet)");
    assert_eq!(items[0].level, "N4");
}

#[tokio::test]
async fn get_vocabulary_id_missing_word_errors() {
    let (db, _dir) = common::temp_db().await;
    let result = db.get_vocabulary_id("nonexistent").await;
    assert!(result.is_err(), "missing word should return error");
}

#[tokio::test]
async fn multiple_vocabulary_entries() {
    let (db, _dir) = common::temp_db().await;
    db.add_vocabulary("猫", "ねこ", "cat", "N5")
        .await
        .expect("add failed");
    db.add_vocabulary("犬", "いぬ", "dog", "N5")
        .await
        .expect("add failed");
    db.add_vocabulary("魚", "さかな", "fish", "N4")
        .await
        .expect("add failed");

    let items = db.all_vocabulary().await.expect("all_vocabulary failed");
    assert_eq!(items.len(), 3);
}
