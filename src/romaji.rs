//! Kana to romaji conversion module.
//!
//! Converts hiragana and katakana strings to their romaji (Latin alphabet)
//! representation. Kanji and other non-kana characters pass through unchanged.

use std::{collections::HashMap, sync::LazyLock};

/// Static lookup table mapping kana strings to romaji.
///
/// Digraphs (two-character combinations like yōon) are included alongside
/// single characters. The caller must check digraphs before singles.
static KANA_MAP: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let entries: &[(&str, &str)] = &[
        // === Hiragana basic vowels ===
        ("あ", "a"),
        ("い", "i"),
        ("う", "u"),
        ("え", "e"),
        ("お", "o"),
        // === Hiragana k-row ===
        ("か", "ka"),
        ("き", "ki"),
        ("く", "ku"),
        ("け", "ke"),
        ("こ", "ko"),
        // === Hiragana s-row ===
        ("さ", "sa"),
        ("し", "shi"),
        ("す", "su"),
        ("せ", "se"),
        ("そ", "so"),
        // === Hiragana t-row ===
        ("た", "ta"),
        ("ち", "chi"),
        ("つ", "tsu"),
        ("て", "te"),
        ("と", "to"),
        // === Hiragana n-row ===
        ("な", "na"),
        ("に", "ni"),
        ("ぬ", "nu"),
        ("ね", "ne"),
        ("の", "no"),
        // === Hiragana h-row ===
        ("は", "ha"),
        ("ひ", "hi"),
        ("ふ", "fu"),
        ("へ", "he"),
        ("ほ", "ho"),
        // === Hiragana m-row ===
        ("ま", "ma"),
        ("み", "mi"),
        ("む", "mu"),
        ("め", "me"),
        ("も", "mo"),
        // === Hiragana y-row ===
        ("や", "ya"),
        ("ゆ", "yu"),
        ("よ", "yo"),
        // === Hiragana r-row ===
        ("ら", "ra"),
        ("り", "ri"),
        ("る", "ru"),
        ("れ", "re"),
        ("ろ", "ro"),
        // === Hiragana w-row + n ===
        ("わ", "wa"),
        ("を", "wo"),
        ("ん", "n"),
        // === Hiragana dakuten g-row ===
        ("が", "ga"),
        ("ぎ", "gi"),
        ("ぐ", "gu"),
        ("げ", "ge"),
        ("ご", "go"),
        // === Hiragana dakuten z-row ===
        ("ざ", "za"),
        ("じ", "ji"),
        ("ず", "zu"),
        ("ぜ", "ze"),
        ("ぞ", "zo"),
        // === Hiragana dakuten d-row ===
        ("だ", "da"),
        ("ぢ", "di"),
        ("づ", "du"),
        ("で", "de"),
        ("ど", "do"),
        // === Hiragana dakuten b-row ===
        ("ば", "ba"),
        ("び", "bi"),
        ("ぶ", "bu"),
        ("べ", "be"),
        ("ぼ", "bo"),
        // === Hiragana handakuten p-row ===
        ("ぱ", "pa"),
        ("ぴ", "pi"),
        ("ぷ", "pu"),
        ("ぺ", "pe"),
        ("ぽ", "po"),
        // === Hiragana yōon digraphs ===
        ("きゃ", "kya"),
        ("きゅ", "kyu"),
        ("きょ", "kyo"),
        ("しゃ", "sha"),
        ("しゅ", "shu"),
        ("しょ", "sho"),
        ("ちゃ", "cha"),
        ("ちゅ", "chu"),
        ("ちょ", "cho"),
        ("にゃ", "nya"),
        ("にゅ", "nyu"),
        ("にょ", "nyo"),
        ("ひゃ", "hya"),
        ("ひゅ", "hyu"),
        ("ひょ", "hyo"),
        ("みゃ", "mya"),
        ("みゅ", "myu"),
        ("みょ", "myo"),
        ("りゃ", "rya"),
        ("りゅ", "ryu"),
        ("りょ", "ryo"),
        ("ぎゃ", "gya"),
        ("ぎゅ", "gyu"),
        ("ぎょ", "gyo"),
        ("じゃ", "ja"),
        ("じゅ", "ju"),
        ("じょ", "jo"),
        ("びゃ", "bya"),
        ("びゅ", "byu"),
        ("びょ", "byo"),
        ("ぴゃ", "pya"),
        ("ぴゅ", "pyu"),
        ("ぴょ", "pyo"),
        // === Katakana basic vowels ===
        ("ア", "a"),
        ("イ", "i"),
        ("ウ", "u"),
        ("エ", "e"),
        ("オ", "o"),
        // === Katakana k-row ===
        ("カ", "ka"),
        ("キ", "ki"),
        ("ク", "ku"),
        ("ケ", "ke"),
        ("コ", "ko"),
        // === Katakana s-row ===
        ("サ", "sa"),
        ("シ", "shi"),
        ("ス", "su"),
        ("セ", "se"),
        ("ソ", "so"),
        // === Katakana t-row ===
        ("タ", "ta"),
        ("チ", "chi"),
        ("ツ", "tsu"),
        ("テ", "te"),
        ("ト", "to"),
        // === Katakana n-row ===
        ("ナ", "na"),
        ("ニ", "ni"),
        ("ヌ", "nu"),
        ("ネ", "ne"),
        ("ノ", "no"),
        // === Katakana h-row ===
        ("ハ", "ha"),
        ("ヒ", "hi"),
        ("フ", "fu"),
        ("ヘ", "he"),
        ("ホ", "ho"),
        // === Katakana m-row ===
        ("マ", "ma"),
        ("ミ", "mi"),
        ("ム", "mu"),
        ("メ", "me"),
        ("モ", "mo"),
        // === Katakana y-row ===
        ("ヤ", "ya"),
        ("ユ", "yu"),
        ("ヨ", "yo"),
        // === Katakana r-row ===
        ("ラ", "ra"),
        ("リ", "ri"),
        ("ル", "ru"),
        ("レ", "re"),
        ("ロ", "ro"),
        // === Katakana w-row + n ===
        ("ワ", "wa"),
        ("ヲ", "wo"),
        ("ン", "n"),
        // === Katakana dakuten g-row ===
        ("ガ", "ga"),
        ("ギ", "gi"),
        ("グ", "gu"),
        ("ゲ", "ge"),
        ("ゴ", "go"),
        // === Katakana dakuten z-row ===
        ("ザ", "za"),
        ("ジ", "ji"),
        ("ズ", "zu"),
        ("ゼ", "ze"),
        ("ゾ", "zo"),
        // === Katakana dakuten d-row ===
        ("ダ", "da"),
        ("ヂ", "di"),
        ("ヅ", "du"),
        ("デ", "de"),
        ("ド", "do"),
        // === Katakana dakuten b-row ===
        ("バ", "ba"),
        ("ビ", "bi"),
        ("ブ", "bu"),
        ("ベ", "be"),
        ("ボ", "bo"),
        // === Katakana handakuten p-row ===
        ("パ", "pa"),
        ("ピ", "pi"),
        ("プ", "pu"),
        ("ペ", "pe"),
        ("ポ", "po"),
        // === Katakana yōon digraphs ===
        ("キャ", "kya"),
        ("キュ", "kyu"),
        ("キョ", "kyo"),
        ("シャ", "sha"),
        ("シュ", "shu"),
        ("ショ", "sho"),
        ("チャ", "cha"),
        ("チュ", "chu"),
        ("チョ", "cho"),
        ("ニャ", "nya"),
        ("ニュ", "nyu"),
        ("ニョ", "nyo"),
        ("ヒャ", "hya"),
        ("ヒュ", "hyu"),
        ("ヒョ", "hyo"),
        ("ミャ", "mya"),
        ("ミュ", "myu"),
        ("ミョ", "myo"),
        ("リャ", "rya"),
        ("リュ", "ryu"),
        ("リョ", "ryo"),
        ("ギャ", "gya"),
        ("ギュ", "gyu"),
        ("ギョ", "gyo"),
        ("ジャ", "ja"),
        ("ジュ", "ju"),
        ("ジョ", "jo"),
        ("ビャ", "bya"),
        ("ビュ", "byu"),
        ("ビョ", "byo"),
        ("ピャ", "pya"),
        ("ピュ", "pyu"),
        ("ピョ", "pyo"),
        // === Special katakana combinations ===
        ("ヴァ", "va"),
        ("ヴィ", "vi"),
        ("ヴ", "vu"),
        ("ヴェ", "ve"),
        ("ヴォ", "vo"),
        ("ファ", "fa"),
        ("フィ", "fi"),
        ("フェ", "fe"),
        ("フォ", "fo"),
        ("ティ", "ti"),
        ("ディ", "di"),
        ("デュ", "dyu"),
        ("ウィ", "wi"),
        ("ウェ", "we"),
        ("ウォ", "wo"),
    ];

    entries.iter().copied().collect()
});

/// Mapping from romaji vowel characters to their string representation,
/// used by chōon (ー) to repeat the previous vowel.
fn last_vowel(romaji: &str) -> Option<char> {
    romaji
        .chars()
        .rev()
        .find(|c| matches!(c, 'a' | 'i' | 'u' | 'e' | 'o'))
}

/// Convert a kana string to its romaji representation.
///
/// Handles hiragana, katakana, yōon digraphs, sokuon (っ/ッ),
/// and chōon (ー). Non-kana characters (kanji, ASCII, punctuation)
/// pass through unchanged.
pub fn to_romaji(kana: &str) -> String {
    let chars: Vec<char> = kana.chars().collect();
    let mut result = String::with_capacity(kana.len());
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        // Handle sokuon: double the next consonant
        if ch == 'っ' || ch == 'ッ' {
            if i + 1 < chars.len() {
                // Look ahead to find what consonant to double
                let next_romaji = lookup_at(&chars, i + 1);
                if let Some(consonant) = next_romaji.and_then(|r| {
                    r.chars()
                        .next()
                        .filter(|c| !matches!(c, 'a' | 'i' | 'u' | 'e' | 'o'))
                }) {
                    result.push(consonant);
                } else {
                    result.push(ch);
                }
            } else {
                result.push(ch);
            }
            i += 1;
            continue;
        }

        // Handle chōon: repeat previous vowel
        if ch == 'ー' {
            if let Some(vowel) = last_vowel(&result) {
                result.push(vowel);
            }
            i += 1;
            continue;
        }

        // Try digraph lookup (2 chars) first
        if i + 1 < chars.len() {
            let digraph: String = chars[i..=i + 1].iter().collect();
            if let Some(romaji) = KANA_MAP.get(digraph.as_str()) {
                result.push_str(romaji);
                i += 2;
                continue;
            }
        }

        // Try single char lookup
        let single: String = chars[i..=i].iter().collect();
        if let Some(romaji) = KANA_MAP.get(single.as_str()) {
            result.push_str(romaji);
        } else {
            // Pass through non-kana characters unchanged
            result.push(ch);
        }
        i += 1;
    }

    result
}

/// Look up the romaji for the kana starting at position `pos`,
/// trying digraph first then single character.
fn lookup_at(chars: &[char], pos: usize) -> Option<&'static str> {
    // Try digraph
    if pos + 1 < chars.len() {
        let digraph: String = chars[pos..=pos + 1].iter().collect();
        if let Some(romaji) = KANA_MAP.get(digraph.as_str()) {
            return Some(romaji);
        }
    }
    // Try single
    let single: String = chars[pos..=pos].iter().collect();
    KANA_MAP.get(single.as_str()).copied()
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
        assert_eq!(to_romaji("だぢづでど"), "dadidudedo");
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
    fn special_katakana() {
        assert_eq!(to_romaji("ファ"), "fa");
        assert_eq!(to_romaji("ティ"), "ti");
        assert_eq!(to_romaji("ディ"), "di");
        assert_eq!(to_romaji("ヴ"), "vu");
    }

    #[test]
    fn mixed_input() {
        // Kanji passes through unchanged
        assert_eq!(to_romaji("東京タワー"), "東京tawaa");
        // ASCII passes through
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
    fn empty_string() {
        assert_eq!(to_romaji(""), "");
    }
}
