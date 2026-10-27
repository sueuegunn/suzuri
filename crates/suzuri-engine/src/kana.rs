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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_hiragana_only() {
        assert_eq!(hiragana_to_katakana("らーめんゔぁ1"), "ラーメンヴァ1");
    }
}
