mod common;

use kotoba::error::KotobaError;

#[tokio::test]
async fn add_grammar_appears_in_all() {
    let (db, _dir) = common::temp_db().await;

    db.add_grammar("〜ている", "ongoing action", "N5", None)
        .await
        .expect("add grammar");

    let items = db.all_grammar(None).await.expect("all grammar");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].pattern, "〜ている");
    assert_eq!(items[0].meaning, "ongoing action");
    assert_eq!(items[0].level, "N5");
    assert!(items[0].example.is_none());
}

#[tokio::test]
async fn add_grammar_with_example() {
    let (db, _dir) = common::temp_db().await;

    db.add_grammar(
        "〜たい",
        "want to do",
        "N5",
        Some("食べたい — I want to eat"),
    )
    .await
    .expect("add grammar");

    let items = db.all_grammar(None).await.expect("all grammar");
    assert_eq!(
        items[0].example.as_deref(),
        Some("食べたい — I want to eat")
    );
}

#[tokio::test]
async fn all_grammar_with_level_filter() {
    let (db, _dir) = common::temp_db().await;

    db.add_grammar("〜ている", "ongoing action", "N5", None)
        .await
        .expect("add N5");
    db.add_grammar("〜ようにする", "try to do", "N3", None)
        .await
        .expect("add N3");

    let n5_items = db.all_grammar(Some("N5")).await.expect("filter N5");
    assert_eq!(n5_items.len(), 1);
    assert_eq!(n5_items[0].pattern, "〜ている");

    let n3_items = db.all_grammar(Some("N3")).await.expect("filter N3");
    assert_eq!(n3_items.len(), 1);
    assert_eq!(n3_items[0].pattern, "〜ようにする");

    let all_items = db.all_grammar(None).await.expect("all");
    assert_eq!(all_items.len(), 2);
}

#[tokio::test]
async fn get_grammar_id_returns_error_for_missing_pattern() {
    let (db, _dir) = common::temp_db().await;

    let result = db.get_grammar_id("〜nonexistent").await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        KotobaError::GrammarNotFound { .. }
    ));
}
