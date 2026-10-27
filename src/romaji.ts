// ローマ字からひらがなへの変換。
// 打鍵ごとに Rust を呼ばずに済むよう、フロントエンドで行う。

const VOWELS = "aiueo";

/** 子音の綴り → あいうえお段のかな */
const ROWS: Record<string, string[]> = {
  "": ["あ", "い", "う", "え", "お"],
  k: ["か", "き", "く", "け", "こ"],
  s: ["さ", "し", "す", "せ", "そ"],
  t: ["た", "ち", "つ", "て", "と"],
  n: ["な", "に", "ぬ", "ね", "の"],
  h: ["は", "ひ", "ふ", "へ", "ほ"],
  m: ["ま", "み", "む", "め", "も"],
  y: ["や", "い", "ゆ", "いぇ", "よ"],
  r: ["ら", "り", "る", "れ", "ろ"],
  w: ["わ", "うぃ", "う", "うぇ", "を"],
  g: ["が", "ぎ", "ぐ", "げ", "ご"],
  z: ["ざ", "じ", "ず", "ぜ", "ぞ"],
  d: ["だ", "ぢ", "づ", "で", "ど"],
  b: ["ば", "び", "ぶ", "べ", "ぼ"],
  p: ["ぱ", "ぴ", "ぷ", "ぺ", "ぽ"],
  f: ["ふぁ", "ふぃ", "ふ", "ふぇ", "ふぉ"],
  j: ["じゃ", "じ", "じゅ", "じぇ", "じょ"],
  v: ["ゔぁ", "ゔぃ", "ゔ", "ゔぇ", "ゔぉ"],
  l: ["ぁ", "ぃ", "ぅ", "ぇ", "ぉ"],
  x: ["ぁ", "ぃ", "ぅ", "ぇ", "ぉ"],
  ky: ["きゃ", "きぃ", "きゅ", "きぇ", "きょ"],
  sy: ["しゃ", "しぃ", "しゅ", "しぇ", "しょ"],
  sh: ["しゃ", "し", "しゅ", "しぇ", "しょ"],
  ty: ["ちゃ", "ちぃ", "ちゅ", "ちぇ", "ちょ"],
  ch: ["ちゃ", "ち", "ちゅ", "ちぇ", "ちょ"],
  cy: ["ちゃ", "ちぃ", "ちゅ", "ちぇ", "ちょ"],
  ts: ["つぁ", "つぃ", "つ", "つぇ", "つぉ"],
  th: ["てゃ", "てぃ", "てゅ", "てぇ", "てょ"],
  dh: ["でゃ", "でぃ", "でゅ", "でぇ", "でょ"],
  tw: ["とぁ", "とぃ", "とぅ", "とぇ", "とぉ"],
  dw: ["どぁ", "どぃ", "どぅ", "どぇ", "どぉ"],
  ny: ["にゃ", "にぃ", "にゅ", "にぇ", "にょ"],
  hy: ["ひゃ", "ひぃ", "ひゅ", "ひぇ", "ひょ"],
  my: ["みゃ", "みぃ", "みゅ", "みぇ", "みょ"],
  ry: ["りゃ", "りぃ", "りゅ", "りぇ", "りょ"],
  gy: ["ぎゃ", "ぎぃ", "ぎゅ", "ぎぇ", "ぎょ"],
  zy: ["じゃ", "じぃ", "じゅ", "じぇ", "じょ"],
  jy: ["じゃ", "じぃ", "じゅ", "じぇ", "じょ"],
  dy: ["ぢゃ", "ぢぃ", "ぢゅ", "ぢぇ", "ぢょ"],
  by: ["びゃ", "びぃ", "びゅ", "びぇ", "びょ"],
  py: ["ぴゃ", "ぴぃ", "ぴゅ", "ぴぇ", "ぴょ"],
  fy: ["ふゃ", "ふぃ", "ふゅ", "ふぇ", "ふょ"],
  vy: ["ゔゃ", "ゔぃ", "ゔゅ", "ゔぇ", "ゔょ"],
  wh: ["うぁ", "うぃ", "う", "うぇ", "うぉ"],
  kw: ["くぁ", "くぃ", "くぅ", "くぇ", "くぉ"],
  gw: ["ぐぁ", "ぐぃ", "ぐぅ", "ぐぇ", "ぐぉ"],
  q: ["くぁ", "くぃ", "く", "くぇ", "くぉ"],
  ly: ["ゃ", "ぃ", "ゅ", "ぇ", "ょ"],
  xy: ["ゃ", "ぃ", "ゅ", "ぇ", "ょ"],
};

const TABLE: Record<string, string> = {
  ...Object.fromEntries(
    Object.entries(ROWS).flatMap(([c, kana]) => [...VOWELS].map((v, i) => [c + v, kana[i]])),
  ),
  shi: "し",
  chi: "ち",
  tsu: "つ",
  ltu: "っ",
  xtu: "っ",
  ltsu: "っ",
  lwa: "ゎ",
  xwa: "ゎ",
  lka: "ゕ",
  xka: "ゕ",
  lke: "ゖ",
  xke: "ゖ",
  nn: "ん",
  "n'": "ん",
  xn: "ん",
  "-": "ー",
  ",": "、",
  ".": "。",
  "?": "？",
  "!": "！",
  "[": "「",
  "]": "」",
  "~": "～",
  "/": "・",
};

/** 表の綴りの途中になりうる文字列 */
const PREFIXES = new Set(Object.keys(TABLE).flatMap((k) => [...k].map((_, i) => k.slice(0, i + 1))));

export type RomajiResult = {
  /** 確定したかな */
  kana: string;
  /** まだかなにならないローマ字 */
  pending: string;
};

/**
 * 未確定のローマ字 `pending` に1文字足し、確定したかなと残りを返す。
 * `prev` は入力位置の直前の文字。数字の直後の「.」「,」は「3.14」「1,000」のため半角のまま入れる
 */
export function feed(pending: string, ch: string, prev = ""): RomajiResult {
  if (pending === "" && (ch === "." || ch === ",") && /^[0-9]$/.test(prev)) {
    return { kana: ch, pending: "" };
  }
  let buf = pending + ch.toLowerCase();
  let kana = "";
  while (buf.length > 0) {
    // 表の綴りはどれも、ほかの綴りの先頭部分になっていない
    if (TABLE[buf] !== undefined) return { kana: kana + TABLE[buf], pending: "" };
    if (PREFIXES.has(buf)) return { kana, pending: buf };
    const [head, next] = [buf[0], buf[1]];
    if (head === next && /[a-z]/.test(head) && !VOWELS.includes(head)) {
      // 子音の重なりは促音
      kana += "っ";
    } else if (head === "n") {
      kana += "ん";
    } else {
      // 綴りにならない文字はそのまま残す
      kana += head;
    }
    buf = buf.slice(1);
  }
  return { kana, pending: "" };
}

/** 変換を始めるときなど、未確定のローマ字を確定させる。末尾の n は ん にする */
export function flush(pending: string): string {
  if (pending === "n") return "ん";
  return pending;
}

/** 文字列をまとめて変換する（テストと貼り付け用） */
export function toHiragana(text: string): string {
  let pending = "";
  let out = "";
  for (const ch of text) {
    const r = feed(pending, ch, out.slice(-1));
    out += r.kana;
    pending = r.pending;
  }
  return out + flush(pending);
}
