import { Fragment, useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { engine } from "./engine";
import { feed, flush } from "./romaji";
import {
  boundariesOf,
  carrySelections,
  clampPin,
  emptySession,
  length,
  rangeAround,
  resultText,
  shiftBoundaries,
  shiftPosition,
  splice,
  type EngineSegment,
  type Segment,
  type Session,
} from "./session";

/** ホイールの何ピクセル分で候補を1つ送るか（トラックパッドの細かい量をまとめる） */
const WHEEL_STEP = 50;
/** ホイールを止めてから候補一覧を消すまで */
const CANDIDATE_LIST_MS = 1500;
/** 行単位のホイールで Ctrl+スクロールするときの1行の高さ */
const WHEEL_LINE_PX = 40;
/** 候補一覧に並べる数 */
const CANDIDATE_LIST_SIZE = 9;

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
  const listTimer = useRef<number | undefined>(undefined);
  const wheelAcc = useRef(0);
  const [drag, setDrag] = useState<Drag | null>(null);
  const [hover, setHover] = useState<number | null>(null);
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
    engine.commit(text, segments);
  };

  const convert = () => {
    const s = settle(sessionRef.current);
    if (s.kana === "") return;
    update(s);
    run(s.kana, null, engine.convert(s.kana));
  };

  /** 変換後にひらがなを直したとき、直した文節だけ区切り直す */
  const edit = (old: Session, next: Session, pos: number, removed: number, inserted: number) => {
    if (!old.segments || next.kana === "") {
      update({ ...next, segments: null });
      return;
    }
    update(next);
    const n = length(next.kana);
    const boundaries = shiftBoundaries(boundariesOf(old.segments), pos, removed, inserted, n);
    const [start, end] = rangeAround(boundaries, pos, pos + inserted, n);
    run(next.kana, old.segments, engine.resegment(next.kana, boundaries, start, end), (x) =>
      shiftPosition(x, pos, removed, inserted),
    );
  };

  const type = (ch: string) => {
    const s = sessionRef.current;
    const r = feed(s.pending, ch);
    if (r.kana === "") {
      update({ ...s, pending: r.pending });
      return;
    }
    const inserted = length(r.kana);
    const next = { ...s, kana: splice(s.kana, s.caret, 0, r.kana), caret: s.caret + inserted, pending: r.pending };
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

  const setBoundaries = (boundaries: number[]) => {
    const s = sessionRef.current;
    if (!s.segments) return;
    run(s.kana, s.segments, engine.convertWithBoundaries(s.kana, boundaries));
  };

  const addPin = (gap: number) => {
    const s = sessionRef.current;
    if (!s.segments) return;
    const bs = boundariesOf(s.segments);
    if (gap <= 0 || gap >= length(s.kana) || bs.includes(gap)) return;
    setBoundaries([...bs, gap].sort((a, b) => a - b));
  };

  const removePin = (index: number) => {
    const s = sessionRef.current;
    if (!s.segments) return;
    setBoundaries(boundariesOf(s.segments).filter((_, i) => i !== index));
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
    setListFor(index);
    window.clearTimeout(listTimer.current);
    listTimer.current = window.setTimeout(() => setListFor(null), CANDIDATE_LIST_MS);
  };

  const onWheel = (index: number, e: React.WheelEvent) => {
    // Ctrl+ホイールはスクロールに使う
    if (e.ctrlKey) return;
    // 行単位・ページ単位のホイールは1回で1候補、ピクセル単位は量をまとめて送る
    if (e.deltaMode !== 0) {
      cycle(index, Math.sign(e.deltaY));
      return;
    }
    wheelAcc.current += e.deltaY;
    const steps = Math.trunc(wheelAcc.current / WHEEL_STEP);
    if (steps !== 0) {
      wheelAcc.current -= steps * WHEEL_STEP;
      cycle(index, steps);
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
        bs[pin] = drag.target;
        setBoundaries(bs);
      }
    }
    setDrag(null);
  };

  /** 文字の左半分なら左の隙間、右半分なら右の隙間 */
  const gapAt = (pos: number, e: React.MouseEvent) => {
    const r = e.currentTarget.getBoundingClientRect();
    return e.clientX < r.left + r.width / 2 ? pos : pos + 1;
  };

  const handlers = useRef({ type, backspace, convert, commit, cancel, moveCaret });
  handlers.current = { type, backspace, convert, commit, cancel, moveCaret };

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
      const s = sessionRef.current;
      switch (e.key) {
        case "Escape":
          h.cancel();
          break;
        case "Enter":
          h.commit();
          break;
        case " ":
          if (!s.segments) h.convert();
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
          h.type(e.key);
      }
      e.preventDefault();
    };
    // 枠外をクリックしたら閉じる。内容は「前回」として残る
    const onBlur = () => handlers.current.cancel();
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

  /** 上段と下段で同じ文節を同じように強調する */
  const segClass = (i: number) => "seg" + (listFor === i ? " active" : hover === i ? " hover" : "");

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
    <div className="panel" ref={panelRef}>
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
      <div className="hint">
        {segments
          ? "ホイール: 候補 / ピンをドラッグ: 区切り / 右クリック: ピン追加・削除 / Ctrl+ホイール: スクロール / Enter: コピー"
          : "Space: 変換 / Enter: そのままコピー / Esc: 閉じる"}
      </div>
    </div>
  );
}

/** 下段の文節の下に出す候補一覧。下に入りきらなければ上に出す */
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
