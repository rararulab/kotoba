//! Dictionary lookup abstraction for auto-filling vocabulary metadata.

use async_trait::async_trait;
use serde::Deserialize;
use snafu::ResultExt;

use crate::error::{self, Result};

/// Auto-filled vocabulary fields from dictionary lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoFillResult {
    pub reading: String,
    pub meaning: String,
}

/// Pluggable dictionary backend interface.
#[async_trait]
pub trait DictionaryBackend {
    /// Stable backend identifier for diagnostics.
    fn name(&self) -> &'static str;

    /// Lookup a word and return reading + meaning when found.
    ///
    /// `Ok(None)` means "not found in this backend", allowing fallback.
    async fn lookup(&self, word: &str) -> Result<Option<AutoFillResult>>;
}

/// Dictionary service that tries backends in order.
pub struct DictionaryService {
    backends: Vec<Box<dyn DictionaryBackend + Send + Sync>>,
}

impl Default for DictionaryService {
    fn default() -> Self { Self::with_backends(vec![Box::new(JishoBackend)]) }
}

impl DictionaryService {
    /// Create a service with explicitly ordered backends.
    pub fn with_backends(backends: Vec<Box<dyn DictionaryBackend + Send + Sync>>) -> Self {
        Self { backends }
    }

    /// Lookup by trying configured backends in order.
    pub async fn lookup(&self, word: &str) -> Result<AutoFillResult> {
        let mut failures: Vec<String> = Vec::new();

        for backend in &self.backends {
            match backend.lookup(word).await {
                Ok(Some(result)) => return Ok(result),
                Ok(None) => failures.push(format!("{}: no result", backend.name())),
                Err(err) => failures.push(format!("{}: {err}", backend.name())),
            }
        }

        let message = if failures.is_empty() {
            "no dictionary backend configured".to_string()
        } else {
            failures.join("; ")
        };

        error::WordLookupSnafu {
            word: word.to_string(),
            message,
        }
        .fail()
    }
}

/// Lookup reading and meaning using configured dictionary backends.
pub async fn lookup(word: &str) -> Result<AutoFillResult> {
    DictionaryService::default().lookup(word).await
}

/// Jisho backend implementation.
struct JishoBackend;

#[derive(Debug, Deserialize)]
struct JishoResponse {
    data: Vec<JishoEntry>,
}

#[derive(Debug, Deserialize)]
struct JishoEntry {
    japanese: Vec<JapaneseForm>,
    senses:   Vec<Sense>,
}

#[derive(Debug, Deserialize)]
struct JapaneseForm {
    word:    Option<String>,
    reading: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Sense {
    english_definitions: Vec<String>,
}

#[async_trait]
impl DictionaryBackend for JishoBackend {
    fn name(&self) -> &'static str { "jisho" }

    async fn lookup(&self, word: &str) -> Result<Option<AutoFillResult>> {
        let response: JishoResponse = crate::http::client()
            .get("https://jisho.org/api/v1/search/words")
            .query(&[("keyword", word)])
            .send()
            .await
            .context(error::HttpSnafu)?
            .json()
            .await
            .context(error::HttpSnafu)?;

        Ok(parse_jisho_lookup(word, &response))
    }
}

fn parse_jisho_lookup(query: &str, response: &JishoResponse) -> Option<AutoFillResult> {
    let entry = response
        .data
        .iter()
        .find(|e| e.japanese.iter().any(|j| j.word.as_deref() == Some(query)))
        .or_else(|| response.data.first())?;

    let reading = entry
        .japanese
        .iter()
        .find_map(|j| {
            (j.word.as_deref() == Some(query))
                .then(|| j.reading.clone())
                .flatten()
        })
        .or_else(|| entry.japanese.iter().find_map(|j| j.reading.clone()))?;

    let meaning = entry
        .senses
        .first()
        .map(|sense| sense.english_definitions.join(", "))
        .filter(|joined| !joined.is_empty())?;

    Some(AutoFillResult { reading, meaning })
}

#[cfg(test)]
mod tests {
    use super::*;

    enum StaticBehavior {
        Hit {
            reading: &'static str,
            meaning: &'static str,
        },
        Miss,
        Fail,
    }

    struct StaticBackend {
        name:     &'static str,
        behavior: StaticBehavior,
    }

    #[async_trait]
    impl DictionaryBackend for StaticBackend {
        fn name(&self) -> &'static str { self.name }

        async fn lookup(&self, _word: &str) -> Result<Option<AutoFillResult>> {
            match self.behavior {
                StaticBehavior::Hit { reading, meaning } => Ok(Some(AutoFillResult {
                    reading: reading.to_string(),
                    meaning: meaning.to_string(),
                })),
                StaticBehavior::Miss => Ok(None),
                StaticBehavior::Fail => error::WordLookupSnafu {
                    word:    "x".to_string(),
                    message: "upstream error".to_string(),
                }
                .fail(),
            }
        }
    }

    #[test]
    fn parse_jisho_lookup_prefers_exact_word_match() {
        let payload = r#"{
            "data": [
                {
                    "japanese": [{"word":"成功者","reading":"せいこうしゃ"}],
                    "senses": [{"english_definitions":["successful person"]}]
                },
                {
                    "japanese": [{"word":"成功","reading":"せいこう"}],
                    "senses": [{"english_definitions":["success","achievement"]}]
                }
            ]
        }"#;

        let response: JishoResponse = serde_json::from_str(payload).expect("valid fixture");
        let result = parse_jisho_lookup("成功", &response).expect("lookup should succeed");

        assert_eq!(
            result,
            AutoFillResult {
                reading: "せいこう".to_string(),
                meaning: "success, achievement".to_string(),
            }
        );
    }

    #[test]
    fn parse_jisho_lookup_falls_back_to_first_entry() {
        let payload = r#"{
            "data": [
                {
                    "japanese": [{"word":"猫","reading":"ねこ"}],
                    "senses": [{"english_definitions":["cat"]}]
                }
            ]
        }"#;

        let response: JishoResponse = serde_json::from_str(payload).expect("valid fixture");
        let result =
            parse_jisho_lookup("未知語", &response).expect("fallback lookup should succeed");

        assert_eq!(
            result,
            AutoFillResult {
                reading: "ねこ".to_string(),
                meaning: "cat".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn service_uses_first_successful_backend() {
        let service = DictionaryService::with_backends(vec![
            Box::new(StaticBackend {
                name:     "primary",
                behavior: StaticBehavior::Hit {
                    reading: "せいこう",
                    meaning: "success",
                },
            }),
            Box::new(StaticBackend {
                name:     "secondary",
                behavior: StaticBehavior::Hit {
                    reading: "fallback",
                    meaning: "fallback",
                },
            }),
        ]);

        let result = service.lookup("成功").await.expect("lookup should succeed");
        assert_eq!(result.reading, "せいこう");
        assert_eq!(result.meaning, "success");
    }

    #[tokio::test]
    async fn service_falls_back_when_backend_returns_none() {
        let service = DictionaryService::with_backends(vec![
            Box::new(StaticBackend {
                name:     "empty",
                behavior: StaticBehavior::Miss,
            }),
            Box::new(StaticBackend {
                name:     "fallback",
                behavior: StaticBehavior::Hit {
                    reading: "ねこ",
                    meaning: "cat",
                },
            }),
        ]);

        let result = service
            .lookup("猫")
            .await
            .expect("fallback lookup should succeed");
        assert_eq!(result.reading, "ねこ");
        assert_eq!(result.meaning, "cat");
    }

    #[tokio::test]
    async fn service_returns_word_lookup_when_all_backends_fail() {
        let service = DictionaryService::with_backends(vec![
            Box::new(StaticBackend {
                name:     "empty",
                behavior: StaticBehavior::Miss,
            }),
            Box::new(StaticBackend {
                name:     "broken",
                behavior: StaticBehavior::Fail,
            }),
        ]);

        let err = service
            .lookup("未知語")
            .await
            .expect_err("lookup should fail");
        let msg = err.to_string();
        assert!(msg.contains("empty: no result"));
        assert!(msg.contains("broken:"));
    }
}
