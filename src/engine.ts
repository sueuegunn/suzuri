// Rust 側のコマンドの呼び出し口

import { invoke } from "@tauri-apps/api/core";
import type { EngineSegment } from "./session";
import type { Bindings } from "./shortcut";

/** 画面の配色。system は OS の設定に合わせる */
export type Theme = "system" | "light" | "dark";

/** オーバーレイの見た目。どちらも百分率 */
export type Appearance = { textScale: number; opacity: number };

/** 確定した文節。manual は手で置いたピンに接するか（区切りの学習の対象） */
export type Committed = { reading: string; surface: string; wasFirst: boolean; manual: boolean };

/** ユーザー辞書の品詞。名前は Rust 側の UserPos と合わせる */
export type UserPos = "noun" | "person" | "place" | "organization";

export type UserWord = { reading: string; surface: string; pos: UserPos };

/** 続きの候補。読みの末尾の matched 文字が、語の読み reading の先頭と一致した */
export type Suggestion = { matched: number; reading: string; surface: string };

/** ユーザー辞書の読み書き。追加と削除は、変えたあとの一覧（登録した順）を返す */
export const userDictionary = {
  list: () => invoke<UserWord[]>("list_user_words"),
  /** 読みや表記が正しくなければ、画面に出す理由で失敗する */
  add: (word: UserWord) => invoke<UserWord[]>("add_user_word", { word }),
  /** old を word に書き換える。正しくなければ、画面に出す理由で失敗する */
  update: (old: UserWord, word: UserWord) => invoke<UserWord[]>("update_user_word", { old, word }),
  remove: (word: UserWord) => invoke<UserWord[]>("remove_user_word", { word }),
  /** オーバーレイの下段の文節を名詞として登録する。末尾の助詞は除く。登録した語を返す */
  registerSegment: (reading: string, surface: string) => invoke<UserWord>("register_segment", { reading, surface }),
};

export const engine = {
  convert: (reading: string) => invoke<EngineSegment[]>("convert", { reading }),
  /** 読みの末尾から、ユーザー辞書の語や学習した語の続きを1つ探す */
  suggest: (reading: string) => invoke<Suggestion | null>("suggest", { reading }),
  convertWithBoundaries: (reading: string, boundaries: number[]) =>
    invoke<EngineSegment[]>("convert_with_boundaries", { reading, boundaries }),
  resegment: (reading: string, boundaries: number[], start: number, end: number) =>
    invoke<EngineSegment[]>("resegment", { reading, boundaries, start, end }),
  /** ranges のそれぞれ [start, end) の内側だけ自動で区切り直す */
  resegmentRanges: (reading: string, boundaries: number[], ranges: [number, number][]) =>
    invoke<EngineSegment[]>("resegment_ranges", { reading, boundaries, ranges }),
  commit: (text: string, segments: Committed[], paste: boolean) =>
    invoke<void>("commit", { text, segments, paste }),
  /** 閉じる。returnFocus なら呼び出し元にフォーカスを戻す（Esc のとき。枠外クリックではクリックした先に任せる） */
  cancel: (returnFocus: boolean) => invoke<void>("cancel", { returnFocus }),
  /** 調べるためのログを Rust の標準エラーに出す（tauri dev のターミナルに出る） */
  debugLog: (message: string) => invoke<void>("debug_log", { message }).catch(() => {}),
  /** オーバーレイを今いるモニターの中央に戻す */
  center: () => invoke<void>("center_overlay"),
  /** クリップボードの文字。文字でなければ空 */
  readClipboard: () => invoke<string>("read_clipboard"),
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
  /** 貼り付けモードで貼り付けたあとも、クリップボードに残すか */
  getKeepPastedInClipboard: () => invoke<boolean>("get_keep_pasted_in_clipboard"),
  setKeepPastedInClipboard: (keep: boolean) => invoke<void>("set_keep_pasted_in_clipboard", { keep }),
};

/** 見た目の設定を CSS の変数に反映する */
export function applyAppearance(a: Appearance) {
  const root = document.documentElement.style;
  root.setProperty("--text-scale", String(a.textScale / 100));
  root.setProperty("--bg-alpha", String(a.opacity / 100));
}
