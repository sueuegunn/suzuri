//! 文節の区切りと候補の生成。
//!
//! 1. 読みの各位置から辞書を引いて単語を並べる（ラティス）
//! 2. Viterbi で、単語コストと接続コストの合計が最小の並びを求める
//! 3. 隣り合う単語の品詞から文節境界を判定する（Mozc の segmenter.def を辞書に焼き込んだもの）
//! 4. 文節ごとに、前後の文脈を固定してその範囲だけの上位候補を求める

use std::collections::HashSet;
use std::ops::Range;

use crate::dict::Dictionary;
use crate::history::UserHistory;
use crate::kana::{ascii_variants, hiragana_to_katakana};

/// 辞書にない文字を1文字の単語として扱うときのコスト。辞書の単語より十分大きくする
const UNKNOWN_COST: i32 = 20_000;
/// 文節ごとに保持する途中経路の数
const BEAM_WIDTH: usize = 50;
/// 文節ごとに返す候補の最大数（ひらがな・カタカナは別に足す）
const MAX_CANDIDATES: usize = 30;
const BOS_EOS: u16 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// 読みの中での位置（文字単位）
    pub start: usize,
    pub end: usize,
    pub reading: String,
    /// 先頭が推奨の候補
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    pub segments: Vec<Segment>,
}

pub struct Engine {
    dict: Dictionary,
}

#[derive(Debug, Clone, Copy)]
struct Node {
    start: usize,
    end: usize,
    lid: u16,
    rid: u16,
    cost: i32,
    /// 辞書の単語の番号。None は未知語
    entry: Option<u32>,
}

struct Lattice<'a> {
    reading: &'a str,
    /// 文字位置 → バイト位置（末尾を含む）
    offsets: Vec<usize>,
    nodes: Vec<Node>,
    by_start: Vec<Vec<usize>>,
    by_end: Vec<Vec<usize>>,
}

impl Lattice<'_> {
    fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    fn slice(&self, range: Range<usize>) -> &str {
        &self.reading[self.offsets[range.start]..self.offsets[range.end]]
    }
}

impl Engine {
    pub fn new(dict: Dictionary) -> Self {
        Engine { dict }
    }

    pub fn dictionary(&self) -> &Dictionary {
        &self.dict
    }

    /// 読みを自動で文節に区切り、各文節の候補を返す
    pub fn convert(&self, reading: &str, history: &UserHistory) -> Conversion {
        let n = reading.chars().count();
        self.run(reading, &[], 0..n, history)
    }

    /// ユーザーが決めた区切り（文字位置）に従って、各文節の候補を返す
    pub fn convert_with_boundaries(
        &self,
        reading: &str,
        boundaries: &[usize],
        history: &UserHistory,
    ) -> Conversion {
        self.run(reading, boundaries, 0..0, history)
    }

    /// `boundaries` は保ったまま、`range` の内側だけ自動で区切り直す。
    /// ひらがなを直したとき、直した文節だけ区切り直すのに使う
    pub fn resegment(
        &self,
        reading: &str,
        boundaries: &[usize],
        range: Range<usize>,
        history: &UserHistory,
    ) -> Conversion {
        self.run(reading, boundaries, range, history)
    }

    fn run(
        &self,
        reading: &str,
        boundaries: &[usize],
        auto: Range<usize>,
        history: &UserHistory,
    ) -> Conversion {
        if reading.is_empty() {
            return Conversion { segments: vec![] };
        }
        let lat = self.build_lattice(reading);
        let n = lat.len();
        let mut fixed: Vec<usize> = boundaries
            .iter()
            .copied()
            .filter(|&b| 0 < b && b < n && !(auto.start < b && b < auto.end))
            .collect();
        fixed.sort_unstable();
        fixed.dedup();
        let mut is_fixed = vec![false; n + 1];
        for &b in &fixed {
            is_fixed[b] = true;
        }

        let path = self.best_path(&lat, &is_fixed);

        // 自動で区切る範囲では、最良経路の品詞の並びから境界を足す
        let mut cuts = fixed;
        for w in path.windows(2) {
            let (a, b) = (&lat.nodes[w[0]], &lat.nodes[w[1]]);
            if auto.start < b.start && b.start < auto.end && self.dict.is_boundary(a.rid, b.lid) {
                cuts.push(b.start);
            }
        }
        cuts.sort_unstable();
        cuts.dedup();

        let mut edges = vec![0];
        edges.extend(cuts);
        edges.push(n);
        let segments = edges
            .windows(2)
            .map(|w| self.segment(&lat, &path, w[0]..w[1], history))
            .collect();
        Conversion { segments }
    }

    fn build_lattice<'a>(&self, reading: &'a str) -> Lattice<'a> {
        let offsets: Vec<usize> = reading
            .char_indices()
            .map(|(i, _)| i)
            .chain([reading.len()])
            .collect();
        let n = offsets.len() - 1;
        let mut nodes = Vec::new();
        let mut by_start = vec![Vec::new(); n + 1];
        let mut by_end = vec![Vec::new(); n + 1];
        let unknown = self.dict.unknown_id();
        let chars: Vec<char> = reading.chars().collect();
        for start in 0..n {
            let mut push = |node: Node| {
                by_start[node.start].push(nodes.len());
                by_end[node.end].push(nodes.len());
                nodes.push(node);
            };
            // 辞書の単語の有無にかかわらず未知語を置き、必ず経路がつながるようにする
            push(Node {
                start,
                end: start + 1,
                lid: unknown,
                rid: unknown,
                cost: UNKNOWN_COST,
                entry: None,
            });
            // 英数字の並び（「UI」「MP3」）は辞書にないので、まとめて1語にする
            if chars[start].is_ascii_alphanumeric() && (start == 0 || !chars[start - 1].is_ascii_alphanumeric()) {
                let end = (start..n).find(|&i| !chars[i].is_ascii_alphanumeric()).unwrap_or(n);
                if end - start > 1 {
                    push(Node {
                        start,
                        end,
                        lid: unknown,
                        rid: unknown,
                        cost: UNKNOWN_COST,
                        entry: None,
                    });
                }
            }
            self.dict
                .common_prefix_search(&reading[offsets[start]..], |len, entries| {
                    let end = offsets
                        .binary_search(&(offsets[start] + len))
                        .expect("読みは文字の境界で終わる");
                    for i in entries {
                        let e = self.dict.entry(i);
                        push(Node {
                            start,
                            end,
                            lid: e.lid,
                            rid: e.rid,
                            cost: e.cost as i32,
                            entry: Some(i),
                        });
                    }
                });
        }
        Lattice {
            reading,
            offsets,
            nodes,
            by_start,
            by_end,
        }
    }

    fn surface(&self, lat: &Lattice, node: &Node) -> String {
        let reading = lat.slice(node.start..node.end);
        match node.entry {
            Some(i) => self.dict.entry(i).surface.resolve(reading).into_owned(),
            None => reading.to_owned(),
        }
    }

    /// 固定した境界をまたがない範囲で、コスト最小の経路（ノード番号の列）を返す
    fn best_path(&self, lat: &Lattice, is_fixed: &[bool]) -> Vec<usize> {
        let n = lat.len();
        let allowed = |node: &Node| !(node.start + 1..node.end).any(|p| is_fixed[p]);
        let mut cost = vec![i32::MAX; lat.nodes.len()];
        let mut back = vec![usize::MAX; lat.nodes.len()];
        for p in 0..n {
            for &i in &lat.by_start[p] {
                let node = &lat.nodes[i];
                if !allowed(node) {
                    continue;
                }
                let (best, from) = if p == 0 {
                    (self.dict.connection_cost(BOS_EOS, node.lid), usize::MAX)
                } else {
                    lat.by_end[p]
                        .iter()
                        .filter(|&&j| cost[j] != i32::MAX)
                        .map(|&j| (cost[j] + self.dict.connection_cost(lat.nodes[j].rid, node.lid), j))
                        .min()
                        .unwrap_or((i32::MAX, usize::MAX))
                };
                if best != i32::MAX {
                    cost[i] = best + node.cost;
                    back[i] = from;
                }
            }
        }
        let mut last = lat.by_end[n]
            .iter()
            .filter(|&&j| cost[j] != i32::MAX)
            .min_by_key(|&&j| cost[j] + self.dict.connection_cost(lat.nodes[j].rid, BOS_EOS))
            .copied()
            .expect("未知語があるので経路は必ずある");
        let mut path = vec![last];
        while back[last] != usize::MAX {
            last = back[last];
            path.push(last);
        }
        path.reverse();
        path
    }

    /// 前後の単語を文脈として固定し、`range` の中だけで上位の候補を求める
    fn segment(&self, lat: &Lattice, path: &[usize], range: Range<usize>, history: &UserHistory) -> Segment {
        let left = path
            .iter()
            .map(|&i| &lat.nodes[i])
            .find(|node| node.end == range.start)
            .map_or(BOS_EOS, |node| node.rid);
        let right = path
            .iter()
            .map(|&i| &lat.nodes[i])
            .find(|node| node.start == range.end)
            .map_or(BOS_EOS, |node| node.lid);
        let best: String = path
            .iter()
            .map(|&i| &lat.nodes[i])
            .filter(|node| range.start <= node.start && node.end <= range.end)
            .map(|node| self.surface(lat, node))
            .collect();

        let mut beams: Vec<Vec<Partial>> = (range.start..=range.end).map(|_| Vec::new()).collect();
        beams[0].push(Partial {
            cost: 0,
            rid: left,
            surface: String::new(),
        });
        for p in range.start..range.end {
            let beam = prune(std::mem::take(&mut beams[p - range.start]));
            for &i in &lat.by_start[p] {
                let node = &lat.nodes[i];
                if node.end > range.end {
                    continue;
                }
                let word = self.surface(lat, node);
                for prev in &beam {
                    beams[node.end - range.start].push(Partial {
                        cost: prev.cost + self.dict.connection_cost(prev.rid, node.lid) + node.cost,
                        rid: node.rid,
                        surface: format!("{}{}", prev.surface, word),
                    });
                }
            }
        }
        let mut finals: Vec<(i32, String)> = beams
            .pop()
            .unwrap()
            .into_iter()
            .map(|x| (x.cost + self.dict.connection_cost(x.rid, right), x.surface))
            .collect();
        finals.sort();

        let reading = lat.slice(range.clone()).to_owned();
        // 英字は辞書で変換せず、書き分けだけを候補にする
        let variants = ascii_variants(&best);
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        for s in [best].into_iter().chain(variants) {
            if seen.insert(s.clone()) {
                candidates.push(s);
            }
        }
        for (_, s) in finals {
            if candidates.len() >= MAX_CANDIDATES {
                break;
            }
            if seen.insert(s.clone()) {
                candidates.push(s);
            }
        }
        history.rank(&reading, &mut candidates);
        for s in [reading.clone(), hiragana_to_katakana(&reading)] {
            if !candidates.contains(&s) {
                candidates.push(s);
            }
        }
        Segment {
            start: range.start,
            end: range.end,
            reading,
            candidates,
        }
    }
}

/// 文節の候補を作る途中の経路
struct Partial {
    cost: i32,
    rid: u16,
    surface: String,
}

/// コストの低い順に並べ、同じ (右文脈, 表記) を1つにまとめて上位だけ残す
fn prune(mut items: Vec<Partial>) -> Vec<Partial> {
    items.sort_by_key(|x| x.cost);
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|x| seen.insert((x.rid, x.surface.clone())))
        .take(BEAM_WIDTH)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::{DictionaryBuilder, RawEntry};
    use crate::history::Committed;

    // 0: BOS/EOS, 1: 名詞, 2: 助詞, 3: 動詞, 4: 未知語
    const NOUN: u16 = 1;
    const PARTICLE: u16 = 2;
    const VERB: u16 = 3;

    fn engine() -> Engine {
        let mut b = DictionaryBuilder::new(5, 4);
        for rid in 0..5 {
            for lid in 0..5 {
                b.set_connection(rid, lid, 1000);
                // 助詞の前では区切らない
                b.set_boundary(rid, lid, lid != PARTICLE || rid == 0);
            }
        }
        b.set_connection(NOUN, PARTICLE, 100);
        b.set_connection(PARTICLE, NOUN, 200);
        b.set_connection(PARTICLE, VERB, 200);
        let words: &[(&str, u16, i16, &str)] = &[
            ("わたし", NOUN, 3000, "私"),
            ("わたし", NOUN, 4000, "渡し"),
            ("は", PARTICLE, 500, "は"),
            ("は", NOUN, 4000, "歯"),
            ("はし", NOUN, 3000, "橋"),
            ("はし", NOUN, 3200, "箸"),
            ("を", PARTICLE, 500, "を"),
            ("わたる", VERB, 3000, "渡る"),
            ("わた", NOUN, 3500, "綿"),
            ("し", NOUN, 4000, "氏"),
        ];
        for &(reading, id, cost, surface) in words {
            b.add_entry(RawEntry {
                reading: reading.into(),
                lid: id,
                rid: id,
                cost,
                surface: surface.into(),
            });
        }
        Engine::new(Dictionary::from_bytes(b.build().unwrap()).unwrap())
    }

    fn view(c: &Conversion) -> Vec<(usize, usize, &str)> {
        c.segments
            .iter()
            .map(|s| (s.start, s.end, s.candidates[0].as_str()))
            .collect()
    }

    #[test]
    fn segments_automatically() {
        let e = engine();
        let c = e.convert("わたしははしをわたる", &UserHistory::default());
        assert_eq!(view(&c), [(0, 4, "私は"), (4, 7, "橋を"), (7, 10, "渡る")]);
    }

    #[test]
    fn follows_given_boundaries() {
        let e = engine();
        let c = e.convert_with_boundaries("わたしははし", &[2], &UserHistory::default());
        assert_eq!(view(&c), [(0, 2, "綿"), (2, 6, "氏は橋")]);
    }

    #[test]
    fn lists_alternatives_and_kana() {
        let e = engine();
        let c = e.convert_with_boundaries("はし", &[], &UserHistory::default());
        let cands = &c.segments[0].candidates;
        assert_eq!(&cands[..2], ["橋", "箸"]);
        assert!(cands.contains(&"はし".to_string()));
        assert_eq!(cands.last().unwrap(), "ハシ");
    }

    #[test]
    fn learned_candidate_comes_first() {
        let e = engine();
        let mut h = UserHistory::default();
        h.commit(&[Committed {
            reading: "はし".into(),
            surface: "箸".into(),
            was_first: false,
        }]);
        let c = e.convert_with_boundaries("はし", &[], &h);
        assert_eq!(c.segments[0].candidates[0], "箸");
    }

    #[test]
    fn resegments_only_inside_range() {
        let e = engine();
        // 0..4 は1文節に固定し、4..10 だけ自動で区切る
        let c = e.resegment("わたしははしをわたる", &[4], 4..10, &UserHistory::default());
        assert_eq!(view(&c), [(0, 4, "私は"), (4, 7, "橋を"), (7, 10, "渡る")]);
        let c = e.resegment("わたしははしをわたる", &[2], 4..10, &UserHistory::default());
        assert_eq!(c.segments[0].end, 2);
    }

    #[test]
    fn keeps_ascii_run_as_one_word() {
        let e = engine();
        let c = e.convert("NPMをわたる", &UserHistory::default());
        assert_eq!(view(&c), [(0, 4, "NPMを"), (4, 7, "渡る")]);
        assert_eq!(&c.segments[0].candidates[..4], ["NPMを", "npmを", "Npmを", "ＮＰＭを"]);
        let c = e.convert("UI", &UserHistory::default());
        assert_eq!(c.segments[0].candidates, ["UI", "ui", "Ui", "ＵＩ"]);
        let c = e.convert("A", &UserHistory::default());
        assert_eq!(c.segments[0].candidates, ["A", "a", "Ａ"]);
    }

    #[test]
    fn handles_unknown_characters() {
        let e = engine();
        let c = e.convert("ぬ1", &UserHistory::default());
        let text: String = c.segments.iter().map(|s| s.candidates[0].as_str()).collect();
        assert_eq!(text, "ぬ1");
        assert!(e.convert("", &UserHistory::default()).segments.is_empty());
    }
}
