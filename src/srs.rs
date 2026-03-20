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
    ensure!(
        matches!(quality, 1 | 3 | 5),
        error::InvalidQualitySnafu { value: quality }
    );

    let item_id = db.get_vocabulary_id(word).await?;
    let prev = db.get_latest_review(item_id, "vocabulary").await?;

    let (interval, ease, reps) = prev.map_or_else(
        || first_review(quality),
        |(prev_interval, prev_ease, prev_reps)| {
            next_review(quality, prev_interval, prev_ease, prev_reps)
        },
    );

    db.insert_review(item_id, "vocabulary", quality, interval, ease, reps)
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
}
