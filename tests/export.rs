mod common;

#[tokio::test]
async fn vocabulary_data_has_expected_fields() {
    let (db, _dir) = common::temp_db().await;

    db.add_vocabulary("水", "みず", "water", "N5")
        .await
        .expect("add vocab");

    let items = db.all_vocabulary().await.expect("all vocabulary");
    assert_eq!(items.len(), 1);

    let item = &items[0];
    assert_eq!(item.word, "水");
    assert_eq!(item.reading, "みず");
    assert_eq!(item.romaji, "mizu");
    assert_eq!(item.meaning, "water");
    assert_eq!(item.level, "N5");
}

#[tokio::test]
async fn vocabulary_json_serialization() {
    let (db, _dir) = common::temp_db().await;

    db.add_vocabulary("火", "ひ", "fire", "N5")
        .await
        .expect("add vocab");

    let items = db.all_vocabulary().await.expect("all vocabulary");
    let json = serde_json::to_string(&items).expect("serialize");

    assert!(json.contains("\"word\":\"火\""));
    assert!(json.contains("\"reading\":\"ひ\""));
    assert!(json.contains("\"romaji\":\"hi\""));
    assert!(json.contains("\"meaning\":\"fire\""));
    assert!(json.contains("\"level\":\"N5\""));
}

#[tokio::test]
async fn grammar_data_has_expected_fields() {
    let (db, _dir) = common::temp_db().await;

    db.add_grammar(
        "〜から",
        "because",
        "N5",
        Some("寒いから — because it's cold"),
    )
    .await
    .expect("add grammar");

    let items = db.all_grammar(None).await.expect("all grammar");
    assert_eq!(items.len(), 1);

    let item = &items[0];
    assert_eq!(item.pattern, "〜から");
    assert_eq!(item.meaning, "because");
    assert_eq!(item.level, "N5");
    assert_eq!(
        item.example.as_deref(),
        Some("寒いから — because it's cold")
    );
}
