//! 打っている語の続きの予測（サジェスト）。docs/design.md の「続きの予測」。
//!
//! 打っている文節の先頭から末尾までの読みを、ユーザー辞書の語・学習した語・システム辞書の語の
//! 読みの先頭と比べ、一致した語の残りを続きとして出す（「あにめ」→「アニメーション」）。
//! 出すのは読みが長い語だけで、文節（助詞などを含むもの）は出さない。短い語は読んで選ぶより打つ方が早く、
//! 文節は「てにをは」まで当てる必要があって外れやすいため。
//! 変換中に呼ぶので、メモリ上の表と辞書だけを見る。

use crate::user_dict::fold_char;
use crate::{Engine, UserData};

/// 続きに出す語の読みの長さの下限（文字数）
pub const MIN_WORD_CHARS: usize = 7;
/// ユーザー辞書・学習した語で、何文字打ったら続きを出すか。1文字だと当たりすぎる
pub const MIN_TYPED_CHARS: usize = 2;
/// システム辞書で、何文字打ったら続きを出すか。辞書は語が多く、短いと関係のない語が出やすい
pub const MIN_TYPED_CHARS_SYSTEM: usize = 3;
/// システム辞書の続きは、少なくともこれだけの文字を補う語に限る（1文字だけ補うのは打つのと変わらない）
pub const MIN_REST_CHARS_SYSTEM: usize = 2;
/// システム辞書で、打った読みで始まる語の読みがこれより多ければ、どれか分からないので出さない（探す時間の上限にもなる）
const SYSTEM_AMBIGUOUS_KEYS: usize = 300;
/// システム辞書の続きは、コストがこれ以下のよく使う語に限る
const SYSTEM_MAX_COST: i16 = 5000;
/// 打った読みそのものが、続きの語よりこれだけ以上よく使う語なら出さない（「わたし」→「私の男」）
const SYSTEM_TYPED_WORD_MARGIN: i32 = 1500;
/// システム辞書の続きは、2番目の語とコストがこれだけ離れているときだけ出す（迷う場面では出さない）
const SYSTEM_MIN_MARGIN: i32 = 500;

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

impl Engine {
    /// 読み `kana` の続きを1つ返す。`starts` は打っている語が始まりうる位置（文字単位。ふつうは最後の文節と
    /// その1つ前の文節の先頭）で、先に挙げた位置から順に探し、見つかったところで止める。
    /// 同じ位置では、ユーザー辞書の新しい語、学習した新しい語、システム辞書のコストの低い語の順に選ぶ
    pub fn suggest(&self, kana: &str, starts: &[usize], user: &UserData) -> Option<Suggestion> {
        let chars: Vec<char> = kana.chars().collect();
        starts.iter().find_map(|&start| {
            let typed = chars.get(start..)?;
            self.suggest_from(typed, user)
        })
    }

    /// 打ちかけの読み `typed` で始まる、それより長い語
    fn suggest_from(&self, typed: &[char], user: &UserData) -> Option<Suggestion> {
        let folded: Vec<char> = typed.iter().map(|&c| fold_char(c)).collect();
        let found = |reading: &str, surface: &str| Suggestion {
            matched: typed.len(),
            reading: reading.to_owned(),
            surface: surface.to_owned(),
        };
        if typed.len() >= MIN_TYPED_CHARS {
            // ユーザー辞書は登録した順なので、後ろほど新しい
            let words = user.dictionary.words();
            if let Some(w) = words.iter().rev().find(|w| long_enough(&w.reading) && extends(&w.reading, &folded)) {
                return Some(found(&w.reading, &w.surface));
            }
            // 学習データは文節（「会議室で」）で覚えているので、システム辞書に同じ読みと表記の名詞があるものだけを使う
            if let Some((r, s, _)) = user
                .history
                .entries()
                .filter(|(r, s, _)| {
                    long_enough(r) && extends(r, &folded) && self.dictionary().lookup(r).iter().any(|e| self.dictionary().is_noun(e.rid) && e.surface.resolve(r) == *s)
                })
                .max_by_key(|(_, _, t)| *t)
            {
                return Some(found(r, s));
            }
        }
        // システム辞書の読みはひらがなだけ
        if typed.len() < MIN_TYPED_CHARS_SYSTEM || typed.iter().any(|c| !matches!(c, 'ぁ'..='ゖ' | 'ー')) {
            return None;
        }
        let prefix: String = typed.iter().collect();
        let dict = self.dictionary();
        // 出せる語の読みごとに、一番低いコストとその表記。短い語（「あにめ」に対する「アニメ」）は迷う相手に数えない。
        // 数えると、短い語の方がよく使われるので3文字では出せず、出るのが遅く感じる
        let mut per_reading: Vec<(i16, String, String)> = Vec::new();
        let mut keys = 0;
        dict.predictive_search(&prefix, SYSTEM_AMBIGUOUS_KEYS + 1, |reading, range| {
            keys += 1;
            let rest: Vec<char> = reading.chars().skip(typed.len()).collect();
            // 短い語、1文字だけ補う語、続きが小さいかなで始まる語（「ひょうじ」→「ひょうじゅん」は「じゅ」を打っている途中）は出さない
            if !long_enough(reading)
                || rest.len() < MIN_REST_CHARS_SYSTEM
                || matches!(rest[0], 'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'ゃ' | 'ゅ' | 'ょ' | 'ゎ')
            {
                return;
            }
            let mut best: Option<(i16, String)> = None;
            for i in range {
                let e = dict.entry(i);
                // 名詞だけ（語の末尾の品詞で見る）。活用した語（「サジェストすれ」）や、たまたま読みが重なる
                // 固有名詞は出さない
                if !dict.is_noun(e.rid) {
                    continue;
                }
                let surface = e.surface.resolve(reading);
                // 英字だけの略語（「いんたーちぇんじ」→「IC」）は、打っているかなから思い浮かべた語と違う
                if surface.is_ascii() {
                    continue;
                }
                if best.as_ref().is_none_or(|(c, _)| e.cost < *c) {
                    best = Some((e.cost, surface.into_owned()));
                }
            }
            if let Some((cost, surface)) = best {
                per_reading.push((cost, reading.to_owned(), surface));
            }
        });
        if keys > SYSTEM_AMBIGUOUS_KEYS {
            return None;
        }
        per_reading.sort();
        let (cost, reading, surface) = per_reading.first()?;
        if *cost > SYSTEM_MAX_COST {
            return None;
        }
        if per_reading
            .get(1)
            .is_some_and(|(second, _, _)| (*second as i32) - (*cost as i32) < SYSTEM_MIN_MARGIN)
        {
            return None;
        }
        // 打った読みそのものが、続きの語よりずっとよく使う語なら出さない（「わたし」は「私」で足りている）。
        // 少しよく使う程度（「あにめ」の「アニメ」と「アニメーション」）なら出す
        if dict
            .lookup(&prefix)
            .iter()
            .any(|e| (e.cost as i32) + SYSTEM_TYPED_WORD_MARGIN <= *cost as i32)
        {
            return None;
        }
        Some(found(reading, surface))
    }
}

/// 続きに出せる長さの語か
fn long_enough(reading: &str) -> bool {
    reading.chars().count() >= MIN_WORD_CHARS
}

/// 語の読み `reading` が、打ちかけの読み（そろえたもの）で始まり、それより長いか
fn extends(reading: &str, typed: &[char]) -> bool {
    let mut chars = reading.chars().map(fold_char);
    typed.iter().all(|&t| chars.next() == Some(t)) && chars.next().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::{Dictionary, DictionaryBuilder, RawEntry};
    use crate::{Committed, UserPos};


    /// 品詞は 0 が名詞、1 が人名などの固有名、2 が動詞の活用形
    fn engine() -> Engine {
        let mut b = DictionaryBuilder::new(3, 2);
        b.set_noun(0, true);
        for (reading, lid, cost, surface) in [
            ("あにめ", 0, 3000, "アニメ"),
            ("あにめーしょん", 0, 4000, "アニメーション"),
            ("あにめーたー", 0, 4800, "アニメーター"),
            ("あにめい", 0, 3500, "アニ明"),
            ("こんぽーねんと", 0, 4500, "コンポーネント"),
            ("こんぽーざー", 0, 6300, "コンポーザー"),
            ("こんぽん", 0, 4600, "根本"),
            // 人名・英字だけの略語・小さいかなで続く語・コストの高い語は出さない
            ("ましたこのみ", 1, 1000, "間下このみ"),
            ("さじぇすと", 0, 5900, "サジェスト"),
            ("さじぇすとすれ", 2, 4900, "サジェストすれ"),
            ("いんたーちぇんじ", 0, 1900, "IC"),
            ("いんたーねっと", 0, 3700, "インターネット"),
            ("ひょうじゅん", 0, 2800, "標準"),
            ("たいみんぐべると", 0, 6700, "タイミングベルト"),
            // 読みが6文字の語は出さない
            ("でーたべーす", 0, 4200, "データベース"),
            // コストの近い長い語が2つある
            ("あぷりけーしょん", 0, 4100, "アプリケーション"),
            ("あぷりけーたーず", 0, 4300, "アプリケーターズ"),
            // 打った読みそのものがずっとよく使う語
            ("わたし", 0, 0, "私"),
            ("わたしのおとこ", 0, 4700, "私の男"),
        ] {
            b.add_entry(RawEntry {
                reading: reading.into(),
                lid,
                rid: lid,
                cost,
                surface: surface.into(),
            });
        }
        Engine::new(Dictionary::from_bytes(b.build().unwrap()).unwrap())
    }

    fn learn(user: &mut UserData, reading: &str, surface: &str) {
        user.history.commit(&[Committed {
            reading: reading.into(),
            surface: surface.into(),
            was_first: false,
        }]);
    }

    #[test]
    fn completes_the_word_being_typed_from_the_system_dictionary() {
        let e = engine();
        let user = UserData::default();
        let s = e.suggest("きょうはあにめー", &[4], &user).unwrap();
        assert_eq!((s.matched, s.reading.as_str(), s.surface.as_str()), (4, "あにめーしょん", "アニメーション"));
        assert_eq!(e.suggest("こんぽー", &[0], &user).unwrap().surface, "コンポーネント");
        // 3文字でも出す。短い語（「アニメ」「根本」）がよく使われても、迷う相手には数えない
        assert_eq!(e.suggest("あにめ", &[0], &user).unwrap().surface, "アニメーション");
        assert_eq!(e.suggest("こんぽ", &[0], &user).unwrap().surface, "コンポーネント");
        assert_eq!(e.suggest("いんたー", &[0], &user).unwrap().surface, "インターネット");
        // 2文字では出さない。打ち終えた語も出さない
        assert_eq!(e.suggest("あに", &[0], &user), None);
        assert_eq!(e.suggest("あにめーしょん", &[0], &user), None);
    }

    #[test]
    fn suggests_only_when_likely_to_be_taken() {
        let e = engine();
        let user = UserData::default();
        let none = |kana: &str| e.suggest(kana, &[0], &user).map(|s| s.surface);
        // 打った読みそのものが、ずっとよく使う語になっている
        assert_eq!(none("わたし"), None);
        // 長い語どうしでコストが近く、どちらか分からない
        assert_eq!(none("あぷり"), None);
        // 短い語、人名、小さいかなで続く語（「じゅ」を打っている途中）、コストの高い語
        assert_eq!(none("でーた"), None);
        assert_eq!(none("ました"), None);
        // 名詞でない語（「サジェストする」の活用形）
        assert_eq!(none("さじぇす"), None);
        assert_eq!(none("ひょうじ"), None);
        assert_eq!(none("たいみん"), None);
    }

    #[test]
    fn prefers_user_words_and_tries_starts_in_order() {
        let e = engine();
        let mut user = UserData::default();
        // 学習した語は、システム辞書の条件（「あにめ」は「アニメ」で足りている）によらず出す。
        // ただし文節（「アニメーションで」）は出さず、システム辞書にある語だけを使う
        learn(&mut user, "あにめーしょん", "アニメーション");
        learn(&mut user, "あにめーしょんで", "アニメーションで");
        assert_eq!(e.suggest("あにめ", &[0], &user).unwrap().surface, "アニメーション");
        // ユーザー辞書の語を先に。短い語は出さない
        user.dictionary.add("あにめぶ", "アニメ部", UserPos::Organization).unwrap();
        assert_eq!(e.suggest("あにめ", &[0], &user).unwrap().surface, "アニメーション");
        user.dictionary.add("あにめーしょんぶ", "アニメーション部", UserPos::Organization).unwrap();
        assert_eq!(e.suggest("あにめ", &[0], &user).unwrap().surface, "アニメーション部");
        // 最後の文節（「め」）では見つからなければ、1つ前の文節の先頭から探す
        assert_eq!(e.suggest("あにめ", &[2, 0], &user).unwrap().matched, 3);
    }

    #[test]
    fn matches_user_words_ignoring_case() {
        let e = engine();
        let mut user = UserData::default();
        user.dictionary.add("npmぱっけーじ", "npm パッケージ", UserPos::Noun).unwrap();
        let s = e.suggest("NPMぱ", &[0], &user).unwrap();
        assert_eq!((s.matched, s.reading.as_str()), (4, "npmぱっけーじ"));
    }
}
