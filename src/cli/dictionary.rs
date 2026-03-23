//! Lightweight dictionary lookup for auto-filling vocabulary metadata.

use serde::Deserialize;
use snafu::ResultExt;

use crate::error::{self, Result};

/// Auto-filled vocabulary fields from dictionary lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoFillResult {
    pub reading: String,
    pub meaning: String,
}

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

/// Lookup reading and meaning from Jisho for a Japanese word.
pub async fn lookup(word: &str) -> Result<AutoFillResult> {
    let response: JishoResponse = crate::http::client()
        .get("https://jisho.org/api/v1/search/words")
        .query(&[("keyword", word)])
        .send()
        .await
        .context(error::HttpSnafu)?
        .json()
        .await
        .context(error::HttpSnafu)?;

    parse_lookup(word, &response).ok_or_else(|| {
        error::WordLookupSnafu {
            word:    word.to_string(),
            message: "no usable dictionary result".to_string(),
        }
        .build()
    })
}

fn parse_lookup(query: &str, response: &JishoResponse) -> Option<AutoFillResult> {
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

    #[test]
    fn parse_lookup_prefers_exact_word_match() {
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
        let result = parse_lookup("成功", &response).expect("lookup should succeed");

        assert_eq!(
            result,
            AutoFillResult {
                reading: "せいこう".to_string(),
                meaning: "success, achievement".to_string(),
            }
        );
    }

    #[test]
    fn parse_lookup_falls_back_to_first_entry() {
        let payload = r#"{
            "data": [
                {
                    "japanese": [{"word":"猫","reading":"ねこ"}],
                    "senses": [{"english_definitions":["cat"]}]
                }
            ]
        }"#;

        let response: JishoResponse = serde_json::from_str(payload).expect("valid fixture");
        let result = parse_lookup("未知語", &response).expect("fallback lookup should succeed");

        assert_eq!(
            result,
            AutoFillResult {
                reading: "ねこ".to_string(),
                meaning: "cat".to_string(),
            }
        );
    }
}
