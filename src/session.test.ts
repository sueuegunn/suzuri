import { describe, expect, it } from "vitest";
import { carrySelections, clampPin, mergeRange, rangeAround, retypeRange, shiftBoundaries, splice } from "./session";

describe("shiftBoundaries", () => {
  it("adds text typed at a boundary to the left segment", () => {
    expect(shiftBoundaries([4, 7], 4, 0, 2, 12)).toEqual([6, 9]);
  });
  it("collapses a segment emptied by deletion", () => {
    // 文節 [4,5) の1文字を消す
    expect(shiftBoundaries([4, 5, 8], 4, 1, 0, 9)).toEqual([4, 7]);
  });
  it("drops boundaries at the ends", () => {
    expect(shiftBoundaries([1], 0, 1, 0, 3)).toEqual([]);
  });
});

describe("rangeAround", () => {
  it("covers the edited segment", () => {
    expect(rangeAround([4, 9], 5, 6, 12)).toEqual([4, 9]);
  });
  it("covers both sides when deleting at a boundary", () => {
    expect(rangeAround([4, 9], 4, 4, 12)).toEqual([0, 9]);
  });
});

describe("retypeRange", () => {
  it("also covers the previous segment", () => {
    // 「くるま|で」の末尾に打ったときは「くるまで…」をまとめて区切り直す
    expect(retypeRange([3], [], 4, 5, 5)).toEqual([0, 5]);
    expect(retypeRange([20, 30], [], 31, 32, 32)).toEqual([20, 32]);
  });
  it("covers segments starting within the window", () => {
    // 「アニメ|ー|しょ」の末尾に「ん」を打ったときは、先頭からまとめて区切り直す
    expect(retypeRange([3, 4], [], 6, 7, 7)).toEqual([0, 7]);
    expect(retypeRange([5, 22, 25, 27], [], 30, 31, 31)).toEqual([22, 31]);
  });
  it("does not cross a pinned boundary or a chosen segment", () => {
    expect(retypeRange([4, 9], [9], 10, 11, 11)).toEqual([9, 11]);
    expect(retypeRange([3, 4], [], 6, 7, 7, [3, 4])).toEqual([4, 7]);
  });
});

describe("carrySelections", () => {
  const seg = (start: number, end: number, reading: string, candidates: string[], selected = 0) => ({
    start,
    end,
    reading,
    candidates,
    selected,
  });
  it("keeps the chosen surface when range and reading are unchanged", () => {
    const prev = [seg(0, 2, "はし", ["橋", "箸"], 1), seg(2, 3, "を", ["を"])];
    const next = [
      { start: 0, end: 2, reading: "はし", candidates: ["橋", "端", "箸"] },
      { start: 2, end: 4, reading: "をわ", candidates: ["をわ"] },
    ];
    expect(carrySelections(prev, next).map((s) => s.selected)).toEqual([2, 0]);
  });
});

describe("clampPin", () => {
  it("stays between neighbouring pins", () => {
    expect(clampPin([3, 6, 9], 1, 1, 12)).toBe(4);
    expect(clampPin([3, 6, 9], 1, 20, 12)).toBe(8);
    expect(clampPin([3, 4], 0, 9, 12)).toBe(3);
  });
});

it("splices by character", () => {
  expect(splice("わたし", 1, 1, "ゔぁ")).toBe("わゔぁし");
});

describe("mergeRange", () => {
  // わたしは|はしを|わたる（0..4, 4..7, 7..10）
  const bs = [4, 7];

  it("文節の途中から途中までを1文節にし、両側の残りを外隣とあわせて区切り直す", () => {
    // 「しははし」（2..6）を選ぶ。左の「わた」は先頭まで、右の「を」は「わたる」とあわせて区切り直す
    expect(mergeRange(bs, [], 2, 6, 10)).toEqual({ boundaries: [2, 6, 7], pinned: [2, 6], ranges: [[0, 2], [6, 10]] });
  });

  it("逆向きに選んでも同じ", () => {
    expect(mergeRange(bs, [], 6, 2, 10)).toEqual(mergeRange(bs, [], 2, 6, 10));
  });

  it("両端が区切りちょうどなら、ほかの文節は区切り直さない", () => {
    expect(mergeRange(bs, [], 0, 7, 10)).toEqual({ boundaries: [7], pinned: [7], ranges: [] });
  });

  it("手で決めた区切りは越えずに区切り直す", () => {
    expect(mergeRange(bs, [4], 5, 6, 10)?.ranges).toEqual([[4, 5], [6, 10]]);
    expect(mergeRange(bs, [7], 5, 6, 10)?.ranges).toEqual([[0, 5], [6, 7]]);
  });

  it("内側の手で決めた区切りは外す", () => {
    expect(mergeRange(bs, [4], 2, 8, 10)?.pinned).toEqual([2, 8]);
  });

  it("空の範囲なら null", () => {
    expect(mergeRange(bs, [], 3, 3, 10)).toBeNull();
  });
});
