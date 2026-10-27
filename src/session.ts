// 入力中の文の状態と、その操作。描画やエンジンの呼び出しを含まない純粋な処理だけを置く。

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
};

export const emptySession: Session = { kana: "", caret: 0, pending: "", segments: null };

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
    let nb: number;
    if (b < pos) nb = b;
    else if (b <= pos + removed) {
      // 区切りの位置に入力したときは、左の文節に足す
      nb = removed === 0 && pos > 0 ? b + inserted : pos + inserted;
    } else nb = b - removed + inserted;
    if (nb > 0 && nb < newLength) out.add(nb);
  }
  return [...out].sort((a, b) => a - b);
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

/** ひらがなを直す前の文字位置を、直したあとの位置に読み替える */
export function shiftPosition(x: number, pos: number, removed: number, inserted: number): number {
  if (x <= pos) return x;
  if (x >= pos + removed) return x - removed + inserted;
  return pos;
}

/**
 * 新しい変換結果に、前の選択を引き継ぐ。
 * 範囲と読みが変わらない文節は、前に選んでいた表記を選んだままにする。
 * `shift` は前の文節の位置を新しい位置に読み替える関数（ひらがなを直したとき用）
 */
export function carrySelections(
  previous: Segment[] | null,
  next: EngineSegment[],
  shift: (pos: number) => number = (p) => p,
): Segment[] {
  const chosen = new Map<string, string>();
  for (const s of previous ?? []) {
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
