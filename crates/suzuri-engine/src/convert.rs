//! 文節の区切りと候補の生成。
//!
//! 1. 読みの各位置から辞書（システム辞書とユーザー辞書）を引いて単語を並べる（ラティス）
//! 2. Viterbi で、単語コストと接続コストの合計が最小の並びを求める
//! 3. 隣り合う単語の品詞から文節境界を判定する（Mozc の segmenter.def を辞書に焼き込んだもの）
//! 4. 文節ごとに、前後の文脈を固定してその範囲だけの上位候補を求める

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::UserData;
use crate::context::ContextKind;
use crate::dict::Dictionary;
use crate::history::Committed;
use crate::kana::{ascii_variants, hiragana_to_katakana, symbol_variants};

/// 辞書にない文字を1文字の単語として扱うときのコスト。辞書の単語より十分大きくする
const UNKNOWN_COST: i32 = 20_000;
/// ユーザー辞書の語のコスト。同じ読みのシステム辞書の語より出やすく、
/// ほかの語の一部に読みが現れたとき（「たなか」と「きたなかった」）にまで割り込みすぎない値にする
const USER_WORD_COST: i32 = 2000;
/// 区切りの学習で覚えた語を、覚えたときの単語の並びよりこれだけ出やすくする。
/// 必ずその区切りにはせず「なりやすい」程度にとどめる（ほかの文で誤って効いたときに直しやすいように）
const LEARNED_BONUS: i32 = 1000;
/// 辞書の単語の並びで作れない表記（カタカナ・ひらがなのまま）を覚えたときのコスト
const LEARNED_FALLBACK_COST: i32 = 4000;
/// これより短い文節は区切りの学習で覚えない（短い読みがほかの文で効きすぎないように）
const MIN_LEARNED_CHARS: usize = 2;
/// 半角数字の並び（「1000」「3.14」）を1語にしたときのコスト。Mozc の「いち → 1」（753）と同じくらいにする
const NUMBER_COST: i32 = 700;
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
    /// 半角数字の並びに付ける品詞。辞書で「0」を引いて得る（suzuri-dictgen が数として登録している）。
    /// 見つからなければ未知語の品詞を使う
    number_id: u16,
}

#[derive(Debug, Clone, Copy)]
struct Node {
    start: usize,
    end: usize,
    lid: u16,
    rid: u16,
    cost: i32,
    source: Source,
}

/// 単語がどこから来たか
#[derive(Debug, Clone, Copy)]
enum Source {
    /// 辞書にない文字や英数字の並び。表記は読みのまま
    Plain,
    /// システム辞書の単語の番号
    System(u32),
    /// ユーザー辞書の語の番号
    User(u32),
    /// 区切りの学習で覚えた語の番号
    Learned(u32),
}

struct Lattice<'a> {
    reading: &'a str,
    user: &'a UserData,
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
        let number_id = dict.lookup("0").first().map_or(dict.unknown_id(), |e| e.lid);
        Engine { dict, number_id }
    }

    pub fn dictionary(&self) -> &Dictionary {
        &self.dict
    }

    /// 読みを自動で文節に区切り、各文節の候補を返す
    pub fn convert(&self, reading: &str, user: &UserData) -> Conversion {
        let n = reading.chars().count();
        self.run(reading, &[], std::slice::from_ref(&(0..n)), user)
    }

    /// ユーザーが決めた区切り（文字位置）に従って、各文節の候補を返す
    pub fn convert_with_boundaries(
        &self,
        reading: &str,
        boundaries: &[usize],
        user: &UserData,
    ) -> Conversion {
        self.run(reading, boundaries, &[], user)
    }

    /// `boundaries` は保ったまま、`range` の内側だけ自動で区切り直す。
    /// ひらがなを直したとき、直した文節だけ区切り直すのに使う
    pub fn resegment(
        &self,
        reading: &str,
        boundaries: &[usize],
        range: Range<usize>,
        user: &UserData,
    ) -> Conversion {
        self.run(reading, boundaries, &[range], user)
    }

    /// `boundaries` は保ったまま、`ranges` のそれぞれの内側だけ自動で区切り直す。
    /// 選んだ範囲を1文節にしたとき、その両側に残った部分を区切り直すのに使う
    pub fn resegment_ranges(
        &self,
        reading: &str,
        boundaries: &[usize],
        ranges: &[Range<usize>],
        user: &UserData,
    ) -> Conversion {
        self.run(reading, boundaries, ranges, user)
    }

    fn run(
        &self,
        reading: &str,
        boundaries: &[usize],
        auto: &[Range<usize>],
        user: &UserData,
    ) -> Conversion {
        // 自動で区切る範囲の内側か（範囲の端は固定した区切りとして残す）
        let inside = |p: usize| auto.iter().any(|r| r.start < p && p < r.end);
        if reading.is_empty() {
            return Conversion { segments: vec![] };
        }
        let lat = self.build_lattice(reading, user, true);
        let n = lat.len();
        let mut fixed: Vec<usize> = boundaries
            .iter()
            .copied()
            .filter(|&b| 0 < b && b < n && !inside(b))
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
            if inside(b.start) && self.dict.is_boundary(a.rid, b.lid) {
                cuts.push(b.start);
            }
        }
        // スペースは前後の語と切り離し、それだけで1文節にする
        for (i, c) in reading.chars().enumerate() {
            if c.is_whitespace() {
                cuts.extend([i, i + 1].into_iter().filter(|&p| 0 < p && p < n && inside(p)));
            }
        }
        cuts.sort_unstable();
        cuts.dedup();

        let mut edges = vec![0];
        edges.extend(cuts);
        edges.push(n);
        // 左から順に作り、前の文節の第1候補の自立語を文脈として渡す
        let mut before: Vec<String> = Vec::new();
        let mut segments = Vec::with_capacity(edges.len() - 1);
        for w in edges.windows(2) {
            let (segment, content) = self.segment(&lat, &path, w[0]..w[1], user, &before);
            before.push(content);
            segments.push(segment);
        }
        Conversion { segments }
    }

    /// `with_learned` なら区切りの学習で覚えた語も足す
    fn build_lattice<'a>(&self, reading: &'a str, user: &'a UserData, with_learned: bool) -> Lattice<'a> {
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
                source: Source::Plain,
            });
            // 英数字の並び（「UI」「MP3」「3.14」）は辞書にないので、まとめて1語にする
            let in_run = |i: usize| in_ascii_run(&chars, i);
            if in_run(start) && (start == 0 || !in_run(start - 1)) {
                let end = (start..n).find(|&i| !in_run(i)).unwrap_or(n);
                // 数字だけなら数の品詞にし、後ろの助数詞（「1000円」）と1文節にまとまるようにする
                let (id, cost) = if chars[start..end].iter().all(|c| !c.is_ascii_alphabetic()) {
                    (self.number_id, NUMBER_COST)
                } else {
                    (unknown, UNKNOWN_COST)
                };
                if end - start > 1 || id != unknown {
                    push(Node {
                        start,
                        end,
                        lid: id,
                        rid: id,
                        cost,
                        source: Source::Plain,
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
                            source: Source::System(i),
                        });
                    }
                });
            // ユーザー辞書と区切りの学習は、それぞれ一番長い読みまで、長さごとに引く
            let dictionary = &user.dictionary;
            for end in start + 1..=n.min(start + dictionary.max_chars()) {
                for &i in dictionary.lookup(&reading[offsets[start]..offsets[end]]) {
                    let id = self.dict.user_pos_id(dictionary.word(i).pos);
                    push(Node {
                        start,
                        end,
                        lid: id,
                        rid: id,
                        cost: USER_WORD_COST,
                        source: Source::User(i),
                    });
                }
            }
            let learned = &user.learned;
            let max = if with_learned { learned.max_chars() } else { 0 };
            let pos_count = self.dict.pos_count();
            for end in start + 1..=n.min(start + max) {
                for &i in learned.lookup(&reading[offsets[start]..offsets[end]]) {
                    let w = learned.word(i);
                    // 辞書を作り直して品詞IDの数が変わっていたら使わない
                    if w.lid as usize >= pos_count || w.rid as usize >= pos_count {
                        continue;
                    }
                    push(Node {
                        start,
                        end,
                        lid: w.lid,
                        rid: w.rid,
                        cost: w.cost,
                        source: Source::Learned(i),
                    });
                }
            }
        }
        Lattice {
            reading,
            user,
            offsets,
            nodes,
            by_start,
            by_end,
        }
    }

    fn surface(&self, lat: &Lattice, node: &Node) -> String {
        let reading = lat.slice(node.start..node.end);
        match node.source {
            Source::System(i) => self.dict.entry(i).surface.resolve(reading).into_owned(),
            Source::User(i) => lat.user.dictionary.word(i).surface.clone(),
            Source::Learned(i) => lat.user.learned.word(i).surface.clone(),
            Source::Plain => reading.to_owned(),
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

    /// 前後の単語を文脈として固定し、`range` の中だけで上位の候補を求める。
    /// `before` は前の文節の第1候補の自立語（古い順）で、文脈つきの学習に使う。
    /// 文節と、その第1候補の自立語を返す
    fn segment(
        &self,
        lat: &Lattice,
        path: &[usize],
        range: Range<usize>,
        user: &UserData,
        before: &[String],
    ) -> (Segment, String) {
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
        let mut best = String::new();
        let mut best_content_end = 0;
        for node in path
            .iter()
            .map(|&i| &lat.nodes[i])
            .filter(|node| range.start <= node.start && node.end <= range.end)
        {
            best.push_str(&self.surface(lat, node));
            if !self.dict.is_functional(node.lid) {
                best_content_end = best.len();
            }
        }

        // 候補の表記 → 末尾の付属語を除いた部分の長さ
        let mut content_ends: HashMap<String, usize> = HashMap::new();
        content_ends.insert(best.clone(), best_content_end);
        let mut finals: Vec<(i32, String)> = self
            .beam(lat, range.clone(), left)
            .into_iter()
            .map(|x| {
                content_ends.entry(x.surface.clone()).or_insert(x.content_end);
                (x.cost + self.dict.connection_cost(x.rid, right), x.surface)
            })
            .collect();
        finals.sort();

        // 最良の並びの末尾の付属語。並びの分からない候補の自立語を求めるのに使う
        let tail = best[best_content_end..].to_owned();
        let reading = lat.slice(range.clone()).to_owned();
        // 英字は辞書で変換せず、書き分けだけを候補にする
        let mut variants = ascii_variants(&best);
        variants.extend(symbol_variants(&best));
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
        // 学習は、文脈なし → 前に出た自立語 → 直前の自立語の順に先頭へ移し、細かい文脈ほど前に来るようにする
        user.history.rank(&reading, &mut candidates);
        let (adjacent, topics) = match before.split_last() {
            Some((last, rest)) => (Some(last.as_str()), rest),
            None => (None, &[][..]),
        };
        user.context.rank(&reading, adjacent, topics, &mut candidates);
        for s in [reading.clone(), hiragana_to_katakana(&reading)] {
            if !candidates.contains(&s) {
                candidates.push(s);
            }
        }
        let content = content_of(&candidates[0], &content_ends, &tail);
        let segment = Segment {
            start: range.start,
            end: range.end,
            reading,
            candidates,
        };
        (segment, content)
    }

    /// `range` の中の単語の並びを、左の文脈 `left` から始めて上位だけ求める。コストは右の文脈とのつながりを含まない
    fn beam(&self, lat: &Lattice, range: Range<usize>, left: u16) -> Vec<Partial> {
        let mut beams: Vec<Vec<Partial>> = (range.start..=range.end).map(|_| Vec::new()).collect();
        beams[0].push(Partial {
            cost: 0,
            rid: left,
            lid: None,
            surface: String::new(),
            content_end: 0,
        });
        for p in range.start..range.end {
            let beam = prune(std::mem::take(&mut beams[p - range.start]));
            for &i in &lat.by_start[p] {
                let node = &lat.nodes[i];
                if node.end > range.end {
                    continue;
                }
                let word = self.surface(lat, node);
                let functional = self.dict.is_functional(node.lid);
                for prev in &beam {
                    let surface = format!("{}{}", prev.surface, word);
                    beams[node.end - range.start].push(Partial {
                        cost: prev.cost + self.dict.connection_cost(prev.rid, node.lid) + node.cost,
                        rid: node.rid,
                        lid: Some(prev.lid.unwrap_or(node.lid)),
                        // 付属語は自立語の表記に含めない
                        content_end: if functional { prev.content_end } else { surface.len() },
                        surface,
                    });
                }
            }
        }
        beams.pop().unwrap()
    }

    /// 文節の読みと選んだ表記から、先頭の左文脈ID・末尾の右文脈ID・内側のコスト・自立語の表記を求める。
    /// 覚えた語が自分自身を見てコストを下げ続けないよう、区切りの学習はラティスに入れない。
    /// 上位の並びに選んだ表記がなければ None
    fn analyze(&self, reading: &str, surface: &str, user: &UserData) -> Option<Analysis> {
        if reading.is_empty() {
            return None;
        }
        let lat = self.build_lattice(reading, user, false);
        let n = lat.len();
        self.beam(&lat, 0..n, BOS_EOS)
            .into_iter()
            .filter(|x| x.surface == surface)
            .min_by_key(|x| x.cost)
            .map(|x| {
                let lid = x.lid.unwrap_or(BOS_EOS);
                Analysis {
                    lid,
                    rid: x.rid,
                    cost: x.cost - self.dict.connection_cost(BOS_EOS, lid),
                    content: x.surface[..x.content_end].to_owned(),
                }
            })
    }

    /// 確定した文節を学習する。選んだ候補を覚え（文脈なしと文脈つき）、手で区切った文節を語として覚える
    pub fn commit(&self, user: &mut UserData, segments: &[CommitSegment]) {
        let committed: Vec<Committed> = segments
            .iter()
            .map(|s| Committed {
                reading: s.reading.clone(),
                surface: s.surface.clone(),
                was_first: s.was_first,
            })
            .collect();
        user.history.commit(&committed);

        let analyses: Vec<Option<Analysis>> = segments
            .iter()
            .map(|s| self.analyze(&s.reading, &s.surface, user))
            .collect();

        // 文脈つきの学習。第1候補以外を選んだときだけ新しく覚え、第1候補のままなら覚えているものの時刻だけを更新する
        let contents: Vec<&str> = segments
            .iter()
            .zip(&analyses)
            .map(|(s, a)| a.as_ref().map_or(s.surface.as_str(), |a| a.content.as_str()))
            .collect();
        for (i, s) in segments.iter().enumerate().skip(1) {
            let touch_only = s.was_first;
            user.context
                .record(ContextKind::Adjacent, contents[i - 1], &s.reading, &s.surface, touch_only);
            let mut seen = HashSet::new();
            for &c in contents[..i - 1].iter().filter(|c| seen.insert(**c)) {
                user.context.record(ContextKind::Topic, c, &s.reading, &s.surface, touch_only);
            }
        }

        // 区切りの学習
        for (s, a) in segments.iter().zip(&analyses) {
            if !s.manual || s.reading.chars().count() < MIN_LEARNED_CHARS {
                continue;
            }
            let (lid, rid, cost) = match a {
                Some(a) => (a.lid, a.rid, a.cost - LEARNED_BONUS),
                // カタカナやひらがなのままなど、辞書の単語の並びで作れない表記は名詞として覚える
                None => {
                    let id = self.dict.unknown_id();
                    (id, id, LEARNED_FALLBACK_COST)
                }
            };
            user.learned.learn(&s.reading, &s.surface, lid, rid, cost);
        }
    }
}

/// 候補の自立語（末尾の付属語を除いた表記）。候補を作った単語の並びが分かればそれで分け、
/// 分からない候補（学習で足したものなど）は、最良の並びの付属語と同じ末尾なら除く
fn content_of(candidate: &str, content_ends: &HashMap<String, usize>, tail: &str) -> String {
    if let Some(&end) = content_ends.get(candidate) {
        return candidate[..end].to_owned();
    }
    match candidate.strip_suffix(tail) {
        Some(head) if !tail.is_empty() => head.to_owned(),
        _ => candidate.to_owned(),
    }
}

/// 確定した文節1つ分
#[derive(Debug, Clone)]
pub struct CommitSegment {
    pub reading: String,
    pub surface: String,
    /// 第1候補のまま確定したか
    pub was_first: bool,
    /// 手で置いた区切り（ピン）に接しているか。区切りの学習の対象にする
    pub manual: bool,
}

/// 文節を単語に分けた結果
struct Analysis {
    lid: u16,
    rid: u16,
    /// 単語のコストと内側のつながりのコストの合計（前後とのつながりは含まない）
    cost: i32,
    /// 末尾の付属語を除いた表記
    content: String,
}

/// 1語にまとめる英数字の並びに入る文字か。数字に挟まれた「.」「,」も含める（「3.14」「1,000」）
fn in_ascii_run(chars: &[char], i: usize) -> bool {
    let c = chars[i];
    if c.is_ascii_alphanumeric() {
        return true;
    }
    let digit = |j: Option<usize>| j.and_then(|j| chars.get(j)).is_some_and(|c| c.is_ascii_digit());
    (c == '.' || c == ',') && digit(i.checked_sub(1)) && digit(Some(i + 1))
}

/// 文節の候補を作る途中の経路
struct Partial {
    cost: i32,
    rid: u16,
    /// 先頭の単語の左文脈ID。まだ単語がなければ None
    lid: Option<u16>,
    surface: String,
    /// surface のうち、末尾の付属語を除いた部分の長さ（バイト）
    content_end: usize,
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
    use crate::user_dict::UserPos;

    // 0: BOS/EOS, 1: 名詞, 2: 助詞, 3: 動詞, 4: 未知語
    const NOUN: u16 = 1;
    const PARTICLE: u16 = 2;
    const VERB: u16 = 3;

    fn engine() -> Engine {
        let mut b = DictionaryBuilder::new(5, 4);
        b.set_user_pos(UserPos::Noun, NOUN);
        b.set_functional(PARTICLE, true);
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
        let c = e.convert("わたしははしをわたる", &UserData::default());
        assert_eq!(view(&c), [(0, 4, "私は"), (4, 7, "橋を"), (7, 10, "渡る")]);
    }

    #[test]
    fn follows_given_boundaries() {
        let e = engine();
        let c = e.convert_with_boundaries("わたしははし", &[2], &UserData::default());
        assert_eq!(view(&c), [(0, 2, "綿"), (2, 6, "氏は橋")]);
    }

    #[test]
    fn lists_alternatives_and_kana() {
        let e = engine();
        let c = e.convert_with_boundaries("はし", &[], &UserData::default());
        let cands = &c.segments[0].candidates;
        assert_eq!(&cands[..2], ["橋", "箸"]);
        assert!(cands.contains(&"はし".to_string()));
        assert_eq!(cands.last().unwrap(), "ハシ");
    }

    #[test]
    fn learned_candidate_comes_first() {
        let e = engine();
        let mut user = UserData::default();
        user.history.commit(&[Committed {
            reading: "はし".into(),
            surface: "箸".into(),
            was_first: false,
        }]);
        let c = e.convert_with_boundaries("はし", &[], &user);
        assert_eq!(c.segments[0].candidates[0], "箸");
    }

    #[test]
    fn user_word_comes_first() {
        let e = engine();
        let mut user = UserData::default();
        user.dictionary.add("はし", "ハシ社", UserPos::Noun).unwrap();
        let c = e.convert_with_boundaries("はし", &[], &user);
        assert_eq!(&c.segments[0].candidates[..3], ["ハシ社", "橋", "箸"]);
    }

    #[test]
    fn user_word_is_not_split() {
        let e = engine();
        let mut user = UserData::default();
        // 登録しなければ「私は｜橋」
        user.dictionary.add("ははし", "ハハシ", UserPos::Noun).unwrap();
        let c = e.convert("わたしははし", &user);
        assert_eq!(view(&c), [(0, 3, "私"), (3, 6, "ハハシ")]);
    }

    fn manual(reading: &str, surface: &str) -> CommitSegment {
        CommitSegment {
            reading: reading.into(),
            surface: surface.into(),
            was_first: false,
            manual: true,
        }
    }

    #[test]
    fn learns_manual_segments() {
        let e = engine();
        let mut user = UserData::default();
        let before = e.convert("わたしははしをわたる", &user);
        assert_eq!(view(&before), [(0, 4, "私は"), (4, 7, "橋を"), (7, 10, "渡る")]);
        // 「わたしははし」を手で1文節にして確定すると、次から1文節になりやすい
        e.commit(&mut user, &[manual("わたしははし", "私は橋")]);
        let after = e.convert("わたしははしをわたる", &user);
        assert_eq!(view(&after), [(0, 7, "私は橋を"), (7, 10, "渡る")]);
    }

    #[test]
    fn learns_only_manual_and_long_segments() {
        let e = engine();
        let mut user = UserData::default();
        let mut auto = manual("わたしは", "私は");
        auto.manual = false;
        e.commit(&mut user, &[auto, manual("は", "歯")]);
        assert!(user.learned.is_empty());
    }

    #[test]
    fn context_beats_plain_learning() {
        let e = engine();
        let mut user = UserData::default();
        let seg = |reading: &str, surface: &str, was_first: bool| CommitSegment {
            reading: reading.into(),
            surface: surface.into(),
            was_first,
            manual: false,
        };
        // 「私は」のあとで「箸」を選び、そのあと別の文脈で「橋」を選ぶ
        e.commit(&mut user, &[seg("わたしは", "私は", true), seg("はし", "箸", false)]);
        e.commit(&mut user, &[seg("わたる", "渡る", true), seg("はし", "橋", false)]);
        // 文脈なしでは最後に選んだ「橋」
        assert_eq!(e.convert_with_boundaries("はし", &[], &user).segments[0].candidates[0], "橋");
        // 「私は」のあと（自立語「私」）では「箸」
        let c = e.convert_with_boundaries("わたしははし", &[4], &user);
        assert_eq!(view(&c), [(0, 4, "私は"), (4, 6, "箸")]);
    }

    #[test]
    fn relearning_does_not_lower_cost_again() {
        let e = engine();
        let mut user = UserData::default();
        e.commit(&mut user, &[manual("わたしははし", "私は橋")]);
        let cost = |u: &UserData| u.learned.word(u.learned.lookup("わたしははし")[0]).cost;
        let first = cost(&user);
        e.commit(&mut user, &[manual("わたしははし", "私は橋")]);
        assert_eq!(cost(&user), first);
        // 辞書の単語の並びで作れない表記は名詞として覚える
        e.commit(&mut user, &[manual("わたしは", "ワタシハ")]);
        let w = user.learned.word(user.learned.lookup("わたしは")[0]);
        assert_eq!((w.lid, w.cost), (4, LEARNED_FALLBACK_COST));
    }

    #[test]
    fn resegments_only_inside_range() {
        let e = engine();
        // 0..4 は1文節に固定し、4..10 だけ自動で区切る
        let c = e.resegment("わたしははしをわたる", &[4], 4..10, &UserData::default());
        assert_eq!(view(&c), [(0, 4, "私は"), (4, 7, "橋を"), (7, 10, "渡る")]);
        let c = e.resegment("わたしははしをわたる", &[2], 4..10, &UserData::default());
        assert_eq!(c.segments[0].end, 2);
    }

    #[test]
    fn resegments_several_ranges() {
        let e = engine();
        let h = UserData::default();
        // 4..7 を1文節に固定し、その両側（0..4 と 7..10）をそれぞれ自動で区切る
        let c = e.resegment_ranges("わたしははしをわたる", &[1, 4, 7, 9], &[0..4, 7..10], &h);
        assert_eq!(view(&c), [(0, 4, "私は"), (4, 7, "橋を"), (7, 10, "渡る")]);
        // 範囲の外の区切りは保つ
        let c = e.resegment_ranges("わたしははしをわたる", &[2, 4, 7], std::slice::from_ref(&(7..10)), &h);
        assert_eq!(c.segments[0].end, 2);
        // 範囲がなければ区切りの指定どおり
        let c = e.resegment_ranges("わたしははし", &[2], &[], &h);
        assert_eq!(view(&c), [(0, 2, "綿"), (2, 6, "氏は橋")]);
    }

    #[test]
    fn keeps_ascii_run_as_one_word() {
        let e = engine();
        let c = e.convert("NPMをわたる", &UserData::default());
        assert_eq!(view(&c), [(0, 4, "NPMを"), (4, 7, "渡る")]);
        assert_eq!(&c.segments[0].candidates[..4], ["NPMを", "npmを", "Npmを", "ＮＰＭを"]);
        let c = e.convert("UI", &UserData::default());
        assert_eq!(c.segments[0].candidates, ["UI", "ui", "Ui", "ＵＩ"]);
        let c = e.convert("A", &UserData::default());
        assert_eq!(c.segments[0].candidates, ["A", "a", "Ａ"]);
        // 小文字で打ったときも、大文字をホイールで選べる
        let c = e.convert("npmを", &UserData::default());
        assert_eq!(&c.segments[0].candidates[..4], ["npmを", "NPMを", "Npmを", "ｎｐｍを"]);
    }

    #[test]
    fn keeps_decimal_number_as_one_word() {
        let e = engine();
        let c = e.convert("3.14を1,000", &UserData::default());
        let readings: Vec<&str> = c.segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, ["3.14を", "1,000"]);
    }

    #[test]
    fn lists_symbol_variants() {
        let e = engine();
        let c = e.convert("。", &UserData::default());
        assert_eq!(c.segments[0].candidates, ["。", ".", "．"]);
    }

    #[test]
    fn makes_space_its_own_segment() {
        let e = engine();
        let c = e.convert("UI をわたる", &UserData::default());
        let readings: Vec<&str> = c.segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, ["UI", " ", "を", "わたる"]);
        assert_eq!(c.segments[1].candidates, [" ", "　"]);
    }

    #[test]
    fn handles_unknown_characters() {
        let e = engine();
        let c = e.convert("ぬ1", &UserData::default());
        let text: String = c.segments.iter().map(|s| s.candidates[0].as_str()).collect();
        assert_eq!(text, "ぬ1");
        assert!(e.convert("", &UserData::default()).segments.is_empty());
    }
}
