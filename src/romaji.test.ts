import { describe, expect, it } from "vitest";
import { feed, toHiragana } from "./romaji";

describe("toHiragana", () => {
  it.each([
    ["watashiha", "わたしは"],
    ["kyouhaiitenkidesune.", "きょうはいいてんきですね。"],
    ["gakkou", "がっこう"],
    ["konnnichiha", "こんにちは"],
    ["kanni", "かんい"],
    ["hon'ya", "ほんや"],
    ["shinbun", "しんぶん"],
    ["tsukue", "つくえ"],
    ["chotto", "ちょっと"],
    ["fairu", "ふぁいる"],
    ["ra-men", "らーめん"],
    ["vaiorin", "ゔぁいおりん"],
    ["xtu", "っ"],
    ["nyanko", "にゃんこ"],
    ["2kai", "2かい"],
    ["WATASHI", "わたし"],
  ])("%s → %s", (input, expected) => {
    expect(toHiragana(input)).toBe(expected);
  });
});

describe("feed", () => {
  it("keeps partial spelling pending", () => {
    expect(feed("", "k")).toEqual({ kana: "", pending: "k" });
    expect(feed("ky", "o")).toEqual({ kana: "きょ", pending: "" });
    expect(feed("n", "k")).toEqual({ kana: "ん", pending: "k" });
  });
});
