//! 文脈つきの学習（docs/design.md）。
//!
//! 確定したとき「前に出た自立語・読み → 選んだ表記」を覚え、次に同じ文脈で同じ読みが来たら、その表記を先頭に出す。
//! 文脈は2種類で、細かい順に引く。
//! - 直前の文節の自立語（「会議で」「会議の」はどちらも「会議」）
//! - 同じ入力の中で、それより前に出た自立語（話題として効く）
//!
//! 変換中はメモリ上のハッシュ表だけを引く。件数に上限を設け、古く使ったものから捨てる。

use std::collections::HashMap;

use crate::history::HistoryError;

pub const DEFAULT_CAPACITY: usize = 50_000;
const HEADER: &str = "suzuri-context-history 1";

/// どの文脈で覚えたか
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextKind {
    /// 直前の文節の自立語
    Adjacent,
    /// 同じ入力の中で、それより前に出た自立語
    Topic,
}

impl ContextKind {
    fn tag(self) -> &'static str {
        match self {
            ContextKind::Adjacent => "a",
            ContextKind::Topic => "t",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "a" => Some(ContextKind::Adjacent),
            "t" => Some(ContextKind::Topic),
            _ => None,
        }
    }
}

/// 読み → (種類, 文脈の自立語) → [(表記, 最後に使った時刻)]
type Table = HashMap<String, HashMap<(ContextKind, String), Vec<(String, u64)>>>;

pub struct ContextHistory {
    entries: Table,
    len: usize,
    clock: u64,
    capacity: usize,
}

impl Default for ContextHistory {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl ContextHistory {
    pub fn new(capacity: usize) -> Self {
        ContextHistory {
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

    fn contains(&self, kind: ContextKind, context: &str, reading: &str, surface: &str) -> bool {
        self.entries
            .get(reading)
            .and_then(|m| m.get(&(kind, context.to_owned())))
            .is_some_and(|v| v.iter().any(|(s, _)| s == surface))
    }

    /// 覚える。`touch_only` なら、すでに覚えているものの時刻だけを更新する（第1候補のまま確定したとき）
    pub fn record(&mut self, kind: ContextKind, context: &str, reading: &str, surface: &str, touch_only: bool) {
        if context.is_empty() || (touch_only && !self.contains(kind, context, reading, surface)) {
            return;
        }
        self.clock += 1;
        let clock = self.clock;
        let list = self
            .entries
            .entry(reading.to_owned())
            .or_default()
            .entry((kind, context.to_owned()))
            .or_default();
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
        let mut clocks: Vec<u64> = self
            .entries
            .values()
            .flat_map(|m| m.values().flatten().map(|(_, t)| *t))
            .collect();
        let cut = clocks.len() - self.capacity;
        let (_, threshold, _) = clocks.select_nth_unstable(cut - 1);
        let threshold = *threshold;
        self.entries.retain(|_, m| {
            m.retain(|_, list| {
                list.retain(|(_, t)| *t > threshold);
                !list.is_empty()
            });
            !m.is_empty()
        });
        self.len = self.entries.values().map(|m| m.values().map(Vec::len).sum::<usize>()).sum();
    }

    /// 文脈で覚えた表記を候補の先頭へ移す。直前の自立語で覚えたものを一番前に、
    /// その次に前に出た自立語で覚えたものを置く。それぞれの中では新しく使った順
    pub fn rank(&self, reading: &str, adjacent: Option<&str>, topics: &[String], candidates: &mut Vec<String>) {
        let Some(map) = self.entries.get(reading) else { return };
        let mut topic: Vec<(&str, u64)> = Vec::new();
        for t in topics {
            if let Some(list) = map.get(&(ContextKind::Topic, t.clone())) {
                for (s, clock) in list {
                    match topic.iter_mut().find(|(x, _)| x == s) {
                        Some(item) => item.1 = item.1.max(*clock),
                        None => topic.push((s, *clock)),
                    }
                }
            }
        }
        move_to_front(candidates, topic);
        if let Some(list) = adjacent.and_then(|a| map.get(&(ContextKind::Adjacent, a.to_owned()))) {
            move_to_front(candidates, list.iter().map(|(s, t)| (s.as_str(), *t)).collect());
        }
    }

    /// 保存用のテキストにする。1行に「時刻 TAB 種類 TAB 文脈 TAB 読み TAB 表記」
    pub fn serialize(&self) -> String {
        let mut rows: Vec<(u64, &str, &str, &str, &str)> = Vec::with_capacity(self.len);
        for (reading, map) in &self.entries {
            for ((kind, context), list) in map {
                for (surface, t) in list {
                    rows.push((*t, kind.tag(), context, reading, surface));
                }
            }
        }
        rows.sort_unstable();
        let mut out = String::from(HEADER);
        out.push('\n');
        for (t, k, c, r, s) in rows {
            out.push_str(&format!("{t}\t{k}\t{c}\t{r}\t{s}\n"));
        }
        out
    }

    pub fn deserialize(text: &str, capacity: usize) -> Result<Self, HistoryError> {
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return Err(HistoryError::new("文脈つきの学習の見出し行が違います"));
        }
        let mut rows = Vec::new();
        for (i, line) in lines.enumerate() {
            let broken = || HistoryError::new(format!("文脈つきの学習の{}行目の形式が違います", i + 2));
            let cols: Vec<&str> = line.split('\t').collect();
            let [t, k, c, r, s] = cols[..] else {
                return Err(broken());
            };
            let t: u64 = t.parse().map_err(|_| broken())?;
            let kind = ContextKind::from_tag(k).ok_or_else(broken)?;
            if c.is_empty() || r.is_empty() || s.is_empty() {
                return Err(broken());
            }
            rows.push((t, kind, c, r, s));
        }
        // 古い順に入れ直せば、時刻の順序と上限の処理がそのまま保たれる
        rows.sort_unstable_by_key(|row| row.0);
        let mut history = ContextHistory::new(capacity);
        for (_, kind, c, r, s) in rows {
            history.record(kind, c, r, s, false);
        }
        Ok(history)
    }
}

/// `learned` の表記を新しく使った順に `candidates` の先頭へ移す。候補になければ足す
fn move_to_front(candidates: &mut Vec<String>, mut learned: Vec<(&str, u64)>) {
    if learned.is_empty() {
        return;
    }
    learned.sort_by_key(|(_, t)| std::cmp::Reverse(*t));
    let front: Vec<String> = learned.into_iter().map(|(s, _)| s.to_owned()).collect();
    candidates.retain(|c| !front.contains(c));
    candidates.splice(0..0, front);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranked(h: &ContextHistory, adjacent: Option<&str>, topics: &[&str]) -> Vec<String> {
        let topics: Vec<String> = topics.iter().map(|s| s.to_string()).collect();
        let mut c: Vec<String> = ["発現", "発言", "発験"].map(String::from).to_vec();
        h.rank("はつげん", adjacent, &topics, &mut c);
        c
    }

    #[test]
    fn adjacent_comes_before_topic() {
        let mut h = ContextHistory::default();
        h.record(ContextKind::Topic, "遺伝子", "はつげん", "発現", false);
        h.record(ContextKind::Adjacent, "会議", "はつげん", "発言", false);
        // 文脈が合わなければ並びを変えない
        assert_eq!(ranked(&h, Some("机"), &["椅子"]), ["発現", "発言", "発験"]);
        assert_eq!(ranked(&h, None, &["遺伝子"]), ["発現", "発言", "発験"]);
        assert_eq!(ranked(&h, Some("会議"), &[]), ["発言", "発現", "発験"]);
        // 直前の文脈が話題より優先される
        assert_eq!(ranked(&h, Some("会議"), &["遺伝子"]), ["発言", "発現", "発験"]);
    }

    #[test]
    fn touch_only_does_not_add() {
        let mut h = ContextHistory::default();
        h.record(ContextKind::Adjacent, "会議", "はつげん", "発言", true);
        assert!(h.is_empty());
        h.record(ContextKind::Adjacent, "", "はつげん", "発言", false);
        assert!(h.is_empty());
    }

    #[test]
    fn evicts_oldest_beyond_capacity() {
        let mut h = ContextHistory::new(10);
        for i in 0..100 {
            h.record(ContextKind::Topic, &format!("語{i}"), "よみ", "表記", false);
            assert!(h.len() <= 11);
        }
        let mut c = Vec::new();
        h.rank("よみ", None, &["語0".into()], &mut c);
        assert!(c.is_empty());
        h.rank("よみ", None, &["語99".into()], &mut c);
        assert_eq!(c, ["表記"]);
    }

    #[test]
    fn roundtrips_through_text() {
        let mut h = ContextHistory::default();
        h.record(ContextKind::Adjacent, "会議", "はつげん", "発言", false);
        h.record(ContextKind::Topic, "遺伝子", "はつげん", "発現", false);
        let restored = ContextHistory::deserialize(&h.serialize(), DEFAULT_CAPACITY).unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(ranked(&restored, Some("会議"), &["遺伝子"]), ["発言", "発現", "発験"]);
        assert!(ContextHistory::deserialize("suzuri-context-history 1\n1\tx\t会議\tはつげん\t発言\n", 10).is_err());
    }
}
