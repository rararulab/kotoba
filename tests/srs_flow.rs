//! Integration tests for SRS (spaced repetition) review flow.

mod common;

use kotoba::srs;

#[tokio::test]
async fn new_word_is_due_for_review() {
    let (db, _dir) = common::temp_db().await;
    db.add_vocabulary("猫", "ねこ", "cat", "N5")
        .await
        .expect("add failed");

    let due = db.due_vocabulary().await.expect("due_vocabulary failed");
    assert_eq!(due.len(), 1, "newly added word should be due");
    assert_eq!(due[0].word, "猫");
}

#[tokio::test]
async fn quality_5_clears_from_due() {
    let (db, _dir) = common::temp_db().await;
    db.add_vocabulary("猫", "ねこ", "cat", "N5")
        .await
        .expect("add failed");

    srs::record_review(&db, "猫", 5)
        .await
        .expect("review failed");

    let due = db.due_vocabulary().await.expect("due_vocabulary failed");
    assert!(
        due.is_empty(),
        "word reviewed with quality=5 should not be immediately due"
    );
}

#[tokio::test]
async fn quality_1_keeps_word_due() {
    let (db, _dir) = common::temp_db().await;
    db.add_vocabulary("犬", "いぬ", "dog", "N5")
        .await
        .expect("add failed");

    srs::record_review(&db, "犬", 1)
        .await
        .expect("review failed");

    let due = db.due_vocabulary().await.expect("due_vocabulary failed");
    // quality=1 sets interval to 0.1 days (~2.4 hours), so it should still be due
    assert_eq!(
        due.len(),
        1,
        "word reviewed with quality=1 should remain due"
    );
}

#[tokio::test]
async fn multiple_reviews_recorded() {
    let (db, _dir) = common::temp_db().await;
    db.add_vocabulary("水", "みず", "water", "N5")
        .await
        .expect("add failed");

    // Record two successive reviews; both should succeed without error
    srs::record_review(&db, "水", 5)
        .await
        .expect("review 1 failed");
    srs::record_review(&db, "水", 3)
        .await
        .expect("review 2 failed");

    let item_id = db
        .get_vocabulary_id("水")
        .await
        .expect("get_vocabulary_id failed");
    let latest = db
        .get_latest_review(item_id, "vocabulary")
        .await
        .expect("get_latest_review failed");
    assert!(latest.is_some(), "should have at least one review recorded");
}

#[tokio::test]
async fn grammar_review_flow() {
    let (db, _dir) = common::temp_db().await;
    db.add_grammar("〜ている", "progressive", "N5", None)
        .await
        .expect("add grammar failed");

    let due_before = db.due_grammar().await.expect("due_grammar failed");
    assert_eq!(due_before.len(), 1, "new grammar should be due");

    srs::record_grammar_review(&db, "〜ている", 5)
        .await
        .expect("grammar review failed");

    let due_after = db.due_grammar().await.expect("due_grammar failed");
    assert!(
        due_after.is_empty(),
        "grammar reviewed with quality=5 should not be immediately due"
    );
}
