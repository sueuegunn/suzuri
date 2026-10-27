//! 実際の辞書で変換結果と処理時間を確かめる。
//!
//! `cargo run -p suzuri-engine --release --example bench [-- 辞書のパス]`

use std::path::PathBuf;
use std::time::Instant;

use suzuri_engine::dict::Dictionary;
use suzuri_engine::history::DEFAULT_CAPACITY;
use suzuri_engine::{Committed, Engine, UserHistory};

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

    let empty = UserHistory::default();
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

    // 学習が上限まで溜まった状態
    let mut full = UserHistory::new(DEFAULT_CAPACITY);
    for i in 0..DEFAULT_CAPACITY {
        full.commit(&[Committed {
            reading: format!("よみ{i}"),
            surface: format!("表記{i}"),
            was_first: false,
        }]);
    }
    let t = Instant::now();
    engine.convert(s, &full);
    println!("学習 {} 件での変換: {:?}", full.len(), t.elapsed());
    let t = Instant::now();
    let text = full.serialize();
    println!("学習の書き出し: {:?}（{} KB）", t.elapsed(), text.len() / 1024);
}
