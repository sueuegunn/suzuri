// Rust 側のコマンドの呼び出し口

import { invoke } from "@tauri-apps/api/core";
import type { EngineSegment } from "./session";

export type Committed = { reading: string; surface: string; wasFirst: boolean };

export const engine = {
  convert: (reading: string) => invoke<EngineSegment[]>("convert", { reading }),
  convertWithBoundaries: (reading: string, boundaries: number[]) =>
    invoke<EngineSegment[]>("convert_with_boundaries", { reading, boundaries }),
  resegment: (reading: string, boundaries: number[], start: number, end: number) =>
    invoke<EngineSegment[]>("resegment", { reading, boundaries, start, end }),
  commit: (text: string, segments: Committed[]) => invoke<void>("commit", { text, segments }),
  cancel: () => invoke<void>("cancel"),
};
