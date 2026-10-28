// Rust 側のコマンドの呼び出し口

import { invoke } from "@tauri-apps/api/core";
import type { EngineSegment } from "./session";
import type { Bindings } from "./shortcut";

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
};
