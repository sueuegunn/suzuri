//! かな文字の変換

/// ひらがなをカタカナにする。ひらがな以外はそのまま残す。
pub fn hiragana_to_katakana(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'ぁ'..='ゖ' | 'ゝ' | 'ゞ' => char::from_u32(c as u32 + 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// 英字の書き分けの候補（小文字・先頭だけ大文字・全角）。英字を含まなければ空
pub fn ascii_variants(text: &str) -> Vec<String> {
    if !text.chars().any(|c| c.is_ascii_alphabetic()) {
        return Vec::new();
    }
    let mut capitalized = String::with_capacity(text.len());
    let mut prev_alpha = false;
    for c in text.chars() {
        let alpha = c.is_ascii_alphabetic();
        capitalized.push(if alpha && !prev_alpha {
            c.to_ascii_uppercase()
        } else {
            c.to_ascii_lowercase()
        });
        prev_alpha = alpha;
    }
    let full_width = text
        .chars()
        .map(|c| match c {
            '!'..='~' => char::from_u32(c as u32 + 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .collect();
    vec![text.to_ascii_lowercase(), capitalized, full_width]
}

/// 記号の書き分けの候補。ローマ字入力で日本語の記号になるものは元の半角と全角を、
/// そのほかの記号は半角と全角を入れ替えたものを返す。記号を含まなければ空。
/// 「ー」は語の中（「ラーメン」）にも現れるので、記号だけの文節のときに限って書き換える
pub fn symbol_variants(text: &str) -> Vec<String> {
    let only_symbols = text.chars().all(|c| !alternatives(c, true).is_empty());
    let alts: Vec<(char, Vec<String>)> = text.chars().map(|c| (c, alternatives(c, only_symbols))).collect();
    let count = alts.iter().map(|(_, a)| a.len()).max().unwrap_or(0);
    (0..count)
        .map(|k| {
            alts.iter()
                .map(|(c, a)| a.get(k).cloned().unwrap_or_else(|| c.to_string()))
                .collect()
        })
        .collect()
}

/// 1文字の記号の書き分け。記号でなければ空
fn alternatives(c: char, long_vowel: bool) -> Vec<String> {
    let fixed: &[&str] = match c {
        '。' => &[".", "．"],
        '、' => &[",", "，"],
        'ー' if long_vowel => &["-", "－"],
        '・' => &["/", "／"],
        '「' => &["[", "［"],
        '」' => &["]", "］"],
        '～' => &["~"],
        '？' => &["?"],
        '！' => &["!"],
        _ => &[],
    };
    if !fixed.is_empty() {
        return fixed.iter().map(|s| s.to_string()).collect();
    }
    let swapped = match c {
        '!'..='~' if !c.is_ascii_alphanumeric() => char::from_u32(c as u32 + 0xFEE0),
        '！'..='～' if !c.is_alphanumeric() => char::from_u32(c as u32 - 0xFEE0),
        _ => None,
    };
    swapped.map(|s| vec![s.to_string()]).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_hiragana_only() {
        assert_eq!(hiragana_to_katakana("らーめんゔぁ1"), "ラーメンヴァ1");
    }

    #[test]
    fn lists_ascii_variants() {
        assert_eq!(ascii_variants("NPMを"), ["npmを", "Npmを", "ＮＰＭを"]);
        assert_eq!(ascii_variants("MP3"), ["mp3", "Mp3", "ＭＰ３"]);
        assert!(ascii_variants("123").is_empty());
    }

    #[test]
    fn lists_symbol_variants() {
        assert_eq!(symbol_variants("。"), [".", "．"]);
        assert_eq!(symbol_variants("ー"), ["-", "－"]);
        assert_eq!(symbol_variants("@を"), ["＠を"]);
        assert_eq!(symbol_variants("（"), ["("]);
        // 語の中の長音は書き換えない
        assert!(symbol_variants("らーめん").is_empty());
        assert!(symbol_variants("かな").is_empty());
    }
}
