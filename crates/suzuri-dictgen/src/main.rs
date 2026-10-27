//! Mozc の OSS 辞書を取得し、スズリのバイナリ辞書に変換する。
//!
//! 使い方: `cargo run -p suzuri-dictgen --release [-- 出力先ディレクトリ]`
//!
//! 取得するのは固定したコミットのファイルだけで、ハッシュ値を照合してから使う。
//! 取得したファイルは `.cache/mozc/<コミット>/` に置き、2回目以降は通信しない。

use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use regex::Regex;
use sha2::{Digest, Sha256};
use suzuri_engine::dict::{Dictionary, DictionaryBuilder, RawEntry};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const MOZC_COMMIT: &str = "d766d25d253141eff71e25c593aaf753f8d91cee";

/// (Mozc リポジトリ内のパス, SHA-256)
const FILES: &[(&str, &str)] = &[
    ("LICENSE", "44cdd923b91ea9199293abecc2762c70c87dbf1e581c027a94c416368d1a648c"),
    ("src/data/dictionary_oss/README.txt", "6b1de66bc6fa30e0b45dc45b8c8f6c57bd78aff923c261a3efffac8eb86f7bac"),
    ("src/data/dictionary_oss/id.def", "07a05a268c3783b7e02b36e4aa591555a2dd19a1f3c93891576740761c154f2d"),
    ("src/data/rules/segmenter.def", "faf14ec987a12e36c796b35b4f84e210c6a780f5ae52bddc7f80accc58861b21"),
    ("src/data/dictionary_oss/connection_single_column.txt", "68823a1f6617372ccb89afdbcb334d7a0bcfa82c9c5dda03fc1f80f1cab5c48e"),
    ("src/data/dictionary_oss/dictionary00.txt", "9e07ce292932c4fff327ed8cb06274c09acca556bd8bdf7c7ef1f6173eae3b61"),
    ("src/data/dictionary_oss/dictionary01.txt", "836fd5fd8399510de0d8ef7b33be913cca6bbaee7b712cc8045fba03f4c75a89"),
    ("src/data/dictionary_oss/dictionary02.txt", "93943858d0cfd092b849a78def6901eaf228177e1ac5663544e865a3a1e45a9e"),
    ("src/data/dictionary_oss/dictionary03.txt", "cb9f1b0b4a7550d682d86244a021cd17e0b15366328819e69135b5f183a1a24b"),
    ("src/data/dictionary_oss/dictionary04.txt", "9fdc4e3948d03463de13ab5782adffbc11dd06e934cb80138c5920029cd3d2f6"),
    ("src/data/dictionary_oss/dictionary05.txt", "199e3b0f050b691823ae7ba943090d437f51af6436b27c5f3dc69cafa12000e4"),
    ("src/data/dictionary_oss/dictionary06.txt", "b95b14f7770ae2a824bc8e9fad8b03c1bc45c504f93a36554b8b56daa0368d2a"),
    ("src/data/dictionary_oss/dictionary07.txt", "7f53649b529d4c8c495d3a8dd9c4d489dfe7696080f7423107ff01331174be48"),
    ("src/data/dictionary_oss/dictionary08.txt", "50a31bb5fc1646721b00be371a5695dffab1292bda143e1e59cc70cbbdd8643f"),
    ("src/data/dictionary_oss/dictionary09.txt", "4e5fcd4cbfbf11395166a37b506f547fc54d20edc0884f5184ecd3fc705f0d9a"),
];

/// 辞書にない文字に割り当てる品詞
const UNKNOWN_POS: &str = "名詞,一般,*,*,*,*,*";

/// Mozc の segmenter.def より優先する規則（書式は segmenter.def と同じ）。
/// Mozc は数字と助数詞を別の文節にして後から書き換えるが、スズリは書き換えを持たないので、
/// 「3時」「第2」「15」のような数のまとまりを1文節にしておく
const SEGMENTER_OVERRIDES: &str = "名詞,数 名詞,数 false
名詞,数 名詞,接尾 false
接頭詞,数接続 名詞,数 false
";

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("src-tauri/resources"));
    let cache = root.join(".cache/mozc").join(MOZC_COMMIT);

    for (path, hash) in FILES {
        fetch(&cache, path, hash)?;
    }
    let read = |path: &str| fs::read_to_string(cache.join(path));

    let pos = parse_id_def(&read("src/data/dictionary_oss/id.def")?)?;
    let pos_count = u16::try_from(pos.len())?;
    let unknown_id = pos
        .iter()
        .position(|p| p == UNKNOWN_POS)
        .ok_or("未知語の品詞が id.def にありません")? as u16;
    let mut builder = DictionaryBuilder::new(pos_count, unknown_id);

    apply_connection(&mut builder, &read("src/data/dictionary_oss/connection_single_column.txt")?, pos.len())?;
    let segmenter = format!("{SEGMENTER_OVERRIDES}{}", read("src/data/rules/segmenter.def")?);
    apply_segmenter(&mut builder, &segmenter, &pos)?;
    let mut count = 0;
    for i in 0..10 {
        let text = read(&format!("src/data/dictionary_oss/dictionary{i:02}.txt"))?;
        for entry in parse_dictionary(&text)? {
            builder.add_entry(entry);
            count += 1;
        }
    }

    for entry in number_entries(&pos)? {
        builder.add_entry(entry);
        count += 1;
    }

    let bytes = builder.build()?;
    // 書き出す前に、読み込めることを確かめる
    Dictionary::from_bytes(bytes.clone())?;
    fs::create_dir_all(&out_dir)?;
    fs::write(out_dir.join("suzuri.dict"), &bytes)?;
    fs::write(
        out_dir.join("THIRD_PARTY_LICENSES.txt"),
        format!(
            "スズリの辞書は Mozc（https://github.com/google/mozc, コミット {MOZC_COMMIT}）の OSS 辞書から生成しています。\n\n\
             ===== Mozc LICENSE =====\n\n{}\n\n===== Mozc dictionary_oss/README.txt =====\n\n{}",
            read("LICENSE")?,
            read("src/data/dictionary_oss/README.txt")?
        ),
    )?;
    println!(
        "{} 語、品詞 {} 個、{:.1} MB を {} に書き出しました",
        count,
        pos_count,
        bytes.len() as f64 / 1_048_576.0,
        out_dir.display()
    );
    Ok(())
}

/// キャッシュになければ取得し、ハッシュ値を照合する
fn fetch(cache: &Path, path: &str, hash: &str) -> Result<()> {
    let dest = cache.join(path);
    if dest.exists() && sha256(&fs::read(&dest)?) == hash {
        return Ok(());
    }
    let url = format!("https://raw.githubusercontent.com/google/mozc/{MOZC_COMMIT}/{path}");
    println!("取得中: {url}");
    let mut body = Vec::new();
    io::copy(&mut ureq::get(&url).call()?.into_body().into_reader(), &mut body)?;
    let actual = sha256(&body);
    if actual != hash {
        return Err(format!("{path} のハッシュ値が一致しません（期待 {hash}、実際 {actual}）").into());
    }
    fs::create_dir_all(dest.parent().unwrap())?;
    fs::write(&dest, body)?;
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// id.def の「ID 品詞」の行から、ID 順の品詞の一覧を作る
fn parse_id_def(text: &str) -> Result<Vec<String>> {
    let mut pos = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let (id, name) = line.split_once(' ').ok_or_else(|| format!("id.def {}行目の形式が違います", i + 1))?;
        if id.parse::<usize>()? != i {
            return Err(format!("id.def の ID が連番ではありません: {line}").into());
        }
        pos.push(name.to_owned());
    }
    Ok(pos)
}

/// 1行目が品詞数、以降が rid * 品詞数 + lid の順のコスト
fn apply_connection(builder: &mut DictionaryBuilder, text: &str, pos_count: usize) -> Result<()> {
    let mut lines = text.lines();
    let size: usize = lines.next().ok_or("接続表が空です")?.trim().parse()?;
    if size != pos_count {
        return Err(format!("接続表の品詞数 {size} が id.def の {pos_count} と違います").into());
    }
    let mut n = 0;
    for (i, line) in lines.enumerate() {
        let cost: i16 = line.trim().parse()?;
        builder.set_connection((i / size) as u16, (i % size) as u16, cost);
        n += 1;
    }
    if n != size * size {
        return Err("接続表の行数が合いません".into());
    }
    Ok(())
}

/// segmenter.def の「左の品詞 右の品詞 true/false」を、先に書かれた規則が優先になるよう焼き込む。
/// 品詞の指定は Mozc の gen_segmenter_code.py と同じく、`*` を任意の1項目とする先頭一致の正規表現
fn apply_segmenter(builder: &mut DictionaryBuilder, text: &str, pos: &[String]) -> Result<()> {
    let ids = |pattern: &str| -> Result<Vec<u16>> {
        if pattern == "*" {
            return Ok((0..pos.len() as u16).collect());
        }
        let re = Regex::new(&format!("^(?:{})", pattern.replace('*', "[^,]+")))?;
        let ids: Vec<u16> = (0..pos.len()).filter(|&i| re.is_match(&pos[i])).map(|i| i as u16).collect();
        if ids.is_empty() {
            return Err(format!("segmenter.def の {pattern} に一致する品詞がありません").into());
        }
        Ok(ids)
    };
    let mut rules = Vec::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        let [left, right, result] = cols[..] else {
            return Err(format!("segmenter.def の形式が違います: {line}").into());
        };
        rules.push((ids(left)?, ids(right)?, result.eq_ignore_ascii_case("true")));
    }
    // 該当する規則がなければ区切る（Builder の初期値）。後ろの規則から順に上書きする
    for (left, right, result) in rules.iter().rev() {
        for &rid in left {
            for &lid in right {
                builder.set_boundary(rid, lid, *result);
            }
        }
    }
    // 文頭・文末は常に区切る
    for id in 0..pos.len() as u16 {
        builder.set_boundary(0, id, true);
        builder.set_boundary(id, 0, true);
    }
    Ok(())
}

/// 半角数字1文字のコスト。Mozc の「いち → 1」（753）と同じくらいにする
const DIGIT_COST: i16 = 700;

/// 半角数字 0〜9 の単語。ローマ字入力ではそのまま入るが、Mozc の辞書の読みはかなだけなので、数として足す。
/// エンジンはこの品詞を、数字の並び（「1000」「3.14」）をまとめた語にも使う
fn number_entries(pos: &[String]) -> Result<Vec<RawEntry>> {
    let name = "名詞,数,アラビア数字,*,*,*,*";
    let id = pos.iter().position(|p| p == name).ok_or(format!("{name} が id.def にありません"))? as u16;
    Ok(('0'..='9')
        .map(|c| RawEntry {
            reading: c.to_string(),
            lid: id,
            rid: id,
            cost: DIGIT_COST,
            surface: c.to_string(),
        })
        .collect())
}

/// 「読み TAB 左文脈ID TAB 右文脈ID TAB コスト TAB 表記」。6列目以降は使わない
fn parse_dictionary(text: &str) -> Result<Vec<RawEntry>> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            let [reading, lid, rid, cost, surface, ..] = cols[..] else {
                return Err(format!("辞書の形式が違います: {line}").into());
            };
            Ok(RawEntry {
                reading: reading.to_owned(),
                lid: lid.parse()?,
                rid: rid.parse()?,
                cost: cost.parse()?,
                surface: surface.to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earlier_segmenter_rule_wins() {
        let pos: Vec<String> = [
            "BOS/EOS,*,*,*,*,*,*",
            "名詞,数,アラビア数字,*,*,*,*",
            "名詞,数,漢数字,*,*,*,*",
            "名詞,接尾,助数詞,*,*,*,時",
            "名詞,一般,*,*,*,*,*",
            "接頭詞,数接続,*,*,*,*,*",
        ]
        .map(String::from)
        .to_vec();
        let mozc = "名詞,数,漢数字 名詞,数,アラビア数字 true\n名詞,数 名詞,接尾,助数詞 true\n";
        let mut b = DictionaryBuilder::new(pos.len() as u16, 4);
        apply_segmenter(&mut b, &format!("{SEGMENTER_OVERRIDES}{mozc}"), &pos).unwrap();
        let d = Dictionary::from_bytes(b.build().unwrap()).unwrap();
        // 数のまとまりは区切らない
        assert!(!d.is_boundary(2, 1));
        assert!(!d.is_boundary(1, 3));
        // 規則のない組み合わせと文頭・文末は区切る
        assert!(d.is_boundary(3, 4));
        assert!(d.is_boundary(0, 1));
        assert!(d.is_boundary(1, 0));
    }
}
