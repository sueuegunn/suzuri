// Rust 側のコマンドの呼び出し口

import { invoke } from "@tauri-apps/api/core";
import type { EngineSegment } from "./session";
import type { Bindings } from "./shortcut";

/** Shift を押しながら打った英字を、大文字と小文字のどちらで入れるか */
export type LetterCase = "upper" | "lower";

/** オーバーレイの見た目。どちらも百分率 */
export type Appearance = { textScale: number; opacity: number };

export type Committed = { reading: string; surface: string; wasFirst: boolean };

export const engine = {
  convert: (reading: string) => invoke<EngineSegment[]>("convert", { reading }),
  convertWithBoundaries: (reading: string, boundaries: number[]) =>
    invoke<EngineSegment[]>("convert_with_boundaries", { reading, boundaries }),
  resegment: (reading: string, boundaries: number[], start: number, end: number) =>
    invoke<EngineSegment[]>("resegment", { reading, boundaries, start, end }),
  commit: (text: string, segments: Committed[], paste: boolean) =>
    invoke<void>("commit", { text, segments, paste }),
  cancel: () => invoke<void>("cancel"),
};

export const settings = {
  get: () => invoke<Bindings>("get_shortcuts"),
  /** 省くと既定に戻す。登録できなければ理由の文字列で失敗する */
  set: (bindings?: Bindings) => invoke<Bindings>("set_shortcuts", { bindings: bindings ?? null }),
  suspend: () => invoke<void>("suspend_shortcuts"),
  resume: () => invoke<void>("resume_shortcuts"),
  getLetterCase: () => invoke<LetterCase>("get_letter_case"),
  /** 変えるとオーバーレイに "letter-case" イベントで知らせる */
  setLetterCase: (letterCase: LetterCase) => invoke<void>("set_letter_case", { letterCase }),
  getAppearance: () => invoke<Appearance>("get_appearance"),
  /** 範囲に収めた値を返す。変えるとオーバーレイに "appearance" イベントで知らせる */
  setAppearance: (appearance: Appearance) => invoke<Appearance>("set_appearance", { appearance }),
  /** 覚えたオーバーレイの位置と大きさを忘れる */
  resetWindowGeometry: () => invoke<void>("reset_window_geometry"),
};

/** 見た目の設定を CSS の変数に反映する */
export function applyAppearance(a: Appearance) {
  const root = document.documentElement.style;
  root.setProperty("--text-scale", String(a.textScale / 100));
  root.setProperty("--bg-alpha", String(a.opacity / 100));
}
