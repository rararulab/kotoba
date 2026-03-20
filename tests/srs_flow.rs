mod common;

use kotoba::srs;

#[tokio::test]
async fn new_word_is_immediately_due() {
    let (db, _dir) = common::temp_db().await;

    db.add_vocabulary("猫", "ねこ", "cat", "N5")
        .await
        .expect("add vocab");

    // A word with no reviews should appear in due list
    let due = db.due_vocabulary().await.expect("due vocabulary");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].word, "猫");
    assert_eq!(due[0].due_at, "now");
}

#[tokio::test]
async fn review_quality_5_makes_word_not_immediately_due() {
    let (db, _dir) = common::temp_db().await;

    db.add_vocabulary("犬", "いぬ", "dog", "N5")
        .await
        .expect("add vocab");

    srs::record_review(&db, "犬", 5).await.expect("review q=5");

    // After a quality=5 review, interval is 1 day so word should not be due
    let due = db.due_vocabulary().await.expect("due vocabulary");
    assert!(
        due.is_empty(),
        "word should not be due after quality=5 review"
    );
}

#[tokio::test]
async fn review_quality_1_keeps_word_due_soon() {
    let (db, _dir) = common::temp_db().await;

    db.add_vocabulary("鳥", "とり", "bird", "N5")
        .await
        .expect("add vocab");

    srs::record_review(&db, "鳥", 1).await.expect("review q=1");

    // After a quality=1 review, interval is 0.1 days (~2.4 hours)
    // so the word should still be due now or very soon
    let due = db.due_vocabulary().await.expect("due vocabulary");
    assert_eq!(
        due.len(),
        1,
        "word should still be due after quality=1 review"
    );
}

#[tokio::test]
async fn first_review_records_expected_srs_state() {
    let (db, _dir) = common::temp_db().await;

    db.add_vocabulary("山", "やま", "mountain", "N5")
        .await
        .expect("add vocab");

    let item_id = db.get_vocabulary_id("山").await.expect("get id");

    // No review yet
    let before = db
        .get_latest_review(item_id, "vocabulary")
        .await
        .expect("get review");
    assert!(before.is_none(), "no review should exist yet");

    // Quality=5 first review: SM-2 gives interval=1.0, ease=2.5, reps=1
    srs::record_review(&db, "山", 5).await.expect("review");

    let after = db
        .get_latest_review(item_id, "vocabulary")
        .await
        .expect("get review");
    let (interval, ease, reps) = after.expect("review should exist");

    #[allow(clippy::float_cmp)] // Exact constants from first_review, no arithmetic drift
    {
        assert_eq!(interval, 1.0);
        assert_eq!(ease, 2.5);
    }
    assert_eq!(reps, 1);
}

#[tokio::test]
async fn grammar_review_works() {
    let (db, _dir) = common::temp_db().await;

    db.add_grammar("〜ます", "polite form", "N5", None)
        .await
        .expect("add grammar");

    // Grammar should be due initially
    let due = db.due_grammar().await.expect("due grammar");
    assert_eq!(due.len(), 1);

    srs::record_grammar_review(&db, "〜ます", 5)
        .await
        .expect("review grammar");

    // After review it should not be due
    let due = db.due_grammar().await.expect("due grammar after review");
    assert!(
        due.is_empty(),
        "grammar should not be due after quality=5 review"
    );
}
