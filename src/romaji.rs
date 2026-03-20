//! Kana to romaji conversion module.
//!
//! Delegates to the [`wana_kana`] crate for correct modified-Hepburn
//! romanization, including contextual ん handling (apostrophe before
//! vowels and y-row) and proper sokuon doubling.

use wana_kana::ConvertJapanese;

/// Convert a kana string to its romaji representation.
///
/// Uses modified Hepburn romanization via `wana_kana`. Non-kana
/// characters (kanji, ASCII, punctuation) pass through unchanged.
pub fn to_romaji(kana: &str) -> String {
    kana.to_romaji()
}

#[cfg(test)]
mod tests {
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
}
