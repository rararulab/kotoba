//! SM-2 inspired spaced repetition algorithm (simplified).
//!
//! Quality ratings:
//! - 5 — user uses word correctly unprompted (instant recall)
//! - 3 — user understands word in context
//! - 1 — user misuses word or asks what it means again

use snafu::ensure;

use crate::{
    db::Database,
    error::{self, Result},
};

/// Record a review for a vocabulary word and update SRS state.
pub async fn record_review(db: &Database, word: &str, quality: u8) -> Result<()> {
    let item_id = db.get_vocabulary_id(word).await?;
    record_review_item(db, item_id, "vocabulary", quality).await
}

/// Record a review for a grammar pattern and update SRS state.
pub async fn record_grammar_review(db: &Database, pattern: &str, quality: u8) -> Result<()> {
    let item_id = db.get_grammar_id(pattern).await?;
    record_review_item(db, item_id, "grammar", quality).await
}

/// Shared SRS review logic for both vocabulary and grammar items.
async fn record_review_item(
    db: &Database,
    item_id: i64,
    item_type: &str,
    quality: u8,
) -> Result<()> {
    ensure!(
        matches!(quality, 1 | 3 | 5),
        error::InvalidQualitySnafu { value: quality }
    );

    let prev = db.get_latest_review(item_id, item_type).await?;

    let (interval, ease, reps) = prev.map_or_else(
        || first_review(quality),
        |(prev_interval, prev_ease, prev_reps)| {
            next_review(quality, prev_interval, prev_ease, prev_reps)
        },
    );

    db.insert_review(item_id, item_type, quality, interval, ease, reps)
        .await
}

const fn first_review(quality: u8) -> (f64, f64, i32) {
    match quality {
        5 => (1.0, 2.5, 1),
        3 => (0.5, 2.5, 1),
        _ => (0.1, 2.5, 0),
    }
}

fn next_review(quality: u8, prev_interval: f64, prev_ease: f64, prev_reps: i32) -> (f64, f64, i32) {
    if quality < 3 {
        let ease = (prev_ease - 0.3).max(1.3);
        return (0.1, ease, 0);
    }

    let reps = prev_reps + 1;
    let ease = if quality == 5 {
        (prev_ease + 0.1).min(3.0)
    } else {
        prev_ease
    };

    let interval = match reps {
        1 => 1.0,
        2 => 3.0,
        _ => (prev_interval * ease).min(365.0),
    };

    (interval, ease, reps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp)] // Exact constants from first_review, no arithmetic drift
    fn first_review_quality_5() {
        let (interval, ease, reps) = first_review(5);
        assert_eq!(interval, 1.0);
        assert_eq!(ease, 2.5);
        assert_eq!(reps, 1);
    }

    #[test]
    fn first_review_quality_1_resets() {
        let (interval, _ease, reps) = first_review(1);
        assert!(interval < 0.5);
        assert_eq!(reps, 0);
    }

    #[test]
    fn failed_recall_resets_interval() {
        let (interval, ease, reps) = next_review(1, 10.0, 2.5, 5);
        assert!(interval < 0.5);
        assert!(ease < 2.5);
        assert_eq!(reps, 0);
    }

    #[test]
    fn successful_recall_increases_interval() {
        let (interval, ease, reps) = next_review(5, 3.0, 2.5, 3);
        assert!(interval > 3.0);
        assert!(ease > 2.5);
        assert_eq!(reps, 4);
    }

    #[test]
    fn interval_capped_at_365() {
        let (interval, ..) = next_review(5, 300.0, 2.5, 10);
        assert!(interval <= 365.0);
    }

    #[test]
    #[allow(clippy::float_cmp)] // Exact constants from first_review, no arithmetic drift
    fn first_review_quality_3() {
        let (interval, ease, reps) = first_review(3);
        assert_eq!(interval, 0.5);
        assert_eq!(ease, 2.5);
        assert_eq!(reps, 1);
    }

    #[test]
    fn quality_3_keeps_ease_stable() {
        let (_, ease, _) = next_review(3, 1.0, 2.5, 1);
        assert!(
            (ease - 2.5).abs() < f64::EPSILON,
            "quality=3 should not change ease"
        );
    }

    #[test]
    fn ease_floor_at_1_3() {
        // Repeated failures should not push ease below 1.3
        let (_, ease1, _) = next_review(1, 10.0, 1.5, 5);
        assert!(ease1 >= 1.3, "ease should not go below 1.3");

        let (_, ease2, _) = next_review(1, 10.0, 1.3, 5);
        assert!(
            (ease2 - 1.3).abs() < f64::EPSILON,
            "ease at floor should stay at 1.3"
        );
    }

    #[test]
    fn ease_ceiling_at_3_0() {
        // Repeated quality=5 should not push ease above 3.0
        let (_, ease, _) = next_review(5, 1.0, 2.95, 3);
        assert!(ease <= 3.0, "ease should not exceed 3.0");

        let (_, ease2, _) = next_review(5, 1.0, 3.0, 3);
        assert!(
            (ease2 - 3.0).abs() < f64::EPSILON,
            "ease at ceiling should stay at 3.0"
        );
    }

    #[test]
    fn multi_review_chain() {
        // Simulate a chain of successful reviews
        let (i1, e1, r1) = first_review(5);
        let (i2, e2, r2) = next_review(5, i1, e1, r1);
        let (i3, e3, r3) = next_review(5, i2, e2, r2);
        let (i4, ..) = next_review(5, i3, e3, r3);

        // Intervals should be monotonically increasing
        assert!(i2 > i1, "second interval should exceed first");
        assert!(i3 > i2, "third interval should exceed second");
        assert!(i4 > i3, "fourth interval should exceed third");
    }
}
