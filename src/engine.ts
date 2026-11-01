// Rust 側のコマンドの呼び出し口

import { invoke } from "@tauri-apps/api/core";
import type { EngineSegment } from "./session";
import type { Bindings } from "./shortcut";

/** 画面の配色。system は OS の設定に合わせる */
export type Theme = "system" | "light" | "dark";

/** オーバーレイの見た目。どちらも百分率 */
export type Appearance = { textScale: number; opacity: number };

export type Committed = { reading: string; surface: string; wasFirst: boolean };

export const engine = {
  convert: (reading: string) => invoke<EngineSegment[]>("convert", { reading }),
  convertWithBoundaries: (reading: string, boundaries: number[]) =>
    invoke<EngineSegment[]>("convert_with_boundaries", { reading, boundaries }),
  resegment: (reading: string, boundaries: number[], start: number, end: number) =>
    invoke<EngineSegment[]>("resegment", { reading, boundaries, start, end }),
  /** ranges のそれぞれ [start, end) の内側だけ自動で区切り直す */
  resegmentRanges: (reading: string, boundaries: number[], ranges: [number, number][]) =>
    invoke<EngineSegment[]>("resegment_ranges", { reading, boundaries, ranges }),
  commit: (text: string, segments: Committed[], paste: boolean) =>
    invoke<void>("commit", { text, segments, paste }),
  cancel: () => invoke<void>("cancel"),
  /** オーバーレイを今いるモニターの中央に戻す */
  center: () => invoke<void>("center_overlay"),
  /** 呼び出し元の OS の IME を、日本語入力（true）か英字入力（false）にする */
  setImeJapanese: (japanese: boolean) => invoke<void>("set_ime_japanese", { japanese }),
};

export const settings = {
  get: () => invoke<Bindings>("get_shortcuts"),
  /** 省くと既定に戻す。登録できなければ理由の文字列で失敗する */
  set: (bindings?: Bindings) => invoke<Bindings>("set_shortcuts", { bindings: bindings ?? null }),
  suspend: () => invoke<void>("suspend_shortcuts"),
  resume: () => invoke<void>("resume_shortcuts"),
  getTheme: () => invoke<Theme>("get_theme"),
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
