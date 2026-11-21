// 入力中の文の状態と、その操作。描画やエンジンの呼び出しを含まない純粋な処理だけを置く。

import { feed, isRomajiPrefix } from "./romaji";

/** エンジンが返す文節 */
export type EngineSegment = {
  start: number;
  end: number;
  reading: string;
  candidates: string[];
};

export type Segment = EngineSegment & {
  /** 選んでいる候補の番号 */
  selected: number;
};

export type Session = {
  /** 確定したひらがな（読み） */
  kana: string;
  /** 入力カーソルの位置（文字単位） */
  caret: number;
  /** まだかなにならないローマ字 */
  pending: string;
  /** 変換前は null */
  segments: Segment[] | null;
  /** ユーザーが手で置いた・動かした区切り。打っている途中に区切り直さない */
  pinned: number[];
  /**
   * かなモードで打ったが、かなにならずに英字のまま読みに入った文字の位置（小さい順）。
   * あとでその直後に英字を打つと、打ちかけのローマ字に戻して合わせる。
   * 英数モードや Shift で打った英字、貼り付けた英字は含めず、ほかの英字と合わせない
   */
  romaji: number[];
};

export const emptySession: Session = { kana: "", caret: 0, pending: "", segments: null, pinned: [], romaji: [] };

export function length(text: string): number {
  return Array.from(text).length;
}

/** 文節の区切り（先頭と末尾を除く文字位置） */
export function boundariesOf(segments: Segment[]): number[] {
  return segments.slice(1).map((s) => s.start);
}

/** 文字位置 `pos` から `removed` 文字を消し、`inserted` 文字を入れたあとの区切り */
export function shiftBoundaries(
  boundaries: number[],
  pos: number,
  removed: number,
  inserted: number,
  newLength: number,
): number[] {
  const out = new Set<number>();
  for (const b of boundaries) {
    const nb = shiftBoundary(b, pos, removed, inserted);
    if (nb > 0 && nb < newLength) out.add(nb);
  }
  return [...out].sort((a, b) => a - b);
}

/** 区切り1つ分の `shiftBoundaries`。先頭と末尾以外の区切りに使う */
function shiftBoundary(b: number, pos: number, removed: number, inserted: number): number {
  if (b < pos) return b;
  // 区切りの位置に入力したときは、左の文節に足す
  if (b <= pos + removed) return removed === 0 && pos > 0 ? b + inserted : pos + inserted;
  return b - removed + inserted;
}

/**
 * ひらがなを直したあとの読み `kana` に合わせて、文節の位置と読みをずらす（`shiftBoundaries` と同じ規則）。
 * 空になった文節は除く。候補はそのまま残す（変換の結果が届くまで、前の表記を出しておく）。
 * 変換の結果を待たずに次の打鍵が来ても（Backspace の押しっぱなし）、区切りを今の読みから求められるようにするため。
 * 古い文節のまま次の打鍵の分だけずらすと、届かなかった打鍵の分だけ区切りがずれる
 */
export function shiftSegments(
  segments: Segment[],
  kana: string,
  pos: number,
  removed: number,
  inserted: number,
): Segment[] {
  const chars = Array.from(kana);
  const n = chars.length;
  const last = segments.length === 0 ? 0 : segments[segments.length - 1].end;
  const shift = (b: number) =>
    b === 0 ? 0 : b >= last ? n : Math.min(n, Math.max(0, shiftBoundary(b, pos, removed, inserted)));
  return segments.flatMap((seg) => {
    const [start, end] = [shift(seg.start), shift(seg.end)];
    if (start >= end) return [];
    const reading = chars.slice(start, end).join("");
    return [{ ...seg, start, end, reading }];
  });
}

/**
 * 直した文字 `from`〜`to` を含む文節の範囲。
 * 区切りちょうどで消したときは、両側の文節を含める
 */
export function rangeAround(boundaries: number[], from: number, to: number, n: number): [number, number] {
  const start = Math.max(0, ...boundaries.filter((b) => b < from));
  const end = Math.min(n, ...boundaries.filter((b) => b >= to && b > from));
  return [start, end];
}

/** 打ったときに、打った位置からこれだけ前（文字数）までに始まる文節を区切り直す */
export const RETYPE_WINDOW = 12;

/** 文の切れ目になる文字。この直後の区切りは、打つそばから区切り直すときに越えない */
const SENTENCE_BREAKS = new Set(["、", "。", "？", "！", "\n"]);

/** 区切りのうち、句読点や改行の直後にあるもの（読みの文字位置） */
export function punctuationFences(kana: string, boundaries: number[]): number[] {
  const chars = Array.from(kana);
  return boundaries.filter((b) => b > 0 && SENTENCE_BREAKS.has(chars[b - 1]));
}

/**
 * 打ったときに区切り直す範囲。直した文節に加えて、1つ前の文節と、打った位置から
 * RETYPE_WINDOW 文字以内に始まる文節を含める。打っている途中は語の切れ目が決まりきらず
 * （「くるま」の時点で区切ると「車｜で」と割れる、「あにめー」で「アニメ｜ー」と割れる）、
 * 前の文節とつなぎ直せるようにするため。それより前は、打つたびに入れ替わってちらつかないよう触らない。
 * 手で置いた区切り（pinned）と、候補を選んだ文節の両端（fixed）と、句読点の直後の区切り（fences）は越えない。
 * `rightIntact` は、消した文字のすぐ後ろが元から文節の切れ目だったか。そのときは右の文節の文字は
 * 変わっていないので区切り直さない（前の文を Backspace で消すたびに、後ろの文の先頭が割られ直さないように）
 */
export function retypeRange(
  boundaries: number[],
  pinned: number[],
  from: number,
  to: number,
  n: number,
  fixed: number[] = [],
  fences: number[] = [],
  rightIntact = false,
): [number, number] {
  const [start, around] = rangeAround(boundaries, from, to, n);
  const floor = Math.max(0, ...[...pinned, ...fixed].filter((b) => b <= start), ...fences.filter((b) => b <= from));
  const previous = Math.max(0, ...boundaries.filter((b) => b < start));
  const near = Math.min(previous, ...boundaries.filter((b) => b >= from - RETYPE_WINDOW && b < start));
  const reach = from - RETYPE_WINDOW <= 0 ? 0 : near;
  const begin = Math.max(floor, reach);
  const right = rightIntact && from === to && boundaries.includes(from) ? from : around;
  const end = Math.min(right, ...fences.filter((b) => b >= to && b > begin));
  return [begin, Math.max(begin, end)];
}

/**
 * `from`〜`to` の文字を1つの文節にしたときの区切りと手で決めた区切り、自動で区切り直す範囲。
 * 内側の区切りは外し、両端を手で決めた区切りにする。両端が文節の途中だったら、
 * 外に残った部分をその外隣の文節とあわせて区切り直す（残りが助詞だけになったら隣に付けられるように）。
 * ただし手で決めた区切りは越えない。範囲が空なら null
 */
export function mergeRange(
  boundaries: number[],
  pinned: number[],
  from: number,
  to: number,
  n: number,
): { boundaries: number[]; pinned: number[]; ranges: [number, number][] } | null {
  [from, to] = [Math.max(0, Math.min(from, to)), Math.min(n, Math.max(from, to))];
  if (from >= to) return null;
  const ends = [from, to].filter((b) => b > 0 && b < n);
  const keep = (b: number) => b < from || b > to;
  const ranges: [number, number][] = [];
  // 左に残った部分。from が文節の途中のときだけ
  const left = Math.max(0, ...boundaries.filter((b) => b <= from));
  if (left < from) {
    const start = left === 0 || pinned.includes(left) ? left : Math.max(0, ...boundaries.filter((b) => b < left));
    ranges.push([start, from]);
  }
  const right = Math.min(n, ...boundaries.filter((b) => b >= to));
  if (to < right) {
    const end = right === n || pinned.includes(right) ? right : Math.min(n, ...boundaries.filter((b) => b > right));
    ranges.push([to, end]);
  }
  return {
    boundaries: [...new Set([...boundaries.filter(keep), ...ends])].sort((a, b) => a - b),
    pinned: [...new Set([...pinned.filter(keep), ...ends])].sort((a, b) => a - b),
    ranges,
  };
}

/** ひらがなを直す前の文字位置を、直したあとの位置に読み替える */
export function shiftPosition(x: number, pos: number, removed: number, inserted: number): number {
  if (x <= pos) return x;
  if (x >= pos + removed) return x - removed + inserted;
  return pos;
}

/**
 * 新しい変換結果に、前の選択を引き継ぐ。
 * 範囲と読みが変わらない文節は、前に選んでいた表記を選んだままにする。
 * 第1候補のままだった文節は引き継がない。学習で候補の順が変わったとき（前の文節が変わって
 * 文脈つきの学習が効くなど）に、新しい第1候補ではなく前の第1候補が2番目以降として選ばれたまま残るため
 * `shift` は前の文節の位置を新しい位置に読み替える関数（ひらがなを直したとき用）
 */
export function carrySelections(
  previous: Segment[] | null,
  next: EngineSegment[],
  shift: (pos: number) => number = (p) => p,
): Segment[] {
  const chosen = new Map<string, string>();
  for (const s of previous ?? []) {
    if (s.selected === 0) continue;
    chosen.set(`${shift(s.start)}:${shift(s.end)}:${s.reading}`, s.candidates[s.selected]);
  }
  return next.map((s) => {
    const surface = chosen.get(`${s.start}:${s.end}:${s.reading}`);
    const index = surface === undefined ? -1 : s.candidates.indexOf(surface);
    return { ...s, selected: Math.max(index, 0) };
  });
}

/** ピンを動かせる範囲（両隣のピンの内側）に収める。動かせないときは null */
export function clampPin(boundaries: number[], index: number, target: number, n: number): number | null {
  const lo = index > 0 ? boundaries[index - 1] + 1 : 1;
  const hi = index < boundaries.length - 1 ? boundaries[index + 1] - 1 : n - 1;
  if (lo > hi) return null;
  return Math.min(hi, Math.max(lo, target));
}

/** クリップボードに入れる文 */
export function resultText(session: Session): string {
  if (!session.segments) return session.kana + session.pending;
  return session.segments.map((s) => s.candidates[s.selected]).join("");
}

/** 文字列の一部を置き換える（文字単位） */
export function splice(text: string, pos: number, removed: number, inserted: string): string {
  const chars = Array.from(text);
  chars.splice(pos, removed, ...Array.from(inserted));
  return chars.join("");
}

/** 文字位置 `pos` から `removed` 文字を消し、`inserted` 文字を入れたあとの、英字のまま残ったローマ字の位置 */
export function shiftRomaji(marks: number[], pos: number, removed: number, inserted: number): number[] {
  return marks.filter((x) => x < pos || x >= pos + removed).map((x) => (x < pos ? x : x - removed + inserted));
}

/** `text` を文字位置 `pos` に入れたときの、その中の英字の位置。かなモードで英字のまま残ったローマ字に印を付ける */
export function romajiLetters(text: string, pos: number): number[] {
  return Array.from(text).flatMap((c, i) => (/^[a-z]$/.test(c) ? [pos + i] : []));
}

/** 印を足した、英字のまま残ったローマ字の位置 */
export function addRomaji(marks: number[], added: number[]): number[] {
  return added.length === 0 ? marks : [...new Set([...marks, ...added])].sort((a, b) => a - b);
}

/** かなモードで1文字打ったときの、読みの書き換え。`pos` から `removed` 文字を `kana` に置き換える */
export type RomajiEdit = { pos: number; removed: number; kana: string; pending: string };

/**
 * かなモードで1文字打ったときの読みの書き換え。入力カーソルの直前に英字のまま残ったローマ字があれば、
 * 打ちかけのローマ字の頭に戻して打った文字と合わせる（「k」が残ったあとで a を打つと「か」）。
 * 合わさらないとき（「k」のあとに「x」）は戻さない
 */
export function typeRomaji(s: Session, ch: string): RomajiEdit {
  const chars = Array.from(s.kana);
  let run = 0;
  while (run < s.caret && s.romaji.includes(s.caret - run - 1)) run++;
  for (let k = run; k > 0; k--) {
    const head = chars.slice(s.caret - k, s.caret).join("");
    if (!isRomajiPrefix(head + s.pending)) continue;
    const r = feed(head + s.pending, ch, chars[s.caret - k - 1]);
    if (r.kana.startsWith(head)) continue;
    return { pos: s.caret - k, removed: k, ...r };
  }
  return { pos: s.caret, removed: 0, ...feed(s.pending, ch, chars[s.caret - 1]) };
}
