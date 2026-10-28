// ショートカットの組み合わせと、キー入力・表示の相互変換。
// 組み合わせは Rust 側と同じ「修飾キー+…+キーのコード」の文字列（例: `Alt+Shift+KeyJ`）で持つ。

export type Bindings = { new: string; previous: string };

type KeyInput = { code: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean };

export type Recorded =
  /** 修飾キーだけが押されていて、まだ決まらない */
  | { kind: "pending" }
  /** Ctrl・Alt・Cmd のどれも押されていない */
  | { kind: "needsModifier" }
  | { kind: "ok"; accelerator: string };

const MODIFIER_CODES = /^(Control|Alt|Shift|Meta|OS)(Left|Right)?$/;

/** 押されたキーから組み合わせを作る */
export function record(e: KeyInput): Recorded {
  if (MODIFIER_CODES.test(e.code)) return { kind: "pending" };
  // Shift だけでは、普段の打鍵（大文字や記号）を奪ってしまう
  if (!e.ctrlKey && !e.altKey && !e.metaKey) return { kind: "needsModifier" };
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);
  return { kind: "ok", accelerator: [...mods, e.code].join("+") };
}

const KEY_NAMES: Record<string, string> = {
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backslash: "\\",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backquote: "`",
};

/** 画面に出す名前。macOS では Alt を Option、Super を Cmd と書く */
export function label(accelerator: string, mac: boolean): string {
  return accelerator
    .split("+")
    .map((part) => {
      if (part === "Alt") return mac ? "Option" : "Alt";
      if (part === "Super") return mac ? "Cmd" : "Win";
      if (part === "Ctrl" || part === "Shift") return part;
      return KEY_NAMES[part] ?? part.replace(/^(Key|Digit)/, "");
    })
    .join(" + ");
}
