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
}
