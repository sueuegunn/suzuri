// かな・英数の書き換え。F6〜F10 で文節の表記を切り替えるときに使う。
// スズリは打った打鍵を覚えていないので、英字にするときは読みのかなからローマ字に戻す。

import { toHiragana } from "./romaji";

/** ひらがなをカタカナにする（ほかの文字はそのまま） */
export function toKatakana(text: string): string {
  return text.replace(/[ぁ-ゖ]/g, (c) => String.fromCharCode(c.charCodeAt(0) + 0x60));
}

/** カタカナをひらがなにする（ほかの文字はそのまま） */
export function katakanaToHiragana(text: string): string {
  return text.replace(/[ァ-ヶ]/g, (c) => String.fromCharCode(c.charCodeAt(0) - 0x60));
}

const FULL_KANA = "ァアィイゥウェエォオカキクケコサシスセソタチッツテトナニヌネノハヒフヘホマミムメモャヤュユョヨラリルレロワヲンー・「」、。";
const HALF_KANA = "ｧｱｨｲｩｳｪｴｫｵｶｷｸｹｺｻｼｽｾｿﾀﾁｯﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓｬﾔｭﾕｮﾖﾗﾘﾙﾚﾛﾜｦﾝｰ･｢｣､｡";
const TO_HALF_KANA = new Map(Array.from(FULL_KANA).map((c, i) => [c, HALF_KANA[i]]));

/** 全角カタカナを半角カタカナにする。濁点・半濁点は分けて ﾞ ﾟ にする（ガ → ｶﾞ）。半角のないもの（ヮ など）はそのまま */
export function toHalfKatakana(text: string): string {
  return Array.from(text.normalize("NFD"))
    .map((c) => (c === "゙" ? "ﾞ" : c === "゚" ? "ﾟ" : (TO_HALF_KANA.get(c) ?? c)))
    .join("")
    .normalize("NFC");
}

/** 半角の英数字・記号を全角にする。半角スペースは全角スペースにする */
export function toFullAscii(text: string): string {
  return text.replace(/[\x20-\x7e]/g, (c) => (c === " " ? "　" : String.fromCharCode(c.charCodeAt(0) + 0xfee0)));
}

/** 全角の英数字・記号を半角にする */
export function toHalfAscii(text: string): string {
  return text.replace(/[！-～　]/g, (c) => (c === "　" ? " " : String.fromCharCode(c.charCodeAt(0) - 0xfee0)));
}

/** かな1文字 → ローマ字。読みやすいヘボン式寄りにする（し → shi、つ → tsu） */
const SINGLE: Record<string, string> = {
  あ: "a", い: "i", う: "u", え: "e", お: "o",
  か: "ka", き: "ki", く: "ku", け: "ke", こ: "ko",
  さ: "sa", し: "shi", す: "su", せ: "se", そ: "so",
  た: "ta", ち: "chi", つ: "tsu", て: "te", と: "to",
  な: "na", に: "ni", ぬ: "nu", ね: "ne", の: "no",
  は: "ha", ひ: "hi", ふ: "fu", へ: "he", ほ: "ho",
  ま: "ma", み: "mi", む: "mu", め: "me", も: "mo",
  や: "ya", ゆ: "yu", よ: "yo",
  ら: "ra", り: "ri", る: "ru", れ: "re", ろ: "ro",
  わ: "wa", ゐ: "wi", ゑ: "we", を: "wo",
  が: "ga", ぎ: "gi", ぐ: "gu", げ: "ge", ご: "go",
  ざ: "za", じ: "ji", ず: "zu", ぜ: "ze", ぞ: "zo",
  だ: "da", ぢ: "di", づ: "du", で: "de", ど: "do",
  ば: "ba", び: "bi", ぶ: "bu", べ: "be", ぼ: "bo",
  ぱ: "pa", ぴ: "pi", ぷ: "pu", ぺ: "pe", ぽ: "po",
  ゔ: "vu",
  ぁ: "xa", ぃ: "xi", ぅ: "xu", ぇ: "xe", ぉ: "xo",
  ゃ: "xya", ゅ: "xyu", ょ: "xyo", っ: "xtu", ゎ: "xwa",
  ー: "-", "、": ",", "。": ".", "？": "?", "！": "!", "「": "[", "」": "]", "～": "~", "・": "/",
};

/** 拗音などの2文字の組み合わせ（きゃ → kya、ふぁ → fa） */
const DOUBLE: Record<string, string> = (() => {
  const table: Record<string, string> = {};
  // い段 + ゃゅょ。し・ち・じは sh・ch・j に、ほかは i を y にする
  const yHeads: Record<string, string> = {
    き: "ky", し: "sh", ち: "ch", に: "ny", ひ: "hy", み: "my", り: "ry",
    ぎ: "gy", じ: "j", ぢ: "dy", び: "by", ぴ: "py",
  };
  for (const [kana, head] of Object.entries(yHeads)) {
    table[kana + "ゃ"] = head + "a";
    table[kana + "ゅ"] = head + "u";
    table[kana + "ょ"] = head + "o";
  }
  table["しぇ"] = "she";
  table["ちぇ"] = "che";
  table["じぇ"] = "je";
  // 小さい母音と組む音
  const vowelHeads: Record<string, string> = { ふ: "f", ゔ: "v" };
  for (const [kana, head] of Object.entries(vowelHeads)) {
    table[kana + "ぁ"] = head + "a";
    table[kana + "ぃ"] = head + "i";
    table[kana + "ぇ"] = head + "e";
    table[kana + "ぉ"] = head + "o";
  }
  table["てぃ"] = "thi";
  table["でぃ"] = "dhi";
  table["とぅ"] = "twu";
  table["どぅ"] = "dwu";
  table["うぃ"] = "wi";
  table["うぇ"] = "we";
  table["うぉ"] = "who";
  return table;
})();

/**
 * かなの読みをローマ字にする。カタカナもひらがなとして扱い、英数字などはそのまま。
 * 「っ」は次の子音を重ね（がっこう → gakkou）、「ん」は母音・や行・な行の前では nn にする（こんいん → konnin）
 */
export function toRomaji(text: string): string {
  const chars = Array.from(katakanaToHiragana(text));
  const out: string[] = [];
  const at = (i: number) => DOUBLE[(chars[i] ?? "") + (chars[i + 1] ?? "")] ?? SINGLE[chars[i]];
  for (let i = 0; i < chars.length; i++) {
    const c = chars[i];
    if (c === "っ") {
      const next = at(i + 1);
      out.push(next && /^[bcdfghjklmpqrstvwxz]/.test(next) ? next[0] : "xtu");
      continue;
    }
    if (c === "ん") {
      const next = at(i + 1);
      out.push(next && /^[aiueoyn]/.test(next) ? "nn" : "n");
      continue;
    }
    const pair = DOUBLE[c + (chars[i + 1] ?? "")];
    if (pair) {
      out.push(pair);
      i++;
      continue;
    }
    out.push(SINGLE[c] ?? c);
  }
  return out.join("");
}

/** F6〜F10 で文節の表記を切り替えるキー */
export type FunctionKey = "F6" | "F7" | "F8" | "F9" | "F10";

export function isFunctionKey(key: string): key is FunctionKey {
  return key === "F6" || key === "F7" || key === "F8" || key === "F9" || key === "F10";
}

/**
 * 文節の読みを、ふつうの IME の F6〜F10 と同じように書き換える。
 * F6 ひらがな、F7 全角カタカナ、F8 半角カタカナ、F9 全角英数、F10 半角英数。
 * 英字だけの読み（英数モードで打った語）は、F6〜F8 ではローマ字として読んでかなにする
 */
export function applyFunctionKey(key: FunctionKey, reading: string): string {
  const ascii = /^[\x20-\x7e]+$/.test(reading);
  const hiragana = ascii ? toHiragana(reading.toLowerCase()) : katakanaToHiragana(reading);
  switch (key) {
    case "F6":
      return hiragana;
    case "F7":
      return toKatakana(hiragana);
    case "F8":
      return toHalfKatakana(toKatakana(hiragana));
    case "F9":
      return toFullAscii(ascii ? reading : toRomaji(reading));
    case "F10":
      return ascii ? reading : toRomaji(reading);
  }
}
