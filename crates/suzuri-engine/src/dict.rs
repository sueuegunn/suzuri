//! バイナリ辞書の形式と読み書き。
//!
//! 形式（数値はすべてリトルエンディアン）:
//!
//! ```text
//! ヘッダー  magic(8) pos_count(u32) unknown_id(u32) 各セクションの (offset u32, len u32) × 5
//! 接続コスト  i16 × pos_count²       （rid * pos_count + lid の順）
//! 文節境界    1bit × pos_count²      （同上。1 なら rid と lid の間で文節を区切る）
//! 読み索引    fst::Map               （読み → 開始位置 << 16 | 件数）
//! 単語        10バイト × 単語数      （lid u16, rid u16, cost i16, 表記 u32）
//! 表記        長さ u16 + UTF-8       （読みと同じ・カタカナ以外の表記だけを重複なく置く）
//! ```

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use crate::kana::hiragana_to_katakana;

const MAGIC: &[u8; 8] = b"SZRDIC01";
const SECTION_COUNT: usize = 5;
const HEADER_SIZE: usize = 8 + 4 + 4 + SECTION_COUNT * 8;
const ENTRY_SIZE: usize = 10;
/// 表記が読みと同じ（ひらがなのまま）
const SURFACE_READING: u32 = u32::MAX;
/// 表記が読みのカタカナ
const SURFACE_KATAKANA: u32 = u32::MAX - 1;

#[derive(Debug)]
pub enum DictError {
    Io(std::io::Error),
    Invalid(String),
}

impl fmt::Display for DictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DictError::Io(e) => write!(f, "辞書を読めません: {e}"),
            DictError::Invalid(msg) => write!(f, "辞書が壊れています: {msg}"),
        }
    }
}

impl std::error::Error for DictError {}

impl From<std::io::Error> for DictError {
    fn from(e: std::io::Error) -> Self {
        DictError::Io(e)
    }
}

fn invalid(msg: impl Into<String>) -> DictError {
    DictError::Invalid(msg.into())
}

/// 辞書に入れる単語1件
#[derive(Debug, Clone)]
pub struct RawEntry {
    pub reading: String,
    pub lid: u16,
    pub rid: u16,
    pub cost: i16,
    pub surface: String,
}

/// テキストから集めたデータをバイナリ辞書にまとめる
pub struct DictionaryBuilder {
    pos_count: usize,
    unknown_id: u16,
    connection: Vec<i16>,
    boundary: Vec<u8>,
    entries: Vec<RawEntry>,
}

impl DictionaryBuilder {
    /// 接続コストはすべて 0、文節境界はすべて「区切る」で初期化する
    pub fn new(pos_count: u16, unknown_id: u16) -> Self {
        let n = pos_count as usize;
        DictionaryBuilder {
            pos_count: n,
            unknown_id,
            connection: vec![0; n * n],
            boundary: vec![0xff; (n * n).div_ceil(8)],
            entries: Vec::new(),
        }
    }

    pub fn set_connection(&mut self, rid: u16, lid: u16, cost: i16) {
        let i = rid as usize * self.pos_count + lid as usize;
        self.connection[i] = cost;
    }

    pub fn set_boundary(&mut self, rid: u16, lid: u16, is_boundary: bool) {
        let i = rid as usize * self.pos_count + lid as usize;
        if is_boundary {
            self.boundary[i / 8] |= 1 << (i % 8);
        } else {
            self.boundary[i / 8] &= !(1 << (i % 8));
        }
    }

    pub fn add_entry(&mut self, entry: RawEntry) {
        self.entries.push(entry);
    }

    pub fn build(mut self) -> Result<Vec<u8>, DictError> {
        let n = self.pos_count;
        if self.unknown_id as usize >= n {
            return Err(invalid("未知語の品詞IDが範囲外です"));
        }
        for e in &self.entries {
            if e.lid as usize >= n || e.rid as usize >= n {
                return Err(invalid(format!("品詞IDが範囲外です: {}", e.reading)));
            }
            if e.reading.is_empty() {
                return Err(invalid("読みが空の単語があります"));
            }
        }
        self.entries.sort_by(|a, b| {
            a.reading
                .as_bytes()
                .cmp(b.reading.as_bytes())
                .then(a.cost.cmp(&b.cost))
        });

        let mut index = fst::MapBuilder::memory();
        let mut entry_bytes = Vec::with_capacity(self.entries.len() * ENTRY_SIZE);
        let mut pool = Vec::new();
        let mut pool_index: HashMap<&str, u32> = HashMap::new();
        let mut start = 0;
        while start < self.entries.len() {
            let reading = &self.entries[start].reading;
            let mut end = start;
            while end < self.entries.len() && self.entries[end].reading == *reading {
                end += 1;
            }
            let count = end - start;
            if count > u16::MAX as usize {
                return Err(invalid(format!("同じ読みの単語が多すぎます: {reading}")));
            }
            index
                .insert(reading, ((start as u64) << 16) | count as u64)
                .map_err(|e| invalid(e.to_string()))?;
            for e in &self.entries[start..end] {
                let surface = if e.surface == e.reading {
                    SURFACE_READING
                } else if e.surface == hiragana_to_katakana(&e.reading) {
                    SURFACE_KATAKANA
                } else if let Some(&offset) = pool_index.get(e.surface.as_str()) {
                    offset
                } else {
                    let len = u16::try_from(e.surface.len())
                        .map_err(|_| invalid(format!("表記が長すぎます: {}", e.surface)))?;
                    let offset = u32::try_from(pool.len())
                        .ok()
                        .filter(|&o| o < SURFACE_KATAKANA)
                        .ok_or_else(|| invalid("表記の領域が大きすぎます"))?;
                    pool.extend_from_slice(&len.to_le_bytes());
                    pool.extend_from_slice(e.surface.as_bytes());
                    pool_index.insert(&e.surface, offset);
                    offset
                };
                entry_bytes.extend_from_slice(&e.lid.to_le_bytes());
                entry_bytes.extend_from_slice(&e.rid.to_le_bytes());
                entry_bytes.extend_from_slice(&e.cost.to_le_bytes());
                entry_bytes.extend_from_slice(&surface.to_le_bytes());
            }
            start = end;
        }
        let index = index.into_inner().map_err(|e| invalid(e.to_string()))?;

        let connection: Vec<u8> = self
            .connection
            .iter()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        let sections: [&[u8]; SECTION_COUNT] =
            [&connection, &self.boundary, &index, &entry_bytes, &pool];

        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(n as u32).to_le_bytes());
        out.extend_from_slice(&(self.unknown_id as u32).to_le_bytes());
        let mut offset = HEADER_SIZE;
        for s in sections {
            let o = u32::try_from(offset).map_err(|_| invalid("辞書が大きすぎます"))?;
            out.extend_from_slice(&o.to_le_bytes());
            out.extend_from_slice(&(s.len() as u32).to_le_bytes());
            offset += s.len();
        }
        for s in sections {
            out.extend_from_slice(s);
        }
        Ok(out)
    }
}

enum Storage {
    Owned(Vec<u8>),
    Mapped(memmap2::Mmap),
}

impl AsRef<[u8]> for Storage {
    fn as_ref(&self) -> &[u8] {
        match self {
            Storage::Owned(v) => v,
            Storage::Mapped(m) => m,
        }
    }
}

/// fst に渡すための、辞書データの一部分
#[derive(Clone)]
struct Slice {
    storage: Arc<Storage>,
    range: Range<usize>,
}

impl AsRef<[u8]> for Slice {
    fn as_ref(&self) -> &[u8] {
        &self.storage.as_ref().as_ref()[self.range.clone()]
    }
}

/// 単語の表記
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface<'a> {
    /// 読みと同じ
    Reading,
    /// 読みのカタカナ
    Katakana,
    Text(&'a str),
}

impl<'a> Surface<'a> {
    pub fn resolve(self, reading: &'a str) -> Cow<'a, str> {
        match self {
            Surface::Reading => Cow::Borrowed(reading),
            Surface::Katakana => Cow::Owned(hiragana_to_katakana(reading)),
            Surface::Text(s) => Cow::Borrowed(s),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub lid: u16,
    pub rid: u16,
    pub cost: i16,
    pub surface: Surface<'a>,
}

/// 読み込み済みのバイナリ辞書
pub struct Dictionary {
    storage: Arc<Storage>,
    pos_count: usize,
    unknown_id: u16,
    connection: Range<usize>,
    boundary: Range<usize>,
    index: fst::raw::Fst<Slice>,
    entries: Range<usize>,
    pool: Range<usize>,
}

impl Dictionary {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, DictError> {
        Self::parse(Arc::new(Storage::Owned(bytes)))
    }

    /// ファイルをメモリマップで開く
    pub fn open(path: &Path) -> Result<Self, DictError> {
        let file = File::open(path)?;
        // SAFETY: 辞書ファイルはアプリの同梱リソースで、開いている間に書き換えられない前提
        let map = unsafe { memmap2::Mmap::map(&file)? };
        Self::parse(Arc::new(Storage::Mapped(map)))
    }

    fn parse(storage: Arc<Storage>) -> Result<Self, DictError> {
        let data = storage.as_ref().as_ref();
        if data.len() < HEADER_SIZE || &data[..8] != MAGIC {
            return Err(invalid("形式が違います"));
        }
        let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap()) as usize;
        let pos_count = u32_at(8);
        let unknown_id = u32_at(12);
        if pos_count == 0 || pos_count > u16::MAX as usize || unknown_id >= pos_count {
            return Err(invalid("品詞数が不正です"));
        }
        let mut sections = Vec::with_capacity(SECTION_COUNT);
        for i in 0..SECTION_COUNT {
            let offset = u32_at(16 + i * 8);
            let len = u32_at(20 + i * 8);
            if offset + len > data.len() {
                return Err(invalid("セクションがファイルの外にあります"));
            }
            sections.push(offset..offset + len);
        }
        let [connection, boundary, index, entries, pool]: [Range<usize>; SECTION_COUNT] =
            sections.try_into().unwrap();
        let cells = pos_count * pos_count;
        if connection.len() != cells * 2 || boundary.len() != cells.div_ceil(8) {
            return Err(invalid("接続表の大きさが合いません"));
        }
        if entries.len() % ENTRY_SIZE != 0 {
            return Err(invalid("単語の領域の大きさが合いません"));
        }
        let index = fst::raw::Fst::new(Slice {
            storage: storage.clone(),
            range: index,
        })
        .map_err(|e| invalid(e.to_string()))?;

        let dict = Dictionary {
            storage,
            pos_count,
            unknown_id: unknown_id as u16,
            connection,
            boundary,
            index,
            entries,
            pool,
        };
        dict.validate_entries()?;
        Ok(dict)
    }

    /// 単語の品詞IDと表記の参照先を確かめる。読み込み後は範囲外アクセスが起きないようにする
    fn validate_entries(&self) -> Result<(), DictError> {
        let pool = &self.bytes()[self.pool.clone()];
        for i in 0..self.entry_count() {
            let r = self.raw_entry(i);
            let lid = u16::from_le_bytes([r[0], r[1]]) as usize;
            let rid = u16::from_le_bytes([r[2], r[3]]) as usize;
            if lid >= self.pos_count || rid >= self.pos_count {
                return Err(invalid("単語の品詞IDが範囲外です"));
            }
            let surface = u32::from_le_bytes([r[6], r[7], r[8], r[9]]);
            if surface < SURFACE_KATAKANA {
                let o = surface as usize;
                let ok = pool.len() >= o + 2 && {
                    let len = u16::from_le_bytes([pool[o], pool[o + 1]]) as usize;
                    pool.len() >= o + 2 + len && std::str::from_utf8(&pool[o + 2..o + 2 + len]).is_ok()
                };
                if !ok {
                    return Err(invalid("表記の参照が不正です"));
                }
            }
        }
        Ok(())
    }

    fn bytes(&self) -> &[u8] {
        self.storage.as_ref().as_ref()
    }

    fn entry_count(&self) -> usize {
        self.entries.len() / ENTRY_SIZE
    }

    fn raw_entry(&self, i: usize) -> &[u8] {
        let o = self.entries.start + i * ENTRY_SIZE;
        &self.bytes()[o..o + ENTRY_SIZE]
    }

    pub fn pos_count(&self) -> usize {
        self.pos_count
    }

    /// 辞書にない文字に使う品詞ID
    pub fn unknown_id(&self) -> u16 {
        self.unknown_id
    }

    /// 左の単語の右文脈IDと右の単語の左文脈IDから、つながりのコストを返す
    pub fn connection_cost(&self, rid: u16, lid: u16) -> i32 {
        let o = self.connection.start + (rid as usize * self.pos_count + lid as usize) * 2;
        let b = self.bytes();
        i16::from_le_bytes([b[o], b[o + 1]]) as i32
    }

    /// 左の単語と右の単語の間で文節を区切るか
    pub fn is_boundary(&self, rid: u16, lid: u16) -> bool {
        let i = rid as usize * self.pos_count + lid as usize;
        self.bytes()[self.boundary.start + i / 8] & (1 << (i % 8)) != 0
    }

    /// `key` の先頭から辞書を引き、見つかった読みごとに (読みのバイト長, 単語の番号の範囲) を渡す
    pub fn common_prefix_search(&self, key: &str, mut f: impl FnMut(usize, Range<u32>)) {
        let fst = &self.index;
        let mut node = fst.root();
        let mut out = fst::raw::Output::zero();
        for (i, &b) in key.as_bytes().iter().enumerate() {
            let Some(t) = node.find_input(b) else { break };
            let t = node.transition(t);
            out = out.cat(t.out);
            node = fst.node(t.addr);
            if node.is_final() {
                let v = out.cat(node.final_output()).value();
                let start = (v >> 16) as u32;
                f(i + 1, start..start + (v & 0xffff) as u32);
            }
        }
    }

    /// 読みが完全に一致する単語を返す
    pub fn lookup(&self, reading: &str) -> Vec<Entry<'_>> {
        let mut found = Vec::new();
        self.common_prefix_search(reading, |len, range| {
            if len == reading.len() {
                found.extend(range.map(|i| self.entry(i)));
            }
        });
        found
    }

    pub fn entry(&self, index: u32) -> Entry<'_> {
        let r = self.raw_entry(index as usize);
        let surface = match u32::from_le_bytes([r[6], r[7], r[8], r[9]]) {
            SURFACE_READING => Surface::Reading,
            SURFACE_KATAKANA => Surface::Katakana,
            o => {
                let pool = &self.bytes()[self.pool.clone()];
                let o = o as usize;
                let len = u16::from_le_bytes([pool[o], pool[o + 1]]) as usize;
                // validate_entries で UTF-8 であることを確認済み
                Surface::Text(std::str::from_utf8(&pool[o + 2..o + 2 + len]).unwrap())
            }
        };
        Entry {
            lid: u16::from_le_bytes([r[0], r[1]]),
            rid: u16::from_le_bytes([r[2], r[3]]),
            cost: i16::from_le_bytes([r[4], r[5]]),
            surface,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(reading: &str, id: u16, cost: i16, surface: &str) -> RawEntry {
        RawEntry {
            reading: reading.into(),
            lid: id,
            rid: id,
            cost,
            surface: surface.into(),
        }
    }

    fn sample() -> Dictionary {
        let mut b = DictionaryBuilder::new(3, 1);
        b.set_connection(1, 2, 123);
        b.set_boundary(1, 2, false);
        b.add_entry(entry("はし", 1, 200, "箸"));
        b.add_entry(entry("はし", 1, 100, "橋"));
        b.add_entry(entry("は", 2, 50, "は"));
        b.add_entry(entry("ぱん", 1, 300, "パン"));
        Dictionary::from_bytes(b.build().unwrap()).unwrap()
    }

    #[test]
    fn looks_up_sorted_by_cost() {
        let d = sample();
        let found: Vec<_> = d
            .lookup("はし")
            .iter()
            .map(|e| e.surface.resolve("はし").into_owned())
            .collect();
        assert_eq!(found, ["橋", "箸"]);
        assert_eq!(d.lookup("ぱん")[0].surface, Surface::Katakana);
        assert_eq!(d.lookup("は")[0].surface, Surface::Reading);
        assert!(d.lookup("ぱ").is_empty());
    }

    #[test]
    fn finds_all_prefixes() {
        let d = sample();
        let mut lens = Vec::new();
        d.common_prefix_search("はしる", |len, _| lens.push(len));
        assert_eq!(lens, ["は".len(), "はし".len()]);
    }

    #[test]
    fn stores_connection_and_boundary() {
        let d = sample();
        assert_eq!(d.connection_cost(1, 2), 123);
        assert_eq!(d.connection_cost(2, 1), 0);
        assert!(!d.is_boundary(1, 2));
        assert!(d.is_boundary(2, 1));
    }

    #[test]
    fn rejects_broken_data() {
        let mut bytes = DictionaryBuilder::new(3, 1).build().unwrap();
        bytes.truncate(bytes.len() - 1);
        assert!(Dictionary::from_bytes(bytes).is_err());
        assert!(Dictionary::from_bytes(b"not a dictionary".to_vec()).is_err());
    }
}
