//! 実際の辞書で変換結果と処理時間を確かめる。
//!
//! `cargo run -p suzuri-engine --release --example bench [-- 辞書のパス]`

use std::path::PathBuf;
use std::time::Instant;

use suzuri_engine::dict::Dictionary;
use suzuri_engine::history::DEFAULT_CAPACITY;
use suzuri_engine::user_dict::MAX_READING_CHARS;
use suzuri_engine::context::{ContextHistory, ContextKind, DEFAULT_CAPACITY as CONTEXT_CAPACITY};
use suzuri_engine::learned::{DEFAULT_CAPACITY as LEARNED_CAPACITY, LearnedWords};
use suzuri_engine::{CommitSegment, Committed, Engine, UserData, UserHistory, UserPos};

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

    // 区切りの学習の効き方。手で区切った文節を確定する前と後を並べる
    let manual = |reading: &str, surface: &str| CommitSegment {
        reading: reading.into(),
        surface: surface.into(),
        was_first: false,
        manual: true,
    };
    let mut learned = UserData::default();
    engine.commit(&mut learned, &[manual("いちぶん", "一文"), manual("だいにかいぎしつ", "第2会議室")]);
    for s in [SENTENCES[2], SENTENCES[4], "いちぶんずつよむ", "いちぶんいっぷん"] {
        let first = |u: &UserData| {
            let c = engine.convert(s, u);
            c.segments.iter().map(|seg| seg.candidates[0].clone()).collect::<Vec<_>>().join("|")
        };
        println!("区切りの学習 {s}: {} → {}", first(&empty), first(&learned));
    }

    // 文脈つきの学習の効き方。「人体の」のあとで「器官」、「ロケットが」のあとで「帰還」を選んだあと
    let seg = |reading: &str, surface: &str, was_first: bool| CommitSegment {
        reading: reading.into(),
        surface: surface.into(),
        was_first,
        manual: false,
    };
    let mut context = UserData::default();
    engine.commit(&mut context, &[seg("じんたいの", "人体の", true), seg("きかん", "器官", false)]);
    engine.commit(&mut context, &[seg("ろけっとが", "ロケットが", true), seg("きかん", "帰還", false)]);
    for s in ["じんたいのきかん", "ろけっとがきかんする", "じんたいのしくみとそのきかん", "きかん"] {
        let first = |u: &UserData| {
            let c = engine.convert(s, u);
            c.segments.iter().map(|seg| seg.candidates[0].clone()).collect::<Vec<_>>().join("|")
        };
        println!("文脈つきの学習 {s}: {} → {}", first(&empty), first(&context));
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
    full.context = ContextHistory::new(CONTEXT_CAPACITY);
    for i in 0..CONTEXT_CAPACITY {
        let kind = if i % 2 == 0 { ContextKind::Adjacent } else { ContextKind::Topic };
        full.context.record(kind, &format!("語{}", i % 500), &format!("よみ{i}"), &format!("表記{i}"), false);
    }
    // 実際の文で文脈が当たるようにも入れる
    for s in SENTENCES {
        for seg in engine.convert(s, &empty).segments {
            full.context.record(ContextKind::Topic, "今日", &seg.reading, &seg.candidates[1.min(seg.candidates.len() - 1)], false);
        }
    }
    full.learned = LearnedWords::new(LEARNED_CAPACITY);
    for i in 0..LEARNED_CAPACITY {
        let reading: String = (0..MAX_READING_CHARS).map(|j| kana[(i * 11 + j * 5) % kana.len()]).collect();
        full.learned.learn(&reading, &format!("学{i}"), 1, 1, 3000);
    }
    for s in SENTENCES {
        let t = Instant::now();
        engine.convert(s, &full);
        println!(
            "学習 {} 件・区切りの学習 {} 件・文脈 {} 件・ユーザー辞書 {} 語での変換（{}文字）: {:?}",
            full.history.len(),
            full.learned.len(),
            full.context.len(),
            full.dictionary.words().len(),
            s.chars().count(),
            t.elapsed()
        );
    }
    // 続き（サジェスト）は打つたびに探すので、学習とユーザー辞書が上限まで溜まった状態で測る
    for s in SENTENCES {
        let t = Instant::now();
        suzuri_engine::suggest::suggest(s, &full);
        println!("続きの予測（{}文字）: {:?}", s.chars().count(), t.elapsed());
    }
    let t = Instant::now();
    let text = full.history.serialize();
    println!("学習の書き出し: {:?}（{} KB）", t.elapsed(), text.len() / 1024);
    let t = Instant::now();
    let text = full.learned.serialize();
    println!("区切りの学習の書き出し: {:?}（{} KB）", t.elapsed(), text.len() / 1024);
    let t = Instant::now();
    let text = full.context.serialize();
    println!("文脈つきの学習の書き出し: {:?}（{} KB）", t.elapsed(), text.len() / 1024);
}
