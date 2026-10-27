//! 候補の学習。
//!
//! 入力を重くしないため、次のことを守る（docs/design.md の「学習」）。
//! - 件数に上限を設け、超えたら最も古く使われたものから捨てる
//! - 変換中はメモリ上のハッシュ表だけを引く。ファイルの読み書きは呼び出し側が別スレッドで行う

use std::collections::HashMap;
use std::fmt;

pub const DEFAULT_CAPACITY: usize = 10_000;
const HEADER: &str = "suzuri-history 1";

/// 確定した文節1つ分
#[derive(Debug, Clone)]
pub struct Committed {
    pub reading: String,
    pub surface: String,
    /// 第1候補のまま確定したか
    pub was_first: bool,
}

#[derive(Debug)]
pub struct HistoryError(String);

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "学習データが壊れています: {}", self.0)
    }
}

impl std::error::Error for HistoryError {}

pub struct UserHistory {
    /// 読み → (表記, 最後に使った時刻)
    entries: HashMap<String, Vec<(String, u64)>>,
    len: usize,
    clock: u64,
    capacity: usize,
}

impl Default for UserHistory {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl UserHistory {
    pub fn new(capacity: usize) -> Self {
        UserHistory {
            entries: HashMap::new(),
            len: 0,
            clock: 0,
            capacity: capacity.max(1),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 確定した文節を覚える。
    /// 第1候補のまま確定した文節は、すでに覚えているものだけ時刻を更新する（件数を無駄に増やさない）。
    pub fn commit(&mut self, committed: &[Committed]) {
        for c in committed {
            if c.was_first && !self.contains(&c.reading, &c.surface) {
                continue;
            }
            self.record(&c.reading, &c.surface);
        }
    }

    fn contains(&self, reading: &str, surface: &str) -> bool {
        self.entries
            .get(reading)
            .is_some_and(|v| v.iter().any(|(s, _)| s == surface))
    }

    fn record(&mut self, reading: &str, surface: &str) {
        self.clock += 1;
        let clock = self.clock;
        let list = self.entries.entry(reading.to_owned()).or_default();
        if let Some(item) = list.iter_mut().find(|(s, _)| s == surface) {
            item.1 = clock;
        } else {
            list.push((surface.to_owned(), clock));
            self.len += 1;
            self.evict_if_needed();
        }
    }

    /// 上限を1割超えたら上限まで減らす。1件ごとに捨てると毎回全件を見ることになるため、まとめて行う
    fn evict_if_needed(&mut self) {
        if self.len <= self.capacity + (self.capacity / 10).max(1) {
            return;
        }
        let mut clocks: Vec<u64> = self.entries.values().flatten().map(|(_, t)| *t).collect();
        let cut = clocks.len() - self.capacity;
        let (_, threshold, _) = clocks.select_nth_unstable(cut - 1);
        let threshold = *threshold;
        self.entries.retain(|_, list| {
            list.retain(|(_, t)| *t > threshold);
            !list.is_empty()
        });
        self.len = self.entries.values().map(Vec::len).sum();
    }

    /// 覚えている表記を、新しく使った順に候補の先頭へ移す
    pub fn rank(&self, reading: &str, candidates: &mut Vec<String>) {
        let Some(list) = self.entries.get(reading) else { return };
        let mut learned: Vec<&(String, u64)> = list.iter().collect();
        learned.sort_by(|a, b| b.1.cmp(&a.1));
        let learned: Vec<String> = learned.into_iter().map(|(s, _)| s.clone()).collect();
        candidates.retain(|c| !learned.contains(c));
        candidates.splice(0..0, learned);
    }

    /// 保存用のテキストにする。1行に「時刻 TAB 読み TAB 表記」
    pub fn serialize(&self) -> String {
        let mut rows: Vec<(u64, &str, &str)> = self
            .entries
            .iter()
            .flat_map(|(r, list)| list.iter().map(move |(s, t)| (*t, r.as_str(), s.as_str())))
            .collect();
        rows.sort_unstable();
        let mut out = String::from(HEADER);
        out.push('\n');
        for (t, r, s) in rows {
            out.push_str(&format!("{t}\t{r}\t{s}\n"));
        }
        out
    }

    pub fn deserialize(text: &str, capacity: usize) -> Result<Self, HistoryError> {
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return Err(HistoryError("見出し行が違います".into()));
        }
        let mut history = UserHistory::new(capacity);
        let mut rows = Vec::new();
        for (i, line) in lines.enumerate() {
            let mut cols = line.split('\t');
            let (Some(t), Some(r), Some(s), None) = (cols.next(), cols.next(), cols.next(), cols.next())
            else {
                return Err(HistoryError(format!("{}行目の形式が違います", i + 2)));
            };
            let t: u64 = t
                .parse()
                .map_err(|_| HistoryError(format!("{}行目の時刻が数値ではありません", i + 2)))?;
            if r.is_empty() || s.is_empty() {
                return Err(HistoryError(format!("{}行目が空です", i + 2)));
            }
            rows.push((t, r, s));
        }
        // 古い順に入れ直せば、時刻の順序と上限の処理がそのまま保たれる
        rows.sort_unstable();
        for (_, r, s) in rows {
            history.record(r, s);
        }
        Ok(history)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(h: &mut UserHistory, reading: &str, surface: &str, was_first: bool) {
        h.commit(&[Committed {
            reading: reading.into(),
            surface: surface.into(),
            was_first,
        }]);
    }

    fn ranked(h: &UserHistory, reading: &str, candidates: &[&str]) -> Vec<String> {
        let mut c: Vec<String> = candidates.iter().map(|s| s.to_string()).collect();
        h.rank(reading, &mut c);
        c
    }

    #[test]
    fn moves_latest_choice_to_front() {
        let mut h = UserHistory::default();
        commit(&mut h, "はし", "箸", false);
        assert_eq!(ranked(&h, "はし", &["橋", "箸", "端"]), ["箸", "橋", "端"]);
        commit(&mut h, "はし", "端", false);
        assert_eq!(ranked(&h, "はし", &["橋", "箸", "端"]), ["端", "箸", "橋"]);
    }

    #[test]
    fn ignores_unlearned_first_candidate() {
        let mut h = UserHistory::default();
        commit(&mut h, "はし", "橋", true);
        assert!(h.is_empty());
        commit(&mut h, "はし", "箸", false);
        commit(&mut h, "はし", "箸", true);
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn evicts_oldest_beyond_capacity() {
        let mut h = UserHistory::new(10);
        for i in 0..100 {
            commit(&mut h, &format!("よみ{i}"), "表記", false);
            assert!(h.len() <= 11);
        }
        assert_eq!(ranked(&h, "よみ99", &[]), ["表記"]);
        assert!(ranked(&h, "よみ0", &[]).is_empty());
    }

    #[test]
    fn roundtrips_through_text() {
        let mut h = UserHistory::default();
        commit(&mut h, "はし", "箸", false);
        commit(&mut h, "はし", "端", false);
        commit(&mut h, "かみ", "紙", false);
        let restored = UserHistory::deserialize(&h.serialize(), DEFAULT_CAPACITY).unwrap();
        assert_eq!(restored.len(), 3);
        assert_eq!(ranked(&restored, "はし", &[]), ["端", "箸"]);
    }

    #[test]
    fn rejects_broken_text() {
        assert!(UserHistory::deserialize("", 10).is_err());
        assert!(UserHistory::deserialize("suzuri-history 1\nx\tはし\t箸\n", 10).is_err());
        assert!(UserHistory::deserialize("suzuri-history 1\n1\tはし\n", 10).is_err());
    }
}
