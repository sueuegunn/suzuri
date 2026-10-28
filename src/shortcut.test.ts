import { describe, expect, it } from "vitest";
import { label, record } from "./shortcut";

const key = (code: string, mods: Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>> = {}) => ({
  code,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

describe("record", () => {
  it("builds an accelerator from modifiers and the key code", () => {
    expect(record(key("KeyJ", { altKey: true, shiftKey: true }))).toEqual({
      kind: "ok",
      accelerator: "Alt+Shift+KeyJ",
    });
    expect(record(key("Semicolon", { ctrlKey: true }))).toEqual({ kind: "ok", accelerator: "Ctrl+Semicolon" });
  });
  it("waits while only modifiers are pressed", () => {
    expect(record(key("AltLeft", { altKey: true }))).toEqual({ kind: "pending" });
  });
  it("rejects keys without Ctrl, Alt or Cmd", () => {
    expect(record(key("KeyJ"))).toEqual({ kind: "needsModifier" });
    expect(record(key("KeyJ", { shiftKey: true }))).toEqual({ kind: "needsModifier" });
  });
});

describe("label", () => {
  it("names keys for each OS", () => {
    expect(label("Alt+Shift+KeyJ", false)).toBe("Alt + Shift + J");
    expect(label("Alt+Shift+KeyJ", true)).toBe("Option + Shift + J");
    expect(label("Super+Semicolon", true)).toBe("Cmd + ;");
    expect(label("Ctrl+Digit1", false)).toBe("Ctrl + 1");
  });
});
