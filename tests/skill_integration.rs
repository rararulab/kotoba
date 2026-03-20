//! Tests verifying the API surface matches what the language-learning skill
//! expects from kotoba. These tests ensure the skill integration contract
//! remains stable.

mod common;

use kotoba::srs;

#[tokio::test]
async fn status_includes_level_field() {
    let (db, _dir) = common::temp_db().await;
    let status = db.status().await.expect("status failed");

    // The skill expects a `level` field in the status response
    let json = serde_json::to_value(&status).expect("serialize failed");
    assert!(
        json.get("level").is_some(),
        "status must include level field"
    );
    assert_eq!(
        json["level"].as_str(),
        Some("N5"),
        "default level should be N5"
    );
}

#[tokio::test]
async fn review_items_include_romaji() {
    let (db, _dir) = common::temp_db().await;
    db.add_vocabulary("食べる", "たべる", "to eat", "N5")
        .await
        .expect("add failed");

    let due = db.due_vocabulary().await.expect("due_vocabulary failed");
    assert_eq!(due.len(), 1);

    // The skill expects romaji in review items for pronunciation guidance
    assert_eq!(due[0].romaji, "taberu", "review item must include romaji");
    assert_eq!(due[0].item_type, "vocabulary");
}

#[tokio::test]
async fn full_skill_call_chain() {
    let (db, _dir) = common::temp_db().await;

    // 1. Status check — skill first queries status
    let status = db.status().await.expect("status failed");
    assert_eq!(status.vocabulary_count, 0);
    assert_eq!(status.due_reviews, 0);

    // 2. Add vocabulary — skill sends new words
    db.add_vocabulary("猫", "ねこ", "cat", "N5")
        .await
        .expect("add failed");

    // 3. Review — skill queries due items
    let due = db.due_vocabulary().await.expect("due failed");
    assert_eq!(due.len(), 1);

    // 4. Seen — skill records review result
    srs::record_review(&db, "猫", 5)
        .await
        .expect("review failed");

    // 5. Verify SRS state updated
    let due_after = db.due_vocabulary().await.expect("due failed");
    assert!(
        due_after.is_empty(),
        "reviewed word should not be immediately due"
    );

    // 6. Config set — skill configures preferences
    db.set_config("blending-intensity", "0.7")
        .await
        .expect("set_config failed");
    let val = db
        .get_config("blending-intensity")
        .await
        .expect("get_config failed");
    assert_eq!(val.as_deref(), Some("0.7"));
}

#[tokio::test]
async fn config_accepts_arbitrary_keys() {
    let (db, _dir) = common::temp_db().await;

    // The skill may set arbitrary config keys
    db.set_config("custom-skill-key", "value1")
        .await
        .expect("set failed");
    db.set_config("another.key", "value2")
        .await
        .expect("set failed");

    let v1 = db.get_config("custom-skill-key").await.expect("get failed");
    let v2 = db.get_config("another.key").await.expect("get failed");
    assert_eq!(v1.as_deref(), Some("value1"));
    assert_eq!(v2.as_deref(), Some("value2"));
}

#[tokio::test]
async fn grammar_review_flow() {
    let (db, _dir) = common::temp_db().await;

    // Skill adds grammar pattern
    db.add_grammar("〜ている", "progressive/state", "N5", Some("食べている"))
        .await
        .expect("add grammar failed");

    // Skill queries due grammar
    let due = db.due_grammar().await.expect("due_grammar failed");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].item_type, "grammar");
    assert_eq!(due[0].word, "〜ている");

    // Skill records grammar review
    srs::record_grammar_review(&db, "〜ている", 5)
        .await
        .expect("review failed");

    let due_after = db.due_grammar().await.expect("due_grammar failed");
    assert!(due_after.is_empty());
}

#[tokio::test]
async fn status_counts_update_correctly() {
    let (db, _dir) = common::temp_db().await;

    let s0 = db.status().await.expect("status failed");
    assert_eq!(s0.vocabulary_count, 0);
    assert_eq!(s0.grammar_count, 0);

    db.add_vocabulary("犬", "いぬ", "dog", "N5")
        .await
        .expect("add vocab failed");
    db.add_grammar("〜てから", "after doing", "N4", None)
        .await
        .expect("add grammar failed");

    let s1 = db.status().await.expect("status failed");
    assert_eq!(s1.vocabulary_count, 1);
    assert_eq!(s1.grammar_count, 1);
    // Both items are due (never reviewed)
    assert_eq!(s1.due_reviews, 2);

    // Review both items
    srs::record_review(&db, "犬", 5)
        .await
        .expect("review failed");
    srs::record_grammar_review(&db, "〜てから", 5)
        .await
        .expect("review failed");

    let s2 = db.status().await.expect("status failed");
    assert_eq!(s2.vocabulary_count, 1, "count should stay the same");
    assert_eq!(s2.grammar_count, 1, "count should stay the same");
    assert_eq!(s2.due_reviews, 0, "no items should be due after review");
}
