//! 実際の辞書で変換結果と処理時間を確かめる。
//!
//! `cargo run -p suzuri-engine --release --example bench [-- 辞書のパス]`

use std::path::PathBuf;
use std::time::Instant;

use suzuri_engine::dict::Dictionary;
use suzuri_engine::history::DEFAULT_CAPACITY;
use suzuri_engine::user_dict::MAX_READING_CHARS;
use suzuri_engine::{Committed, Engine, UserData, UserHistory, UserPos};

/// ユーザー辞書の効き方を見る文。登録した語が効くか、ほかの語の一部の読みに割り込まないか
const USER_WORDS: &[(&str, &str, UserPos)] = &[
    ("たなか", "田中", UserPos::Person),
    ("すずり", "スズリ", UserPos::Organization),
    ("かい", "KAI", UserPos::Noun),
];
const USER_SENTENCES: &[&str] = &[
    "たなかさんにあう",
    "きたなかった",
    "すずりでへんかんする",
    "すずりですみをする",
    "かいぎにでる",
    "かいをひらく",
    "にかいめ",
];

const SENTENCES: &[&str] = &[
    "わたしはきょうがっこうにいきます",
    "きょうはいいてんきですね",
    "にほんごのぶんしょうをいちぶんまるごとうってからいっきにへんかんする",
    "かれはそのはなしをきいてとてもおどろいたようすだった",
    "あしたのかいぎはごごさんじからだいにかいぎしつでおこないます",
];

fn main() {
    let path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/suzuri.dict")
    });
    let t = Instant::now();
    let engine = Engine::new(Dictionary::open(&path).expect("辞書を開けません"));
    println!("辞書の読み込み: {:?}", t.elapsed());

    let empty = UserData::default();
    for s in SENTENCES {
        let t = Instant::now();
        let c = engine.convert(s, &empty);
        let elapsed = t.elapsed();
        let view: Vec<String> = c
            .segments
            .iter()
            .map(|seg| format!("{}({})", seg.candidates[0], seg.candidates.len()))
            .collect();
        println!("{:?} {}", elapsed, view.join(" | "));
    }

    // ピンを1つ動かしたときの再変換
    let s = SENTENCES[2];
    let n = s.chars().count();
    let boundaries: Vec<usize> = (1..n).step_by(4).collect();
    let t = Instant::now();
    engine.convert_with_boundaries(s, &boundaries, &empty);
    println!("区切り指定での再変換（{}文字）: {:?}", n, t.elapsed());

    // 右ドラッグで選んだ範囲を1文節にし、その両側を区切り直したとき
    let (from, to) = (n / 3, n / 2);
    let mut merged: Vec<usize> = boundaries.iter().copied().filter(|&b| b <= from || b >= to).collect();
    merged.extend([from, to]);
    merged.sort_unstable();
    merged.dedup();
    let t = Instant::now();
    engine.resegment_ranges(s, &merged, &[0..from, to..n], &empty);
    println!("範囲を1文節にしての再変換（{}文字）: {:?}", n, t.elapsed());

    // 登録した語の効き方。登録の前と後を並べる
    let mut words = UserData::default();
    for &(reading, surface, pos) in USER_WORDS {
        words.dictionary.add(reading, surface, pos).unwrap();
    }
    for s in USER_SENTENCES {
        let first = |u: &UserData| {
            let c = engine.convert(s, u);
            c.segments.iter().map(|seg| seg.candidates[0].clone()).collect::<Vec<_>>().join("|")
        };
        println!("ユーザー辞書 {s}: {} → {}", first(&empty), first(&words));
    }

    // 学習が上限まで溜まり、ユーザー辞書にも多く登録した状態。
    // 読みを上限の長さにした語を入れ、ラティスで引く長さが一番長くなるようにする
    let mut full = UserData {
        history: UserHistory::new(DEFAULT_CAPACITY),
        ..Default::default()
    };
    for i in 0..DEFAULT_CAPACITY {
        full.history.commit(&[Committed {
            reading: format!("よみ{i}"),
            surface: format!("表記{i}"),
            was_first: false,
        }]);
    }
    let kana: Vec<char> = ('あ'..='ん').collect();
    for i in 0..1000 {
        let reading: String = (0..MAX_READING_CHARS).map(|j| kana[(i * 7 + j * 13) % kana.len()]).collect();
        full.dictionary.add(&reading, &format!("語{i}"), UserPos::Noun).unwrap();
    }
    for s in SENTENCES {
        let t = Instant::now();
        engine.convert(s, &full);
        println!(
            "学習 {} 件・ユーザー辞書 {} 語での変換（{}文字）: {:?}",
            full.history.len(),
            full.dictionary.words().len(),
            s.chars().count(),
            t.elapsed()
        );
    }
    let t = Instant::now();
    let text = full.history.serialize();
    println!("学習の書き出し: {:?}（{} KB）", t.elapsed(), text.len() / 1024);
}
