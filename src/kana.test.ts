import { describe, expect, it } from "vitest";
import { applyFunctionKey, toFullAscii, toHalfAscii, toHalfKatakana, toKatakana, toRomaji } from "./kana";

describe("toRomaji", () => {
  it("ヘボン式寄りのローマ字にする", () => {
    expect(toRomaji("きょうはいいてんきですね")).toBe("kyouhaiitenkidesune");
    expect(toRomaji("しちつふじ")).toBe("shichitsufuji");
    expect(toRomaji("ちゃんと")).toBe("chanto");
  });
  it("っは次の子音を重ねる", () => {
    expect(toRomaji("がっこう")).toBe("gakkou");
    expect(toRomaji("まっちゃ")).toBe("maccha");
    // 子音で始まらない前（文末など）は xtu にする
    expect(toRomaji("あっ")).toBe("axtu");
  });
  it("んは母音・や行・な行の前で nn にする", () => {
    expect(toRomaji("こんいん")).toBe("konnin");
    expect(toRomaji("ほんや")).toBe("honnya");
    expect(toRomaji("こんな")).toBe("konnna");
    expect(toRomaji("さんま")).toBe("sanma");
  });
  it("カタカナ・小さい母音・記号も戻し、英数字はそのまま", () => {
    expect(toRomaji("ファイル")).toBe("fairu");
    expect(toRomaji("パーティー")).toBe("pa-thi-");
    expect(toRomaji("ね、MP3。")).toBe("ne,MP3.");
  });
});

describe("書き分け", () => {
  it("カタカナ", () => {
    expect(toKatakana("がっこう")).toBe("ガッコウ");
    expect(toHalfKatakana("ガッコウ・パーティー")).toBe("ｶﾞｯｺｳ･ﾊﾟｰﾃｨｰ");
    expect(toHalfKatakana("ヴ")).toBe("ｳﾞ");
  });
  it("全角と半角の英数", () => {
    expect(toFullAscii("MP3 ok")).toBe("ＭＰ３　ｏｋ");
    expect(toHalfAscii("ＭＰ３　ｏｋ")).toBe("MP3 ok");
  });
});

describe("applyFunctionKey", () => {
  it("かなの読みを F6〜F10 で書き換える", () => {
    expect(applyFunctionKey("F6", "がっこう")).toBe("がっこう");
    expect(applyFunctionKey("F7", "がっこう")).toBe("ガッコウ");
    expect(applyFunctionKey("F8", "がっこう")).toBe("ｶﾞｯｺｳ");
    expect(applyFunctionKey("F9", "がっこう")).toBe("ｇａｋｋｏｕ");
    expect(applyFunctionKey("F10", "がっこう")).toBe("gakkou");
  });
  it("英字だけの読みは、F6〜F8 ではローマ字として読む", () => {
    expect(applyFunctionKey("F6", "sushi")).toBe("すし");
    expect(applyFunctionKey("F7", "Sushi")).toBe("スシ");
    expect(applyFunctionKey("F9", "npm")).toBe("ｎｐｍ");
    expect(applyFunctionKey("F10", "npm")).toBe("npm");
  });
});
