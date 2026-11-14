//! 打っている文の続きの予測（サジェスト）。docs/design.md の「続きの予測」。
//!
//! 読みの末尾が、ユーザー辞書の語や学習した語の読みの先頭と一致したら、その語を続きとして出す。
//! 変換中に呼ぶので、メモリ上の表だけを見る。

use std::collections::{HashMap, HashSet};

use crate::UserData;
use crate::user_dict::fold_char;

/// 末尾の何文字から一致を探すか。1文字だと当たりすぎる
pub const MIN_MATCH_CHARS: usize = 2;
/// 末尾の何文字まで見るか。ユーザー辞書の読みの上限に合わせる
pub const MAX_MATCH_CHARS: usize = crate::user_dict::MAX_READING_CHARS;

/// 続きの候補
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// 読みの末尾の何文字が、語の読みの先頭と一致したか
    pub matched: usize,
    /// 語の読み全体。末尾の matched 文字のあとに、読みの matched 文字目以降を足すと、この語の読みになる
    pub reading: String,
    /// 語の表記
    pub surface: String,
}

/// 読み `kana` の続きを1つ返す。末尾と一致する文字数が長いものを選び、同じならユーザー辞書の語、
/// 次に新しく学習した語を選ぶ。一致する語がなければ None
pub fn suggest(kana: &str, user: &UserData) -> Option<Suggestion> {
    let folded: Vec<char> = kana.chars().map(fold_char).collect();
    let n = folded.len();
    // 末尾の k 文字（そろえたもの）→ k
    let tails: HashMap<String, usize> = (MIN_MATCH_CHARS..=n.min(MAX_MATCH_CHARS))
        .map(|k| (folded[n - k..].iter().collect(), k))
        .collect();
    if tails.is_empty() {
        return None;
    }
    // 末尾の各一致の先頭2文字。語の読みの先頭2文字がこのどれでもなければ、文字列を作らずに飛ばす
    let heads: HashSet<(char, char)> = (MIN_MATCH_CHARS..=n.min(MAX_MATCH_CHARS))
        .map(|k| (folded[n - k], folded[n - k + 1]))
        .collect();
    // (一致した文字数, 出どころの優先, 新しさ) が大きいものを選ぶ
    let dictionary = user
        .dictionary
        .words()
        .iter()
        .enumerate()
        .map(|(i, w)| (w.reading.as_str(), w.surface.as_str(), 1u8, i as u64));
    let history = user.history.entries().map(|(r, s, t)| (r, s, 0u8, t));
    let mut best: Option<((usize, u8, u64), &str, &str)> = None;
    for (reading, surface, source, recency) in dictionary.chain(history) {
        let mut head = reading.chars().map(fold_char);
        let (Some(a), Some(b)) = (head.next(), head.next()) else {
            continue;
        };
        if !heads.contains(&(a, b)) {
            continue;
        }
        let Some(k) = matched_chars(reading, &tails, n) else {
            continue;
        };
        let key = (k, source, recency);
        if best.as_ref().is_none_or(|(b, _, _)| key > *b) {
            best = Some((key, reading, surface));
        }
    }
    best.map(|((matched, _, _), reading, surface)| Suggestion {
        matched,
        reading: reading.to_owned(),
        surface: surface.to_owned(),
    })
}

/// 語の読みの先頭が、読みの末尾（`tails`）と一致する一番長い文字数。続きが残るよう、語の読みより短い一致だけを見る
fn matched_chars(reading: &str, tails: &HashMap<String, usize>, n: usize) -> Option<usize> {
    let chars: Vec<char> = reading.chars().map(fold_char).collect();
    let longest = chars.len().saturating_sub(1).min(n).min(MAX_MATCH_CHARS);
    let mut prefix: String = chars[..longest].iter().collect();
    for k in (MIN_MATCH_CHARS..=longest).rev() {
        prefix.truncate(chars[..k].iter().map(|c| c.len_utf8()).sum());
        if tails.get(prefix.as_str()) == Some(&k) {
            return Some(k);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Committed, UserPos};

    fn learn(user: &mut UserData, reading: &str, surface: &str) {
        user.history.commit(&[Committed {
            reading: reading.into(),
            surface: surface.into(),
            was_first: false,
        }]);
    }

    #[test]
    fn suggests_the_rest_of_a_learned_word() {
        let mut user = UserData::default();
        learn(&mut user, "ありがとうございます", "ありがとうございます");
        let s = suggest("きょうはありが", &user).unwrap();
        assert_eq!(s.matched, 3);
        assert_eq!(s.reading, "ありがとうございます");
        // 1文字だけ・全部打ち終えた・一致しないときは出さない
        assert_eq!(suggest("あ", &user), None);
        assert_eq!(suggest("ありがとうございます", &user), None);
        assert_eq!(suggest("いぬ", &user), None);
    }

    #[test]
    fn prefers_longer_match_then_user_dictionary_then_newer() {
        let mut user = UserData::default();
        learn(&mut user, "かいぎしつ", "会議室");
        learn(&mut user, "かいだん", "階段");
        // 長く一致する方
        assert_eq!(suggest("かいぎ", &user).unwrap().surface, "会議室");
        // 同じ長さなら新しく学習した方
        assert_eq!(suggest("かい", &user).unwrap().surface, "階段");
        // ユーザー辞書の語を学習した語より先に
        user.dictionary
            .add("かいしゃめい", "株式会社スズリ", UserPos::Organization)
            .unwrap();
        assert_eq!(suggest("かい", &user).unwrap().surface, "株式会社スズリ");
    }

    #[test]
    fn matches_alphabet_ignoring_case() {
        let mut user = UserData::default();
        user.dictionary
            .add("npmぱっけーじ", "npm パッケージ", UserPos::Noun)
            .unwrap();
        let s = suggest("NPMぱ", &user).unwrap();
        assert_eq!((s.matched, s.reading.as_str()), (4, "npmぱっけーじ"));
    }
}
