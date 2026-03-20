//! Integration tests for grammar management flow.

mod common;

#[tokio::test]
async fn add_and_list_grammar() {
    let (db, _dir) = common::temp_db().await;
    db.add_grammar("〜ている", "progressive/state", "N5", None)
        .await
        .expect("add_grammar failed");

    let items = db.all_grammar(None).await.expect("all_grammar failed");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].pattern, "〜ている");
    assert_eq!(items[0].meaning, "progressive/state");
}

#[tokio::test]
async fn add_grammar_with_example() {
    let (db, _dir) = common::temp_db().await;
    db.add_grammar("〜てから", "after doing", "N4", Some("食べてから出かける"))
        .await
        .expect("add_grammar failed");

    let items = db.all_grammar(None).await.expect("all_grammar failed");
    assert_eq!(items[0].example.as_deref(), Some("食べてから出かける"));
}

#[tokio::test]
async fn grammar_level_filter() {
    let (db, _dir) = common::temp_db().await;
    db.add_grammar("〜ている", "progressive", "N5", None)
        .await
        .expect("add failed");
    db.add_grammar("〜てから", "after doing", "N4", None)
        .await
        .expect("add failed");
    db.add_grammar("〜ので", "because", "N5", None)
        .await
        .expect("add failed");

    let n5 = db
        .all_grammar(Some("N5"))
        .await
        .expect("filter by N5 failed");
    assert_eq!(n5.len(), 2);

    let n4 = db
        .all_grammar(Some("N4"))
        .await
        .expect("filter by N4 failed");
    assert_eq!(n4.len(), 1);
    assert_eq!(n4[0].pattern, "〜てから");
}

#[tokio::test]
async fn get_grammar_id_missing_pattern_errors() {
    let (db, _dir) = common::temp_db().await;
    let result = db.get_grammar_id("nonexistent").await;
    assert!(result.is_err(), "missing pattern should return error");
}
