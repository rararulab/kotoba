//! Integration tests verifying kotoba's API matches the contract expected by
//! the `language-learning` skill in rara-skills. The skill calls kotoba via CLI
//! and parses its output; these tests exercise the underlying DB API to ensure
//! the data shapes and call chains remain compatible.

mod common;

/// Verify status output includes level field for skill to determine blending ratio.
#[tokio::test]
async fn status_includes_level() {
    let (db, _dir) = common::temp_db().await;
    let status = db.status().await.unwrap();
    assert_eq!(status.level, "N5"); // Default level
}

/// Verify review output includes romaji (skill uses it for annotations).
#[tokio::test]
async fn review_items_include_romaji() {
    let (db, _dir) = common::temp_db().await;
    db.add_vocabulary("成功", "せいこう", "success", "N5")
        .await
        .unwrap();
    let due = db.due_vocabulary().await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].romaji, "seikou");
    assert!(!due[0].reading.is_empty());
    assert!(!due[0].meaning.is_empty());
}

/// Simulate the complete skill call chain: status -> add -> review -> seen -> verify SRS -> config.
#[tokio::test]
async fn full_skill_call_chain() {
    let (db, _dir) = common::temp_db().await;

    // 1. Status check (skill startup)
    let status = db.status().await.unwrap();
    assert_eq!(status.level, "N5");
    assert_eq!(status.due_reviews, 0);

    // 2. Add new word (skill teaches during conversation)
    db.add_vocabulary("成功", "せいこう", "success", "N5")
        .await
        .unwrap();

    // 3. Check review (skill checks what's due)
    let due = db.due_vocabulary().await.unwrap();
    assert_eq!(due.len(), 1);

    // 4. Record review (skill observes user understands)
    kotoba::srs::record_review(&db, "成功", 5).await.unwrap();

    // 5. Verify SRS scheduled (word not due immediately after quality=5)
    let due_after = db.due_vocabulary().await.unwrap();
    assert!(
        due_after.is_empty(),
        "word should not be due immediately after quality=5 review"
    );

    // 6. Config set (skill adjusts blending intensity)
    db.set_config("blending-intensity", "low").await.unwrap();
    let val = db.get_config("blending-intensity").await.unwrap();
    assert_eq!(val, Some("low".to_string()));
}

/// Verify config set accepts arbitrary keys (skill uses custom keys like blending-intensity).
#[tokio::test]
async fn config_accepts_arbitrary_keys() {
    let (db, _dir) = common::temp_db().await;
    db.set_config("blending-intensity", "low").await.unwrap();
    db.set_config("custom-key", "custom-value").await.unwrap();
    let val = db.get_config("custom-key").await.unwrap();
    assert_eq!(val, Some("custom-value".to_string()));
}

/// Verify grammar review flow works for skill's grammar teaching.
#[tokio::test]
async fn grammar_review_flow() {
    let (db, _dir) = common::temp_db().await;
    db.add_grammar("～てください", "please do ~", "N5", Some("座ってください"))
        .await
        .unwrap();
    let due = db.due_grammar().await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].word, "～てください");

    kotoba::srs::record_grammar_review(&db, "～てください", 3)
        .await
        .unwrap();
    // After quality=3, interval is 0.5 days so item should not be immediately due
}

/// Verify status counts update correctly as skill adds items.
#[tokio::test]
async fn status_counts_reflect_additions() {
    let (db, _dir) = common::temp_db().await;

    let s1 = db.status().await.unwrap();
    assert_eq!(s1.vocabulary_count, 0);
    assert_eq!(s1.grammar_count, 0);

    db.add_vocabulary("猫", "ねこ", "cat", "N5").await.unwrap();
    db.add_grammar("～です", "is/am/are", "N5", None)
        .await
        .unwrap();

    let s2 = db.status().await.unwrap();
    assert_eq!(s2.vocabulary_count, 1);
    assert_eq!(s2.grammar_count, 1);
    // Both new items are due for review
    assert_eq!(s2.due_reviews, 2);
}
