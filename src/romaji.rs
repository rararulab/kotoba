//! Kana to romaji conversion module.
//!
//! Delegates to the [`wana_kana`] crate for correct modified-Hepburn
//! romanization, including contextual ん handling (apostrophe before
//! vowels and y-row) and proper sokuon doubling.

use serde_json::Value;
use wana_kana::ConvertJapanese;

/// Convert a kana string to its romaji representation.
///
/// Uses modified Hepburn romanization via `wana_kana`. Non-kana
/// characters (kanji, ASCII, punctuation) pass through unchanged.
pub fn to_romaji(kana: &str) -> String { kana.to_romaji() }

/// Convert Japanese text to natural sentence-level romaji.
///
/// For mixed kanji/kana input, this tries Google's romanization output first
/// (which includes word boundaries and long-vowel marks), then falls back to
/// local kana-only conversion on failure.
pub async fn to_romaji_natural(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let fallback = to_romaji(trimmed);
    if !contains_japanese(trimmed) {
        return fallback;
    }

    google_romanization(trimmed)
        .await
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback)
}

async fn google_romanization(text: &str) -> Option<String> {
    let response: Value = crate::http::client()
        .get("https://translate.googleapis.com/translate_a/single")
        .query(&[
            ("client", "gtx"),
            ("sl", "ja"),
            ("tl", "en"),
            ("dt", "t"),
            ("dt", "rm"),
            ("q", text),
        ])
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    parse_google_romanization(&response)
}

fn parse_google_romanization(value: &Value) -> Option<String> {
    let segments = value.get(0)?.as_array()?;
    for part in segments.iter().rev() {
        let candidate = part.get(3).and_then(Value::as_str)?;
        if !candidate.trim().is_empty() {
            return Some(candidate.trim().to_string());
        }
    }
    None
}

fn contains_japanese(text: &str) -> bool { text.chars().any(is_japanese_char) }

const fn is_japanese_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3040}'..='\u{30FF}'
            | '\u{31F0}'..='\u{31FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{3005}'
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn basic_hiragana() {
        assert_eq!(to_romaji("あいうえお"), "aiueo");
        assert_eq!(to_romaji("かきくけこ"), "kakikukeko");
        assert_eq!(to_romaji("さしすせそ"), "sashisuseso");
    }

    #[test]
    fn dakuten() {
        assert_eq!(to_romaji("がぎぐげご"), "gagigugego");
        assert_eq!(to_romaji("ざじずぜぞ"), "zajizuzezo");
        assert_eq!(to_romaji("だぢづでど"), "dajizudedo");
        assert_eq!(to_romaji("ばびぶべぼ"), "babibubebo");
    }

    #[test]
    fn handakuten() {
        assert_eq!(to_romaji("ぱぴぷぺぽ"), "papipupepo");
    }

    #[test]
    fn yoon() {
        assert_eq!(to_romaji("きゃ"), "kya");
        assert_eq!(to_romaji("しゃ"), "sha");
        assert_eq!(to_romaji("ちゃ"), "cha");
        assert_eq!(to_romaji("にゃ"), "nya");
        assert_eq!(to_romaji("じゃ"), "ja");
    }

    #[test]
    fn sokuon() {
        assert_eq!(to_romaji("がっこう"), "gakkou");
        assert_eq!(to_romaji("きって"), "kitte");
        assert_eq!(to_romaji("ざっし"), "zasshi");
    }

    #[test]
    fn choon() {
        assert_eq!(to_romaji("コーヒー"), "koohii");
        assert_eq!(to_romaji("ラーメン"), "raamen");
    }

    #[test]
    fn katakana_basic() {
        assert_eq!(to_romaji("アイウエオ"), "aiueo");
        assert_eq!(to_romaji("カタカナ"), "katakana");
    }

    #[test]
    fn mixed_input() {
        assert_eq!(to_romaji("東京タワー"), "東京tawaa");
        assert_eq!(to_romaji("hello"), "hello");
    }

    #[test]
    fn real_words() {
        assert_eq!(to_romaji("ありがとう"), "arigatou");
        assert_eq!(to_romaji("がっこう"), "gakkou");
        assert_eq!(to_romaji("コーヒー"), "koohii");
        assert_eq!(to_romaji("しんぶん"), "shinbun");
        assert_eq!(to_romaji("とうきょう"), "toukyou");
        assert_eq!(to_romaji("おはよう"), "ohayou");
    }

    #[test]
    fn n_before_vowel_gets_apostrophe() {
        assert_eq!(to_romaji("おんよみ"), "on'yomi");
        assert_eq!(to_romaji("しんいち"), "shin'ichi");
        assert_eq!(to_romaji("かんあい"), "kan'ai");
    }

    #[test]
    fn empty_string() {
        assert_eq!(to_romaji(""), "");
    }

    #[test]
    fn ascii_passthrough() {
        assert_eq!(to_romaji("abc123!@#"), "abc123!@#");
    }

    #[test]
    fn kanji_passthrough() {
        assert_eq!(to_romaji("漢字"), "漢字");
    }

    #[test]
    fn hiragana_rows() {
        assert_eq!(to_romaji("なにぬねの"), "naninuneno");
        assert_eq!(to_romaji("はひふへほ"), "hahifuheho");
        assert_eq!(to_romaji("まみむめも"), "mamimumemo");
        assert_eq!(to_romaji("やゆよ"), "yayuyo");
        assert_eq!(to_romaji("らりるれろ"), "rarirurero");
        assert_eq!(to_romaji("わをん"), "wawon");
    }

    #[test]
    fn multiple_sokuon() {
        assert_eq!(to_romaji("ばっさっき"), "bassakki");
    }

    #[test]
    fn mixed_hiragana_katakana() {
        assert_eq!(to_romaji("すしとラーメン"), "sushitoraamen");
    }

    #[test]
    fn katakana_dakuten_rows() {
        assert_eq!(to_romaji("ガギグゲゴ"), "gagigugego");
        assert_eq!(to_romaji("ザジズゼゾ"), "zajizuzezo");
        assert_eq!(to_romaji("ダヂヅデド"), "dajizudedo");
        assert_eq!(to_romaji("バビブベボ"), "babibubebo");
    }

    #[test]
    fn katakana_handakuten() {
        assert_eq!(to_romaji("パピプペポ"), "papipupepo");
    }

    #[test]
    fn parses_google_romanization_from_response() {
        let payload = json!([
            [
                [
                    "I'm really happy today! ",
                    "今日は本当に嬉しい！",
                    null,
                    null
                ],
                [
                    "But I'm a little nervous.",
                    "でも少し緊張してる。",
                    null,
                    null
                ],
                [
                    null,
                    null,
                    null,
                    "Kyō wa hontōni ureshī! Demo sukoshi kinchō shi teru."
                ]
            ],
            null,
            "ja"
        ]);

        let parsed = parse_google_romanization(&payload).expect("romanization should be parsed");
        assert_eq!(
            parsed,
            "Kyō wa hontōni ureshī! Demo sukoshi kinchō shi teru."
        );
    }

    #[test]
    fn google_romanization_missing_returns_none() {
        let payload = json!([[["x", "y", null, null]], null, "ja"]);
        assert!(parse_google_romanization(&payload).is_none());
    }

    #[test]
    fn detects_japanese_text() {
        assert!(contains_japanese("今日はいい天気"));
        assert!(contains_japanese("カタカナ"));
        assert!(!contains_japanese("hello world"));
    }
}
