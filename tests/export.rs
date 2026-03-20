//! Integration tests for vocabulary and grammar export data structures.

mod common;

#[tokio::test]
async fn vocabulary_export_has_all_fields() {
    let (db, _dir) = common::temp_db().await;
    db.add_vocabulary("食べる", "たべる", "to eat", "N5")
        .await
        .expect("add failed");

    let items = db
        .all_vocabulary(None)
        .await
        .expect("all_vocabulary failed");
    assert_eq!(items.len(), 1);

    let v = &items[0];
    assert_eq!(v.word, "食べる");
    assert_eq!(v.reading, "たべる");
    assert_eq!(v.romaji, "taberu");
    assert_eq!(v.meaning, "to eat");
    assert_eq!(v.level, "N5");
}

#[tokio::test]
async fn grammar_export_has_all_fields() {
    let (db, _dir) = common::temp_db().await;
    db.add_grammar("〜ている", "progressive/state", "N5", Some("食べている"))
        .await
        .expect("add failed");

    let items = db.all_grammar(None).await.expect("all_grammar failed");
    assert_eq!(items.len(), 1);

    let g = &items[0];
    assert_eq!(g.pattern, "〜ている");
    assert_eq!(g.meaning, "progressive/state");
    assert_eq!(g.level, "N5");
    assert_eq!(g.example.as_deref(), Some("食べている"));
}

#[tokio::test]
async fn vocabulary_export_serializes_to_json() {
    let (db, _dir) = common::temp_db().await;
    db.add_vocabulary("猫", "ねこ", "cat", "N5")
        .await
        .expect("add failed");

    let items = db
        .all_vocabulary(None)
        .await
        .expect("all_vocabulary failed");
    let json = serde_json::to_string(&items).expect("serialization failed");
    assert!(json.contains("\"word\":\"猫\""));
    assert!(json.contains("\"romaji\":\"neko\""));
}
