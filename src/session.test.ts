import { describe, expect, it } from "vitest";
import { carrySelections, clampPin, rangeAround, shiftBoundaries, splice } from "./session";

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
