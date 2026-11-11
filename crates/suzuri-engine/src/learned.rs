//! 文節区切りの学習。手で区切った文節を「学習した語」として覚え、次からラティスに足す（docs/design.md）。
//!
//! 自動で増えるユーザー辞書として扱う。ユーザー辞書と違い、品詞とコストは覚えたときに
//! エンジンが求めたものを持ち、件数に上限を設けて古く使ったものから捨てる。

use std::collections::HashMap;

use crate::history::HistoryError;

pub const DEFAULT_CAPACITY: usize = 10_000;
const HEADER: &str = "suzuri-learned-words 1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearnedWord {
    pub reading: String,
    pub surface: String,
    /// 先頭の単語の左文脈ID
    pub lid: u16,
    /// 末尾の単語の右文脈ID
    pub rid: u16,
    pub cost: i32,
    clock: u64,
}

pub struct LearnedWords {
    words: Vec<LearnedWord>,
    /// 読み → words の番号
    by_reading: HashMap<String, Vec<u32>>,
    max_chars: usize,
    clock: u64,
    capacity: usize,
}

impl Default for LearnedWords {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl LearnedWords {
    pub fn new(capacity: usize) -> Self {
        LearnedWords {
            words: Vec::new(),
            by_reading: HashMap::new(),
            max_chars: 0,
            clock: 0,
            capacity: capacity.max(1),
        }
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    pub fn word(&self, index: u32) -> &LearnedWord {
        &self.words[index as usize]
    }

    /// 覚えた読みのうち一番長いものの文字数。ラティスでこれより長くは引かない
    pub fn max_chars(&self) -> usize {
        self.max_chars
    }

    /// 読みが完全に一致する語の番号
    pub fn lookup(&self, reading: &str) -> &[u32] {
        self.by_reading.get(reading).map_or(&[], Vec::as_slice)
    }

    /// 語を覚える。同じ読みと表記を覚えていれば、品詞とコストを置き換えて時刻を更新する
    pub fn learn(&mut self, reading: &str, surface: &str, lid: u16, rid: u16, cost: i32) {
        self.clock += 1;
        let clock = self.clock;
        let found = self
            .lookup(reading)
            .iter()
            .copied()
            .find(|&i| self.words[i as usize].surface == surface);
        if let Some(i) = found {
            let w = &mut self.words[i as usize];
            (w.lid, w.rid, w.cost, w.clock) = (lid, rid, cost, clock);
            return;
        }
        let index = self.words.len() as u32;
        self.words.push(LearnedWord {
            reading: reading.to_owned(),
            surface: surface.to_owned(),
            lid,
            rid,
            cost,
            clock,
        });
        self.by_reading.entry(reading.to_owned()).or_default().push(index);
        self.max_chars = self.max_chars.max(reading.chars().count());
        self.evict_if_needed();
    }

    /// 上限を1割超えたら、新しく使った順に上限まで残す。1件ごとに捨てると毎回全件を見ることになるため、まとめて行う
    fn evict_if_needed(&mut self) {
        if self.words.len() <= self.capacity + (self.capacity / 10).max(1) {
            return;
        }
        self.words.sort_unstable_by_key(|w| std::cmp::Reverse(w.clock));
        self.words.truncate(self.capacity);
        self.reindex();
    }

    fn reindex(&mut self) {
        self.by_reading.clear();
        for (i, w) in self.words.iter().enumerate() {
            self.by_reading.entry(w.reading.clone()).or_default().push(i as u32);
        }
        self.max_chars = self.words.iter().map(|w| w.reading.chars().count()).max().unwrap_or(0);
    }

    /// 保存用のテキストにする。1行に「時刻 TAB 読み TAB 表記 TAB 左文脈ID TAB 右文脈ID TAB コスト」
    pub fn serialize(&self) -> String {
        let mut words: Vec<&LearnedWord> = self.words.iter().collect();
        words.sort_unstable_by_key(|w| w.clock);
        let mut out = String::from(HEADER);
        out.push('\n');
        for w in words {
            out.push_str(&format!("{}\t{}\t{}\t{}\t{}\t{}\n", w.clock, w.reading, w.surface, w.lid, w.rid, w.cost));
        }
        out
    }

    pub fn deserialize(text: &str, capacity: usize) -> Result<Self, HistoryError> {
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return Err(HistoryError::new("区切りの学習の見出し行が違います"));
        }
        let mut rows = Vec::new();
        for (i, line) in lines.enumerate() {
            let broken = || HistoryError::new(format!("区切りの学習の{}行目の形式が違います", i + 2));
            let cols: Vec<&str> = line.split('\t').collect();
            let [t, r, s, lid, rid, cost] = cols[..] else {
                return Err(broken());
            };
            if r.is_empty() || s.is_empty() {
                return Err(broken());
            }
            let t: u64 = t.parse().map_err(|_| broken())?;
            let lid: u16 = lid.parse().map_err(|_| broken())?;
            let rid: u16 = rid.parse().map_err(|_| broken())?;
            let cost: i32 = cost.parse().map_err(|_| broken())?;
            rows.push((t, r, s, lid, rid, cost));
        }
        // 古い順に入れ直せば、時刻の順序と上限の処理がそのまま保たれる
        rows.sort_unstable_by_key(|row| row.0);
        let mut words = LearnedWords::new(capacity);
        for (_, r, s, lid, rid, cost) in rows {
            words.learn(r, s, lid, rid, cost);
        }
        Ok(words)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_and_updates() {
        let mut w = LearnedWords::default();
        w.learn("やまだたろう", "山田太郎", 1, 1, 5000);
        w.learn("やまだたろう", "山田太郎", 1, 2, 4000);
        assert_eq!(w.len(), 1);
        let i = w.lookup("やまだたろう")[0];
        assert_eq!((w.word(i).rid, w.word(i).cost), (2, 4000));
        assert_eq!(w.max_chars(), 6);
    }

    #[test]
    fn evicts_oldest_beyond_capacity() {
        let mut w = LearnedWords::new(10);
        for i in 0..100 {
            w.learn(&format!("よみ{i}"), "表記", 1, 1, 0);
            assert!(w.len() <= 11);
        }
        assert_eq!(w.lookup("よみ99").len(), 1);
        assert!(w.lookup("よみ0").is_empty());
    }

    #[test]
    fn roundtrips_through_text() {
        let mut w = LearnedWords::default();
        w.learn("かいぎで", "会議で", 3, 4, 1234);
        w.learn("やまだたろう", "山田太郎", 1, 1, -50);
        let restored = LearnedWords::deserialize(&w.serialize(), DEFAULT_CAPACITY).unwrap();
        assert_eq!(restored.len(), 2);
        let i = restored.lookup("かいぎで")[0];
        assert_eq!((restored.word(i).lid, restored.word(i).rid, restored.word(i).cost), (3, 4, 1234));
        assert!(LearnedWords::deserialize("x", 10).is_err());
        assert!(LearnedWords::deserialize("suzuri-learned-words 1\n1\tかいぎ\t会議\n", 10).is_err());
    }
}
