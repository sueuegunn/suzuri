import { Fragment, useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalPosition, LogicalSize } from "@tauri-apps/api/dpi";
import { applyAppearance, engine, settings, type Appearance, type LetterCase } from "./engine";
import { feed, flush } from "./romaji";
import {
  boundariesOf,
  carrySelections,
  clampPin,
  emptySession,
  length,
  retypeRange,
  resultText,
  shiftBoundaries,
  shiftPosition,
  splice,
  type EngineSegment,
  type Segment,
  type Session,
} from "./session";

/**
 * ホイールの何ピクセル分で候補を1つ送るか（トラックパッドの細かい量をまとめる）。
 * これ以上の量が1回で来たら、マウスのホイールの1目盛りとみなして1つだけ送る
 */
const WHEEL_STEP = 50;
/** これだけ止まっていたら、まとめかけのホイールの量を捨てる */
const WHEEL_IDLE_MS = 300;
/** 行単位のホイールで Ctrl+スクロールするときの1行の高さ */
const WHEEL_LINE_PX = 40;
/** 候補一覧に並べる数 */
const CANDIDATE_LIST_SIZE = 9;

/** Enter で何をするか。copy はクリップボードに入れるだけ、paste は呼び出し元に貼り付けまでする */
type Mode = "copy" | "paste";
const MODE_KEY = "suzuri.mode";

/** 前回使ったモード。初めて起動したときや読めないときは貼り付けのモード */
function loadMode(): Mode {
  try {
    return localStorage.getItem(MODE_KEY) === "copy" ? "copy" : "paste";
  } catch {
    return "paste";
  }
}

function saveMode(mode: Mode) {
  try {
    localStorage.setItem(MODE_KEY, mode);
  } catch {
    // 覚えられなくても、その回の切り替えは効く
  }
}

type Drag = { pin: number; target: number; x: number; y: number; height: number };

/** 未確定のローマ字をカーソル位置に確定させる */
function settle(s: Session): Session {
  const text = flush(s.pending);
  if (text === "") return s;
  return { ...s, kana: splice(s.kana, s.caret, 0, text), caret: s.caret + length(text), pending: "" };
}

export default function App() {
  const [session, setSession] = useState<Session>(emptySession);
  const sessionRef = useRef(session);
  const previousRef = useRef<Session>(emptySession);
  const requestRef = useRef(0);
  const [listFor, setListFor] = useState<number | null>(null);
  const wheel = useRef({ acc: 0, index: -1, at: 0 });
  const [drag, setDrag] = useState<Drag | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  /** 動かす掴みどころにマウスが乗っているか。乗っている間は、左下の案内をその使い方に替える */
  const [grabberHover, setGrabberHover] = useState(false);
  const [mode, setMode] = useState<Mode>(loadMode);
  const modeRef = useRef(mode);
  const panelRef = useRef<HTMLDivElement>(null);
  /** 上段（読み）の領域。ピンの位置の計算の基準 */
  const bodyRef = useRef<HTMLDivElement>(null);
  /** 下段の各文節の要素。候補一覧の位置合わせに使う */
  const resultRefs = useRef<(HTMLSpanElement | null)[]>([]);

  const update = (s: Session) => {
    sessionRef.current = s;
    setSession(s);
  };

  /** エンジンを呼び、結果が届いた時点で読みが変わっていなければ反映する */
  const run = async (
    reading: string,
    previous: Segment[] | null,
    call: Promise<EngineSegment[]>,
    shift?: (pos: number) => number,
  ) => {
    const id = ++requestRef.current;
    const segments = await call;
    const cur = sessionRef.current;
    if (id !== requestRef.current || cur.kana !== reading) return;
    update({ ...cur, segments: carrySelections(previous, segments, shift) });
  };

  const close = () => {
    previousRef.current = settle(sessionRef.current);
    setListFor(null);
    setDrag(null);
  };

  const cancel = () => {
    close();
    engine.cancel();
  };

  const commit = () => {
    const s = settle(sessionRef.current);
    const text = resultText(s);
    close();
    if (text === "") {
      engine.cancel();
      return;
    }
    const segments = (s.segments ?? []).map((seg) => ({
      reading: seg.reading,
      surface: seg.candidates[seg.selected],
      wasFirst: seg.selected === 0,
    }));
    engine.commit(text, segments, modeRef.current === "paste");
  };

  /**
   * ひらがなを直したら、打つそばから変換する（ライブ変換）。
   * 変換済みなら直した文節だけ区切り直し、ほかの文節の区切りと選んだ候補は保つ
   */
  const edit = (old: Session, next: Session, pos: number, removed: number, inserted: number) => {
    if (next.kana === "") {
      update({ ...next, segments: null, pinned: [] });
      return;
    }
    if (!old.segments) {
      next = { ...next, pinned: [] };
      update(next);
      run(next.kana, null, engine.convert(next.kana));
      return;
    }
    const n = length(next.kana);
    const boundaries = shiftBoundaries(boundariesOf(old.segments), pos, removed, inserted, n);
    const pinned = shiftBoundaries(old.pinned, pos, removed, inserted, n);
    next = { ...next, pinned };
    update(next);
    const [start, end] = retypeRange(boundaries, pinned, pos, pos + inserted, n);
    run(next.kana, old.segments, engine.resegment(next.kana, boundaries, start, end), (x) =>
      shiftPosition(x, pos, removed, inserted),
    );
  };

  const type = (ch: string) => {
    const s = sessionRef.current;
    const r = feed(s.pending, ch, Array.from(s.kana)[s.caret - 1]);
    if (r.kana === "") {
      update({ ...s, pending: r.pending });
      return;
    }
    const inserted = length(r.kana);
    const next = { ...s, kana: splice(s.kana, s.caret, 0, r.kana), caret: s.caret + inserted, pending: r.pending };
    edit(s, next, s.caret, 0, inserted);
  };

  const toggleMode = () => {
    const next: Mode = modeRef.current === "copy" ? "paste" : "copy";
    modeRef.current = next;
    setMode(next);
    saveMode(next);
  };

  /**
   * Shift を押しながら打った英字。ローマ字にせず、そのまま入れる。
   * 空文字なら、打ちかけのローマ字（末尾の n など）を確定させるだけ
   */
  const typeLetter = (ch: string) => {
    const s = sessionRef.current;
    // 打ちかけのローマ字は先に確定させる
    const text = flush(s.pending) + ch;
    if (text === "") return;
    const inserted = length(text);
    const next = { ...s, kana: splice(s.kana, s.caret, 0, text), caret: s.caret + inserted, pending: "" };
    edit(s, next, s.caret, 0, inserted);
  };

  const backspace = () => {
    const s = sessionRef.current;
    if (s.pending !== "") {
      update({ ...s, pending: s.pending.slice(0, -1) });
      return;
    }
    if (s.caret === 0) return;
    const next = { ...s, kana: splice(s.kana, s.caret - 1, 1, ""), caret: s.caret - 1 };
    edit(s, next, s.caret - 1, 1, 0);
  };

  const moveCaret = (to: (s: Session) => number) => {
    const s = settle(sessionRef.current);
    update({ ...s, caret: Math.max(0, Math.min(length(s.kana), to(s))) });
  };

  /** 手で区切りを変えたとき。`pinned` は手で決めた区切りの新しい一覧 */
  const setBoundaries = (boundaries: number[], pinned: number[]) => {
    const s = sessionRef.current;
    if (!s.segments) return;
    update({ ...s, pinned });
    run(s.kana, s.segments, engine.convertWithBoundaries(s.kana, boundaries));
  };

  const addPin = (gap: number) => {
    const s = sessionRef.current;
    if (!s.segments) return;
    const bs = boundariesOf(s.segments);
    if (gap <= 0 || gap >= length(s.kana) || bs.includes(gap)) return;
    setBoundaries(
      [...bs, gap].sort((a, b) => a - b),
      [...s.pinned, gap],
    );
  };

  const removePin = (index: number) => {
    const s = sessionRef.current;
    if (!s.segments) return;
    const bs = boundariesOf(s.segments);
    setBoundaries(
      bs.filter((_, i) => i !== index),
      s.pinned.filter((p) => p !== bs[index]),
    );
  };

  const cycle = (index: number, step: number) => {
    const s = sessionRef.current;
    if (!s.segments) return;
    const segments = s.segments.map((seg, i) => {
      if (i !== index) return seg;
      const n = seg.candidates.length;
      return { ...seg, selected: (seg.selected + step + n) % n };
    });
    update({ ...s, segments });
    // 一覧は文節からマウスが離れるまで出しておく
    setListFor(index);
  };

  const onWheel = (index: number, e: React.WheelEvent) => {
    // Ctrl+ホイールはスクロールに使う
    if (e.ctrlKey) return;
    // 行単位・ページ単位のホイールは1回で1候補、ピクセル単位は量をまとめて送る
    const w = wheel.current;
    const now = performance.now();
    // 前の操作の残りは捨てる。残すと、逆に回したときに打ち消し合って候補が動かないことがある
    if (index !== w.index || now - w.at > WHEEL_IDLE_MS || Math.sign(e.deltaY) !== Math.sign(w.acc)) w.acc = 0;
    w.index = index;
    w.at = now;
    // 目盛りのあるホイールは、1目盛りの量が画面の拡大率で変わるため、1回で1つだけ送る
    if (e.deltaMode !== 0 || Math.abs(e.deltaY) >= WHEEL_STEP) {
      w.acc = 0;
      if (e.deltaY !== 0) cycle(index, Math.sign(e.deltaY));
      return;
    }
    w.acc += e.deltaY;
    if (Math.abs(w.acc) >= WHEEL_STEP) {
      cycle(index, Math.sign(w.acc));
      w.acc = 0;
    }
  };

  /** 文字と文字の間の位置（隙間の番号 → 画面上の座標） */
  const gapPositions = () => {
    const body = bodyRef.current!;
    const origin = body.getBoundingClientRect();
    const chars = [...body.querySelectorAll<HTMLElement>("[data-pos]")]
      .sort((a, b) => Number(a.dataset.pos) - Number(b.dataset.pos))
      .map((el) => el.getBoundingClientRect());
    const gaps = new Map<number, { x: number; y: number; height: number }>();
    for (let g = 1; g < chars.length; g++) {
      const [prev, cur] = [chars[g - 1], chars[g]];
      const sameRow = Math.abs(prev.top - cur.top) < cur.height / 2;
      const x = sameRow ? (prev.right + cur.left) / 2 : cur.left;
      // ゴーストのピンを .body の中に置くので、.body の左上を原点にする
      gaps.set(g, { x: x - origin.left, y: cur.top - origin.top + body.scrollTop, height: cur.height });
    }
    return gaps;
  };

  const nearestGap = (clientX: number, clientY: number) => {
    const body = bodyRef.current!;
    const origin = body.getBoundingClientRect();
    const [px, py] = [clientX - origin.left, clientY - origin.top + body.scrollTop];
    let best = -1;
    let bestDist = Infinity;
    for (const [g, p] of gapPositions()) {
      const d = Math.hypot(p.x - px, (p.y + p.height / 2 - py) * 2);
      if (d < bestDist) [best, bestDist] = [g, d];
    }
    return best;
  };

  const onPinMove = (pin: number, e: React.PointerEvent) => {
    if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
    const s = sessionRef.current;
    if (!s.segments) return;
    const target = clampPin(boundariesOf(s.segments), pin, nearestGap(e.clientX, e.clientY), length(s.kana));
    if (target === null) return;
    const p = gapPositions().get(target);
    if (p) setDrag({ pin, target, ...p });
  };

  const onPinUp = (pin: number) => {
    const s = sessionRef.current;
    if (drag && drag.pin === pin && s.segments) {
      const bs = boundariesOf(s.segments);
      if (bs[pin] !== drag.target) {
        const pinned = [...s.pinned.filter((p) => p !== bs[pin]), drag.target];
        bs[pin] = drag.target;
        setBoundaries(bs, pinned);
      }
    }
    setDrag(null);
  };

  /** 文字の左半分なら左の隙間、右半分なら右の隙間 */
  const gapAt = (pos: number, e: React.MouseEvent) => {
    const r = e.currentTarget.getBoundingClientRect();
    return e.clientX < r.left + r.width / 2 ? pos : pos + 1;
  };

  // 打鍵ごとに問い合わせないよう、起動時に読んでおき、設定画面で変えたらイベントで受け取る
  const letterCaseRef = useRef<LetterCase>("upper");
  useEffect(() => {
    settings.getLetterCase().then((c) => (letterCaseRef.current = c));
    settings.getAppearance().then(applyAppearance);
    const unlisten = [
      listen<LetterCase>("letter-case", (e) => (letterCaseRef.current = e.payload)),
      listen<Appearance>("appearance", (e) => applyAppearance(e.payload)),
    ];
    return () => {
      unlisten.forEach((u) => u.then((f) => f()));
    };
  }, []);

  const handlers = useRef({ type, typeLetter, backspace, commit, cancel, moveCaret, toggleMode });
  handlers.current = { type, typeLetter, backspace, commit, cancel, moveCaret, toggleMode };

  useEffect(() => {
    const unlisten = listen<"new" | "previous">("summon", (e) => {
      update(e.payload === "previous" ? previousRef.current : emptySession);
      setListFor(null);
      setDrag(null);
      setHover(null);
      window.focus();
    });
    const onKey = (e: KeyboardEvent) => {
      // OS の IME が動いているときの打鍵は扱わない
      if (e.isComposing || e.key === "Process" || e.ctrlKey || e.metaKey || e.altKey) return;
      const h = handlers.current;
      switch (e.key) {
        case "Escape":
          h.cancel();
          break;
        case "Enter":
          h.commit();
          break;
        case "Tab":
          h.toggleMode();
          break;
        case " ":
          // 打つそばから変換していて Space で変換することはないので、Space は半角スペースを入れる。
          // 全角はホイールで選ぶ
          h.typeLetter(" ");
          break;
        case "Backspace":
          h.backspace();
          break;
        case "ArrowLeft":
          h.moveCaret((s) => s.caret - 1);
          break;
        case "ArrowRight":
          h.moveCaret((s) => s.caret + 1);
          break;
        case "Home":
          h.moveCaret(() => 0);
          break;
        case "End":
          h.moveCaret((s) => length(s.kana));
          break;
        default:
          if (e.key.length !== 1) return;
          // CapsLock の状態によらず、Shift を押しているかで英字かローマ字かを決める
          if (e.shiftKey && /^[a-z]$/i.test(e.key))
            h.typeLetter(letterCaseRef.current === "lower" ? e.key.toLowerCase() : e.key.toUpperCase());
          else h.type(e.key);
      }
      e.preventDefault();
    };
    // 枠外をクリックしたら閉じる。内容は「前回」として残る。
    // ウィンドウを動かしている間・大きさを変えている間も OS の処理に入って画面からフォーカスが外れるが、
    // そのときはウィンドウ自体は前面のままなので閉じない。縁の数 px は OS が直接大きさ変更を始めるため、
    // 画面側では操作が始まったことを知れない。OS にウィンドウが前面かを問い合わせて見分ける
    const onBlur = () => {
      getCurrentWindow()
        .isFocused()
        .then((focused) => {
          if (!focused) handlers.current.cancel();
        })
        .catch(() => handlers.current.cancel());
    };
    const noMenu = (e: MouseEvent) => e.preventDefault();
    // ホイールは候補の切り替えに使い、スクロールは Ctrl+ホイールかスクロールバーで行う。
    // React の onWheel は passive で止められないため、ここで止める。Ctrl+ホイールの拡大縮小も止まる
    const noWheelScroll = (e: WheelEvent) => {
      e.preventDefault();
      if (!e.ctrlKey || !(e.target instanceof Element)) return;
      // 上段・下段のうち、マウスがある方をスクロールする
      const area = e.target.closest<HTMLElement>(".area");
      if (!area) return;
      const unit = e.deltaMode === 1 ? WHEEL_LINE_PX : e.deltaMode === 2 ? area.clientHeight : 1;
      area.scrollTop += e.deltaY * unit;
    };
    const panel = panelRef.current!;
    panel.addEventListener("wheel", noWheelScroll, { passive: false });
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onBlur);
    window.addEventListener("contextmenu", noMenu);
    return () => {
      unlisten.then((f) => f());
      panel.removeEventListener("wheel", noWheelScroll);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("contextmenu", noMenu);
    };
  }, []);

  const caretMark = (pos: number) =>
    session.caret === pos && (
      <span className="caret-wrap" key={`caret-${pos}`}>
        {session.pending && <span className="pending">{session.pending}</span>}
        <span className="caret" />
      </span>
    );

  const chars = (text: string, offset: number, isLast: boolean) => (
    <>
      {Array.from(text).map((c, j) => {
        const pos = offset + j;
        return (
          <Fragment key={pos}>
            {caretMark(pos)}
            <span
              className="char"
              data-pos={pos}
              onClick={(e) => moveCaret(() => gapAt(pos, e))}
              onContextMenu={(e) => addPin(gapAt(pos, e))}
            >
              {c}
            </span>
          </Fragment>
        );
      })}
      {isLast && caretMark(offset + length(text))}
    </>
  );

  /** 上段と下段で同じ文節を同じように強調する。下線の色は文節ごとに交互にし、上下の対応を見せる */
  const segClass = (i: number) =>
    "seg" + (i % 2 ? " odd" : "") + (listFor === i ? " active" : hover === i ? " hover" : "");

  /** 上段・下段の文節に共通のマウス操作 */
  const segHandlers = (i: number) => ({
    onWheel: (e: React.WheelEvent) => onWheel(i, e),
    onMouseEnter: () => setHover(i),
    onMouseLeave: () => {
      setHover(null);
      if (listFor === i) setListFor(null);
    },
  });

  const segments = session.segments;
  return (
    <div className="frame">
      <ResizeHandles />
      {/* 上の縁の掴みどころより手前に出すため、パネルの外に置く */}
      <div
        className="grabber"
        onPointerDown={onGrabberDown}
        onPointerEnter={() => setGrabberHover(true)}
        onPointerLeave={() => setGrabberHover(false)}
      />
      <div className="panel" ref={panelRef} onPointerDown={startMove}>
        <div className="area reading-area" ref={bodyRef}>
          <div className="reading">
            {segments
              ? segments.map((seg, i) => (
                  <Fragment key={`${seg.start}:${seg.end}`}>
                    {i > 0 && (
                      <span
                        className={"pin" + (drag?.pin === i - 1 ? " dragging" : "")}
                        title="ドラッグで区切りを移動、右クリックで削除"
                        onPointerDown={(e) => e.button === 0 && e.currentTarget.setPointerCapture(e.pointerId)}
                        onPointerMove={(e) => onPinMove(i - 1, e)}
                        onPointerUp={() => onPinUp(i - 1)}
                        onContextMenu={() => removePin(i - 1)}
                      />
                    )}
                    <span className={segClass(i)} {...segHandlers(i)}>
                      {chars(seg.reading, seg.start, i === segments.length - 1)}
                    </span>
                  </Fragment>
                ))
              : chars(session.kana, 0, true)}
          </div>
          {drag && <div className="ghost-pin" style={{ left: drag.x, top: drag.y, height: drag.height }} />}
        </div>
        <div className="area result-area">
          {segments?.map((seg, i) => (
            <span
              key={`${seg.start}:${seg.end}`}
              className={segClass(i)}
              ref={(el) => {
                resultRefs.current[i] = el;
              }}
              {...segHandlers(i)}
            >
              {seg.candidates[seg.selected]}
            </span>
          ))}
        </div>
        {segments && listFor !== null && segments[listFor] && (
          <CandidateList segment={segments[listFor]} anchor={() => resultRefs.current[listFor]} />
        )}
        <div className="footer">
          <div className="hint">
            {grabberHover ? (
              <>
                <MouseIcon part="left" drag /> ドラッグで移動{SEP}
                <MouseIcon part="left" /> ダブルクリックで中央に戻す
              </>
            ) : segments ? (
              <>
                <MouseIcon part="wheel" /> 候補{SEP}
                <MouseIcon part="left" drag /> 文節の区切りを移動{SEP}
                <MouseIcon part="right" /> ピン追加・削除{SEP}
                Ctrl+
                <MouseIcon part="wheel" /> スクロール
              </>
            ) : (
              "Esc: 閉じる"
            )}
          </div>
          <div className={"mode " + mode}>
            <EnterIcon /> {mode === "paste" ? "貼り付け" : "コピー"}
            <span className="mode-key">Tab で切り替え</span>
          </div>
        </div>
      </div>
    </div>
  );
}

/** 案内の項目の区切り */
const SEP = "   /   ";

/** マウスの絵。使うボタンかホイールを色付けする。
 * 操作に使うボタンとホイールを大きく見せるため、下の胴の部分は短くしている。
 * `drag` なら左右に矢印を付け、押したまま横に動かすことを表す */
function MouseIcon({ part, drag = false }: { part: "left" | "right" | "wheel"; drag?: boolean }) {
  // 矢印を付けるときは、マウスを右にずらして左右に場所をあける
  const x = drag ? 8 : 0;
  return (
    <svg className="mouse" viewBox={`0 0 ${12 + x * 2} 15`} aria-hidden="true">
      <g transform={`translate(${x} 0)`}>
        {part === "left" && <path className="hl" d="M6 0.75A5.25 5.25 0 0 0 0.75 6V9H6Z" />}
        {part === "right" && <path className="hl" d="M6 0.75A5.25 5.25 0 0 1 11.25 6V9H6Z" />}
        <rect className="line" x="0.75" y="0.75" width="10.5" height="13.5" rx="5.25" />
        <path className="line" d="M0.75 9H11.25M6 0.75V9" />
        <rect className={part === "wheel" ? "hl wheel" : "wheel"} x="4.3" y="2.5" width="3.4" height="4.2" rx="1.7" />
      </g>
      {drag && <path className="line" d="M1 7.5H6.5M3.5 5L1 7.5L3.5 10M21.5 7.5H27M24.5 5L27 7.5L24.5 10" />}
    </svg>
  );
}

/** Enter キーの絵。JIS 配列の縦長で L 字型のキーに、改行の矢印を入れる */
function EnterIcon() {
  return (
    <svg className="enter" viewBox="0 0 24 18" aria-hidden="true">
      <path className="line" d="M1 1H23V17H8V8.5H1Z" />
      <path className="line" d="M19 4.5V12.5H11M12.8 10.7L11 12.5L12.8 14.3" />
    </svg>
  );
}

/** 下段の文節の下に出す候補一覧。下に入りきらなければ上に出す */
/**
 * 上の掴みどころ・余白・下の案内の欄を左ボタンで押さえたら、ウィンドウを動かす。
 * 文字の上はピンや文節の操作に使うので動かさない。動かした位置は隠すときに Rust 側で覚える
 */
function startMove(e: React.PointerEvent<HTMLElement>) {
  if (e.button !== 0) return;
  const target = e.target as HTMLElement;
  if (target !== e.currentTarget && !target.closest(".footer")) return;
  e.preventDefault();
  getCurrentWindow()
    .startDragging()
    .catch(() => {});
}

/** 掴みどころを2回押したとみなす間隔 */
const DOUBLE_CLICK_MS = 400;
let lastGrabberDown = 0;

/**
 * 掴みどころを押したら動かし、続けてもう一度押したら画面の中央に戻す。
 * 1回目で OS の移動の処理に入るため、ダブルクリックのイベントは届かない。押した間隔で見分ける
 */
function onGrabberDown(e: React.PointerEvent<HTMLElement>) {
  if (e.button !== 0) return;
  const now = performance.now();
  if (now - lastGrabberDown < DOUBLE_CLICK_MS) {
    lastGrabberDown = 0;
    e.preventDefault();
    engine.center();
    return;
  }
  lastGrabberDown = now;
  startMove(e);
}

type ResizeDirection = Parameters<ReturnType<typeof getCurrentWindow>["startResizeDragging"]>[0];

/** 小さくできる限り。tauri.conf.json の minWidth・minHeight と合わせる */
const MIN_WIDTH = 496;
const MIN_HEIGHT = 316;

/**
 * 縁を押さえて大きさを変える。まず OS に任せ、できない OS（macOS）ではポインターの動きから自分で変える
 */
function startResize(e: React.PointerEvent<HTMLElement>, dir: ResizeDirection) {
  const el = e.currentTarget;
  const pointerId = e.pointerId;
  const win = getCurrentWindow();
  const origin = { x: e.screenX, y: e.screenY };
  let pointer = origin;
  let start: { x: number; y: number; width: number; height: number } | null = null;
  let frame = 0;
  const apply = () => {
    frame = 0;
    if (!start) return;
    const dx = pointer.x - origin.x;
    const dy = pointer.y - origin.y;
    let { x, y, width, height } = start;
    if (dir.includes("East")) width = Math.max(MIN_WIDTH, start.width + dx);
    if (dir.includes("West")) {
      width = Math.max(MIN_WIDTH, start.width - dx);
      x = start.x + start.width - width;
    }
    if (dir.includes("South")) height = Math.max(MIN_HEIGHT, start.height + dy);
    if (dir.includes("North")) {
      height = Math.max(MIN_HEIGHT, start.height - dy);
      y = start.y + start.height - height;
    }
    win.setSize(new LogicalSize(width, height));
    if (x !== start.x || y !== start.y) win.setPosition(new LogicalPosition(x, y));
  };
  const onMove = (ev: PointerEvent) => {
    pointer = { x: ev.screenX, y: ev.screenY };
    if (start && !frame) frame = requestAnimationFrame(apply);
  };
  const onUp = () => {
    el.removeEventListener("pointermove", onMove);
    el.removeEventListener("pointerup", onUp);
    el.removeEventListener("pointercancel", onUp);
  };
  el.addEventListener("pointermove", onMove);
  el.addEventListener("pointerup", onUp);
  el.addEventListener("pointercancel", onUp);
  win.startResizeDragging(dir).catch(async () => {
    // OS に任せるときは掴まない（Windows の大きさ変更の処理とぶつからないように）
    try {
      el.setPointerCapture(pointerId);
    } catch {
      // もうボタンを離していたら何もしない
      onUp();
      return;
    }
    const [position, size, scale] = await Promise.all([win.outerPosition(), win.outerSize(), win.scaleFactor()]);
    const p = position.toLogical(scale);
    const s = size.toLogical(scale);
    start = { x: p.x, y: p.y, width: s.width, height: s.height };
    apply();
  });
}

const RESIZE_EDGES: { dir: ResizeDirection; className: string }[] = [
  { dir: "North", className: "n" },
  { dir: "South", className: "s" },
  { dir: "West", className: "w" },
  { dir: "East", className: "e" },
  { dir: "NorthWest", className: "nw" },
  { dir: "NorthEast", className: "ne" },
  { dir: "SouthWest", className: "sw" },
  { dir: "SouthEast", className: "se" },
];

/**
 * 大きさを変えるための掴みどころ。枠のないウィンドウは OS の縁がないため。
 * パネルの外の透明な余白と、パネルの縁のぼかしにまたがって置き、見た目の縁より少し外を押しても掴めるようにする
 */
function ResizeHandles() {
  return (
    <>
      {RESIZE_EDGES.map(({ dir, className }) => (
        <div
          key={dir}
          className={"resize " + className}
          onPointerDown={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            e.stopPropagation();
            startResize(e, dir);
          }}
        />
      ))}
    </>
  );
}

function CandidateList({ segment, anchor }: { segment: Segment; anchor: () => HTMLElement | null }) {
  const ref = useRef<HTMLOListElement>(null);
  useLayoutEffect(() => {
    const list = ref.current;
    const a = anchor()?.getBoundingClientRect();
    if (!list || !a) return;
    const below = a.bottom + 4;
    const top = below + list.offsetHeight <= window.innerHeight ? below : a.top - 4 - list.offsetHeight;
    list.style.top = `${Math.max(0, top)}px`;
    list.style.left = `${Math.max(0, Math.min(a.left, window.innerWidth - list.offsetWidth))}px`;
  });
  const n = segment.candidates.length;
  const half = Math.floor(CANDIDATE_LIST_SIZE / 2);
  const from = Math.max(0, Math.min(segment.selected - half, n - CANDIDATE_LIST_SIZE));
  const shown = segment.candidates.slice(from, from + CANDIDATE_LIST_SIZE);
  return (
    <ol className="candidates" ref={ref}>
      {shown.map((c, j) => (
        <li key={from + j} className={from + j === segment.selected ? "selected" : ""}>
          {c}
        </li>
      ))}
      <li className="count">
        {segment.selected + 1}/{n}
      </li>
    </ol>
  );
}
