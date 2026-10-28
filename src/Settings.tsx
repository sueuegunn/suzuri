import { useEffect, useRef, useState } from "react";
import { settings, type LetterCase } from "./engine";
import { label, record, type Bindings } from "./shortcut";

const MAC = /Mac/i.test(navigator.userAgent);

const ROWS: { kind: keyof Bindings; title: string }[] = [
  { kind: "new", title: "新しく入力する" },
  { kind: "previous", title: "前回の状態を直す" },
];

const CASES: { value: LetterCase; title: string }[] = [
  { value: "upper", title: "大文字" },
  { value: "lower", title: "小文字" },
];

/** トレイの「設定…」から開くウィンドウ。ショートカットを押して記録する */
export default function Settings() {
  const [bindings, setBindings] = useState<Bindings | null>(null);
  const [recording, setRecording] = useState<keyof Bindings | null>(null);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
  const [letterCase, setLetterCase] = useState<LetterCase | null>(null);
  const recordingRef = useRef(recording);
  recordingRef.current = recording;

  useEffect(() => {
    settings.get().then(setBindings);
    settings.getLetterCase().then(setLetterCase);
  }, []);

  const stop = () => {
    setRecording(null);
    settings.resume();
  };

  const start = (kind: keyof Bindings) => {
    setMessage(null);
    setRecording(kind);
    // 登録したままだと、今のショートカットを押したときにオーバーレイが開いてしまい記録できない
    settings.suspend();
  };

  const save = async (next: Bindings | undefined, done: string) => {
    try {
      setBindings(await settings.set(next));
      setMessage({ text: done, error: false });
    } catch (e) {
      setMessage({ text: String(e), error: true });
    }
  };

  const changeLetterCase = async (next: LetterCase) => {
    try {
      await settings.setLetterCase(next);
      setLetterCase(next);
    } catch (e) {
      setMessage({ text: String(e), error: true });
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const kind = recordingRef.current;
      if (!kind || !bindings) return;
      // ブラウザの機能（Ctrl+R の再読み込みなど）を動かさない
      e.preventDefault();
      if (e.code === "Escape" && !e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey) {
        stop();
        return;
      }
      const r = record(e);
      if (r.kind === "pending") return;
      if (r.kind === "needsModifier") {
        setMessage({ text: `${MAC ? "Control・Option・Cmd" : "Ctrl・Alt"} のどれかと一緒に押してください`, error: true });
        return;
      }
      stop();
      save({ ...bindings, [kind]: r.accelerator }, "変更しました");
    };
    // 記録中にウィンドウから離れたら、記録をやめてショートカットを戻す
    const onBlur = () => recordingRef.current && stop();
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onBlur);
    };
  }, [bindings]);

  return (
    <div className="settings">
      <h1>ショートカット</h1>
      {ROWS.map(({ kind, title }) => (
        <div className="settings-row" key={kind}>
          <span>{title}</span>
          <button
            className={"shortcut-field" + (recording === kind ? " recording" : "")}
            onClick={() => (recording === kind ? stop() : start(kind))}
          >
            {recording === kind ? "キーを押してください（Esc でやめる）" : bindings ? label(bindings[kind], MAC) : "…"}
          </button>
        </div>
      ))}
      <p className={"settings-message" + (message?.error ? " error" : "")}>{message?.text ?? ""}</p>
      <div className="settings-actions">
        <button onClick={() => save(undefined, "既定に戻しました")} disabled={recording !== null}>
          既定に戻す
        </button>
      </div>
      <h1>入力</h1>
      <div className="settings-row">
        <span>Shift + 英字</span>
        <div className="segmented">
          {CASES.map(({ value, title }) => (
            <button
              key={value}
              className={letterCase === value ? "selected" : ""}
              onClick={() => changeLetterCase(value)}
              disabled={recording !== null}
            >
              {title}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
