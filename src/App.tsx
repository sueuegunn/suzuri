import { Fragment, useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalPosition, LogicalSize } from "@tauri-apps/api/dpi";
import { applyAppearance, engine, settings, userDictionary, type Appearance, type UserWord } from "./engine";
import { applyFunctionKey, isFunctionKey, type FunctionKey } from "./kana";
import { feed, flush } from "./romaji";
import {
  boundariesOf,
  carrySelections,
  clampPin,
  emptySession,
  length,
  mergeRange,
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
/** ピンの上で右ボタンを押してからこれだけ動かしたら、範囲の選択に移る */
const PIN_DRAG_START_PX = 4;
/** ピンの上で左ボタンを押してからこれより短く離し、ほとんど動かさなければ、クリックとみなして入力カーソルを置く */
const PIN_CLICK_MS = 300;
/** 候補の行の文節をユーザー辞書に登録したときの知らせを出しておく時間。その間は左クリックで取り消せる */
const NOTICE_MS = 5000;
/** 登録したときの知らせの、語の前と後ろ。語が長くて1行に収まらないときは、語の末尾だけを省略する */
const REGISTERED_BEFORE = "ユーザ辞書に「";
const REGISTERED_AFTER = "」を登録しました。左クリックでキャンセルできます。";

/** 区切り線の上に1行だけ出す知らせ。word があれば、左クリックでその登録を取り消す */
type Notice = { id: number; text: string; word: UserWord | null };
/** ドラッグ中に端を越えて反対側へ移したあと、これだけの間は次を移さない。移す前の位置のイベントが遅れて届き、戻されないようにする */
const WRAP_COOLDOWN_MS = 100;
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

/** 打った文字をローマ字としてかなにするか（kana）、そのまま入れるか（alnum） */
type InputMode = "kana" | "alnum";

/** Rust の "summon" イベントの中身 */
type Summon = {
  mode: "new" | "previous";
  /** 始めるときの入力モード。トレイメニューの設定と呼び出し元の IME から Rust 側で決める */
  input: InputMode;
};

const IS_MAC = navigator.userAgent.includes("Mac");

/**
 * 次の描画が終わるのを待つ。描画が止まっているときに待ち続けないよう、長くても 50ms で打ち切る。
 * 描画の通知（rAF）で抜けたら "raf"、打ち切ったら "timeout" を返す
 */
function afterPaint(): Promise<"raf" | "timeout"> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve("timeout"), 50);
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        clearTimeout(timer);
        resolve("raf");
      }),
    );
  });
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
  /** 閉じてから次に呼び出されるまでの間か */
  const closedRef = useRef(false);
  /**
   * パネルを見せるか。閉じるときは隠す前に消してウィンドウを丸ごと透明にし、出すときは描き終えてから見せる。
   * 隠した WebView は描き直さず、次に出した瞬間に最後の絵が、移す前の位置（前のディスプレイ）に見えることがあるため
   */
  const [shown, setShown] = useState(true);
  /** 呼び出しの回数。パネルを見せる前に閉じられたり呼び出し直されたりしたら見せない */
  const summonRef = useRef(0);
  const [listFor, setListFor] = useState<number | null>(null);
  const wheel = useRef({ acc: 0, index: -1, at: 0 });
  const [drag, setDrag] = useState<Drag | null>(null);
  /** 右ドラッグで選んでいる範囲（押した位置と今の位置の隙間の番号）。離したら1文節にまとめる */
  const [rangeSel, setRangeSel] = useState<{ from: number; to: number } | null>(null);
  const rangeRef = useRef(rangeSel);
  const setRange = (r: { from: number; to: number } | null) => {
    rangeRef.current = r;
    setRangeSel(r);
  };
  /** マウスを乗せている文節。F6〜F10 をこの文節に効かせる。強調は CSS の :hover で行うので、描き直さないよう ref だけに持つ */
  const hoverRef = useRef<number | null>(null);
  /** 上の掴みどころ・入力モードの切り替えのどれにマウスが乗っているか。乗っている間は、左下の案内をその使い方に替える */
  const [topHover, setTopHover] = useState<"grabber" | "input" | null>(null);
  /** 呼び出すたびに、呼び出し元の IME に合わせて決め直す */
  const [inputMode, setInputModeState] = useState<InputMode>("kana");
  const inputModeRef = useRef(inputMode);
  const [mode, setMode] = useState<Mode>(loadMode);
  const modeRef = useRef(mode);
  const panelRef = useRef<HTMLDivElement>(null);
  /** ひらがなと候補を並べる領域。ピンの位置の計算の基準 */
  const bodyRef = useRef<HTMLDivElement>(null);
  /** 各文節の候補の行の要素。候補一覧の位置合わせに使う */
  const candidateRefs = useRef<(HTMLSpanElement | null)[]>([]);
  /** 候補の行で右ボタンを押した文節と、今乗っている文節（文節の番号）。離したらその範囲を辞書に登録する */
  const [registerSel, setRegisterSelState] = useState<{ from: number; to: number } | null>(null);
  const registerSelRef = useRef(registerSel);
  const setRegisterSel = (r: { from: number; to: number } | null) => {
    registerSelRef.current = r;
    setRegisterSelState(r);
  };
  const [notice, setNotice] = useState<Notice | null>(null);
  const noticeTimer = useRef<number | undefined>(undefined);
  const noticeId = useRef(0);

  /** 知らせにマウスが乗っているか。乗っている間は残り時間を5秒に戻して止める */
  const [noticeHover, setNoticeHover] = useState(false);
  /** 円の印を欠け始めからやり直すたびに増やす（要素を作り直してアニメーションを最初から流す） */
  const [noticeRun, setNoticeRun] = useState(0);

  /** 今の知らせを、今から5秒後に消す */
  const startNoticeTimer = (id: number) => {
    window.clearTimeout(noticeTimer.current);
    noticeTimer.current = window.setTimeout(() => setNotice((n) => (n?.id === id ? null : n)), NOTICE_MS);
  };

  const showNotice = (text: string, word: UserWord | null) => {
    const id = ++noticeId.current;
    setNotice({ id, text, word });
    setNoticeHover(false);
    startNoticeTimer(id);
  };

  const onNoticeEnter = () => {
    window.clearTimeout(noticeTimer.current);
    setNoticeHover(true);
  };

  const onNoticeLeave = () => {
    if (!notice) return;
    setNoticeHover(false);
    setNoticeRun((r) => r + 1);
    startNoticeTimer(notice.id);
  };

  const hideNotice = () => {
    window.clearTimeout(noticeTimer.current);
    setNotice(null);
  };

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
    adjust: (segments: Segment[]) => Segment[] = (x) => x,
  ) => {
    const id = ++requestRef.current;
    const segments = await call;
    const cur = sessionRef.current;
    if (id !== requestRef.current || cur.kana !== reading) return;
    update({ ...cur, segments: adjust(carrySelections(previous, segments, shift)) });
  };

  /**
   * 前回の内容を覚え、画面を空にする。返す Promise は空の画面を描き終えたら解決する。
   * 隠したあとの WebView は描き直さないので、隠す前に空にしておかないと、次に出したときに
   * 前の内容が一部残って見えることがある（macOS で起きた）
   */
  const close = () => {
    // 隠したときの blur でもう一度呼ばれるので、空にした画面で前回を上書きしない
    if (closedRef.current) return Promise.resolve();
    closedRef.current = true;
    previousRef.current = settle(sessionRef.current);
    // 届く途中の変換結果を捨てる
    requestRef.current++;
    update(emptySession);
    setListFor(null);
    setDrag(null);
    setRange(null);
    setRegisterSel(null);
    hideNotice();
    setShown(false);
    // 消し終えてから隠せたかを確かめるため、描画の通知で抜けたか打ち切ったかを残す（macOS の調査用）
    return afterPaint().then((how) => engine.debugLog(`close: ${how}`));
  };

  const cancel = (returnFocus: boolean) => {
    close().then(() => engine.cancel(returnFocus));
  };

  const commit = () => {
    const s = settle(sessionRef.current);
    const text = resultText(s);
    const closed = close();
    if (text === "") {
      closed.then(() => engine.cancel(true));
      return;
    }
    const segments = (s.segments ?? []).map((seg) => ({
      reading: seg.reading,
      surface: seg.candidates[seg.selected],
      wasFirst: seg.selected === 0,
      // 手で置いたピンに接する文節は、区切りの学習で覚える
      manual: s.pinned.includes(seg.start) || s.pinned.includes(seg.end),
    }));
    const paste = modeRef.current === "paste";
    closed.then(() => engine.commit(text, segments, paste));
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
    // 候補を選んだ文節は区切り直さず、選んだ候補を保つ
    const fixed = old.segments
      .filter((seg) => seg.selected !== 0)
      .flatMap((seg) => [seg.start, seg.end].map((x) => shiftPosition(x, pos, removed, inserted)));
    const [start, end] = retypeRange(boundaries, pinned, pos, pos + inserted, n, fixed);
    run(next.kana, old.segments, engine.resegment(next.kana, boundaries, start, end), (x) =>
      shiftPosition(x, pos, removed, inserted),
    );
  };

  /**
   * 直前の打鍵で打ちかけのローマ字が確定したときの、打つ前の状態と打ったあとの状態。
   * 次の Backspace でその打鍵だけを取り消し、前後の打鍵をつなげる（t k BS u → つ、n k BS a → な）
   */
  const retypeRef = useRef<{ before: Session; after: Session } | null>(null);

  const type = (ch: string) => {
    const s = sessionRef.current;
    const r = feed(s.pending, ch, Array.from(s.kana)[s.caret - 1]);
    // 打った文字が打ちかけのローマ字として残っているときだけ。「ka」で「か」になったあとの BS は「か」を消す
    const kana = splice(s.kana, s.caret, 0, r.kana);
    const after = { ...s, kana, caret: s.caret + length(r.kana), pending: r.pending };
    retypeRef.current = s.pending !== "" && r.pending !== "" ? { before: s, after } : null;
    if (r.kana === "") {
      update({ ...s, pending: r.pending });
      return;
    }
    const inserted = length(r.kana);
    const next = { ...s, kana: splice(s.kana, s.caret, 0, r.kana), caret: s.caret + inserted, pending: r.pending };
    edit(s, next, s.caret, 0, inserted);
  };

  /** 入力モードを変える。打ちかけのローマ字は、そのまま確定させてから切り替える */
  const setInputMode = (next: InputMode) => {
    if (inputModeRef.current === next) return;
    typeLetter("");
    inputModeRef.current = next;
    setInputModeState(next);
    // 呼び出し元の OS の IME は変えない。閉じたときだけ変えると、いつ変わるのか分かりにくいため
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
    const retype = retypeRef.current;
    retypeRef.current = null;
    // Shift+英字やカーソル移動を挟んだときは、かなや打ちかけが変わっているので取り消さない
    const after = retype?.after;
    if (retype && s.kana === after?.kana && s.caret === after.caret && s.pending === after.pending) {
      const { before } = retype;
      const removed = s.caret - before.caret;
      const next = { ...s, kana: before.kana, caret: before.caret, pending: before.pending };
      if (removed === 0) update(next);
      else edit(s, next, before.caret, removed, 0);
      return;
    }
    if (s.pending !== "") {
      update({ ...s, pending: s.pending.slice(0, -1) });
      return;
    }
    if (s.caret === 0) return;
    const next = { ...s, kana: splice(s.kana, s.caret - 1, 1, ""), caret: s.caret - 1 };
    edit(s, next, s.caret - 1, 1, 0);
  };

  /** カーソルの後ろの1文字を消す（Delete）。打ちかけのローマ字は先に確定させる */
  const deleteForward = () => {
    retypeRef.current = null;
    typeLetter("");
    const s = sessionRef.current;
    if (s.caret >= length(s.kana)) return;
    const next = { ...s, kana: splice(s.kana, s.caret, 1, "") };
    edit(s, next, s.caret, 1, 0);
  };

  /**
   * カーソルの前（`forward` なら後ろ）を、文節の区切りまでまとめて消す（Ctrl+Backspace・Ctrl+Delete）。
   * `all` なら先頭（末尾）まで消す（macOS の Cmd+Backspace）。打ちかけのローマ字があれば、それだけを消す
   */
  const deleteChunk = (forward: boolean, all: boolean) => {
    retypeRef.current = null;
    const s = sessionRef.current;
    if (s.pending !== "") {
      update({ ...s, pending: "" });
      return;
    }
    const n = length(s.kana);
    const bs = s.segments ? boundariesOf(s.segments) : [];
    const [from, to] = forward
      ? [s.caret, all ? n : Math.min(n, ...bs.filter((b) => b > s.caret))]
      : [all ? 0 : Math.max(0, ...bs.filter((b) => b < s.caret)), s.caret];
    if (from >= to) return;
    const next = { ...s, kana: splice(s.kana, from, to - from, ""), caret: from };
    edit(s, next, from, to - from, 0);
  };

  /**
   * クリップボードの文字をカーソル位置に入れる。ローマ字にはせず、漢字も英字もそのまま読みに入れる。
   * 改行は \n にそろえ、タブは空白に、ほかの制御文字は捨てる
   */
  const paste = async () => {
    const raw = await engine.readClipboard();
    if (!closedRef.current) pasteText(raw);
  };

  const pasteText = (raw: string) => {
    const text = raw
      .replace(/\r\n?/g, "\n")
      .replace(/\t/g, " ")
      .replace(/[\u0000-\u0009\u000b-\u001f\u007f]/g, "");
    if (text !== "") handlers.current.typeLetter(text);
  };

  /** マウスを乗せている文節を、F6〜F10 でひらがな・カタカナ・英数などに書き換える */
  const applyKey = (key: FunctionKey) => {
    const s = sessionRef.current;
    const index = hoverRef.current;
    const seg = index === null ? undefined : s.segments?.[index];
    if (index === null || !seg || !s.segments) return;
    const surface = applyFunctionKey(key, seg.reading);
    // 候補にない書き方（半角カタカナなど）は、候補の最後に足して選ぶ
    const found = seg.candidates.indexOf(surface);
    const candidates = found < 0 ? [...seg.candidates, surface] : seg.candidates;
    const selected = found < 0 ? candidates.length - 1 : found;
    const segments = s.segments.map((x, i) => (i === index ? { ...x, candidates, selected } : x));
    update({ ...s, segments });
    setListFor(index);
  };

  /**
   * 文節の候補を、今の表記のままひらがなの行（読み）に書き込む。「明るい」を書き込んでから「るい」を消し、
   * 読みを知らない「明」だけを残す使い方のため。書き込んだ漢字は貼り付けた漢字と同じく、そのまま変換を通る。
   * 区切りは変えずに変換し直し、書き込んだ文節は書き込んだ表記を選ぶ（「明るい」の「るい」が「類」にならないように）
   */
  const bake = (index: number) => {
    const s = sessionRef.current;
    const seg = s.segments?.[index];
    // 打ちかけのローマ字があると、文節の位置が読みとずれるので何もしない
    if (!s.segments || !seg || s.pending !== "") return;
    const surface = seg.candidates[seg.selected];
    if (surface === seg.reading) return;
    const removed = seg.end - seg.start;
    const inserted = length(surface);
    const shift = (x: number) => shiftPosition(x, seg.start, removed, inserted);
    const kana = splice(s.kana, seg.start, removed, surface);
    const boundaries = boundariesOf(s.segments).map(shift);
    retypeRef.current = null;
    update({ ...s, kana, caret: seg.start + inserted, pinned: s.pinned.map(shift) });
    run(kana, s.segments, engine.convertWithBoundaries(kana, boundaries), shift, (segments) =>
      segments.map((x) => {
        if (x.start !== seg.start || x.reading !== surface) return x;
        const found = x.candidates.indexOf(surface);
        const candidates = found < 0 ? [...x.candidates, surface] : x.candidates;
        return { ...x, candidates, selected: found < 0 ? candidates.length - 1 : found };
      }),
    );
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

  /** `from`〜`to` の文字を1文節にまとめ、両側に残った部分を区切り直す */
  const mergeSelected = (from: number, to: number) => {
    const s = sessionRef.current;
    if (!s.segments) return;
    const m = mergeRange(boundariesOf(s.segments), s.pinned, from, to, length(s.kana));
    if (!m) return;
    update({ ...s, pinned: m.pinned });
    const call = m.ranges.length
      ? engine.resegmentRanges(s.kana, m.boundaries, m.ranges)
      : engine.convertWithBoundaries(s.kana, m.boundaries);
    run(s.kana, s.segments, call);
  };

  /**
   * 左クリックした点に一番近い、入力カーソルを置ける位置。文字の上でなくても（行の右の余白や
   * 最後の行の下でも）、縦に一番近い行の中で横に一番近い文字の、近い側に置く
   */
  const caretFromPoint = (x: number, y: number) => {
    const els = bodyRef.current?.querySelectorAll<HTMLElement>("[data-pos]") ?? [];
    let best: { pos: number; r: DOMRect } | null = null;
    let bestRow = Infinity;
    let bestCol = Infinity;
    for (const el of els) {
      const r = el.getBoundingClientRect();
      const row = y < r.top ? r.top - y : y > r.bottom ? y - r.bottom : 0;
      const col = x < r.left ? r.left - x : x > r.right ? x - r.right : 0;
      if (row < bestRow || (row === bestRow && col < bestCol)) {
        best = { pos: Number(el.dataset.pos), r };
        [bestRow, bestCol] = [row, col];
      }
    }
    if (!best) return 0;
    // 改行は行の右端に幅 0 で置かれる。その右を押しても次の行の先頭にせず、改行の手前に置く
    if (Array.from(sessionRef.current.kana)[best.pos] === "\n") return best.pos;
    return x < best.r.left + best.r.width / 2 ? best.pos : best.pos + 1;
  };

  /**
   * ↑↓で、入力カーソルの横の位置のまま上・下の行に動かす。上・下に行がなければ先頭・末尾に動かす
   */
  const moveVertical = (dir: -1 | 1) => {
    const body = bodyRef.current;
    const caret = body?.querySelector<HTMLElement>(".caret")?.getBoundingClientRect();
    if (!body || !caret) return;
    const mid = (r: DOMRect) => (r.top + r.bottom) / 2;
    // 今の行より上（下）にある文字のうち、一番近い行
    let target: number | null = null;
    for (const el of body.querySelectorAll<HTMLElement>("[data-pos]")) {
      const r = el.getBoundingClientRect();
      const beyond = dir < 0 ? mid(r) < caret.top : mid(r) > caret.bottom;
      if (beyond && (target === null || Math.abs(mid(r) - mid(caret)) < Math.abs(target - mid(caret)))) {
        target = mid(r);
      }
    }
    if (target === null) {
      moveCaret((s) => (dir < 0 ? 0 : length(s.kana)));
      return;
    }
    const pos = caretFromPoint(caret.left, target);
    moveCaret(() => pos);
  };

  const onReadingClick = (e: React.MouseEvent) => {
    // ピンの上はピンのドラッグに使う。macOS の Control+クリックは右クリックなので除く
    if (e.button !== 0 || (IS_MAC && e.ctrlKey) || (e.target as HTMLElement).closest(".pin")) return;
    const pos = caretFromPoint(e.clientX, e.clientY);
    moveCaret(() => pos);
  };

  /** 画面上の点にある文字の、近い側の隙間。文字の上でなければ null */
  const gapFromPoint = (x: number, y: number) => {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-pos]");
    if (!el || !bodyRef.current?.contains(el)) return null;
    const r = el.getBoundingClientRect();
    const pos = Number(el.dataset.pos);
    return x < r.left + r.width / 2 ? pos : pos + 1;
  };

  /**
   * ドラッグ中にポインターがパネルの左右の端を越えたら、反対側の端へ移す。越えた分だけ内側に入れる。
   * ドラッグ中はポインターを捕まえているので、ウィンドウの外に出てもイベントが届く
   */
  const lastWrapRef = useRef(0);
  const wrapAtEdges = (e: React.PointerEvent) => {
    const r = panelRef.current?.getBoundingClientRect();
    const now = performance.now();
    if (!r || now - lastWrapRef.current < WRAP_COOLDOWN_MS) return;
    let x;
    if (e.clientX < r.left) x = e.clientX + r.width;
    else if (e.clientX >= r.right) x = e.clientX - r.width;
    else return;
    x = Math.min(Math.max(x, r.left + 1), r.right - 2);
    lastWrapRef.current = now;
    getCurrentWindow()
      .setCursorPosition(new LogicalPosition(x, e.clientY))
      .catch(() => {});
  };

  /** 文字の上で右ボタンを押したら、範囲の選択を始める。動かさずに離したらピンを足す */
  const onCharRightDown = (pos: number, e: React.PointerEvent) => {
    if (!isRightButton(e) || !sessionRef.current.segments) return;
    const g = gapAt(pos, e);
    setRange({ from: g, to: g });
    bodyRef.current?.setPointerCapture(e.pointerId);
  };

  /**
   * ピンの上で右ボタンを押したところ。動かさずに離したらピンを消し、
   * 少し動かしたらピンの位置から範囲の選択に移る（押した瞬間に消すとドラッグに移れない）
   */
  const pinPressRef = useRef<{ index: number; gap: number; x: number; y: number } | null>(null);

  const onPinRightDown = (index: number, e: React.PointerEvent) => {
    const s = sessionRef.current;
    if (!isRightButton(e) || !s.segments) return;
    const gap = boundariesOf(s.segments)[index];
    pinPressRef.current = { index, gap, x: e.clientX, y: e.clientY };
    bodyRef.current?.setPointerCapture(e.pointerId);
  };

  const onRangeMove = (e: React.PointerEvent) => {
    const press = pinPressRef.current;
    if (press && Math.hypot(e.clientX - press.x, e.clientY - press.y) >= PIN_DRAG_START_PX) {
      pinPressRef.current = null;
      setRange({ from: press.gap, to: press.gap });
    }
    const r = rangeRef.current;
    if (!r) return;
    wrapAtEdges(e);
    const g = gapFromPoint(e.clientX, e.clientY);
    if (g !== null && g !== r.to) setRange({ ...r, to: g });
  };

  const onRangeUp = () => {
    // 押したときに右ボタンと分かっているので、離したボタンは見ない（macOS で Control を先に離しても続ける）
    const press = pinPressRef.current;
    if (press) {
      pinPressRef.current = null;
      removePin(press.index);
      return;
    }
    const r = rangeRef.current;
    if (!r) return;
    setRange(null);
    if (r.from === r.to) addPin(r.from);
    else mergeSelected(r.from, r.to);
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
    const els = [...body.querySelectorAll<HTMLElement>("[data-pos]")].sort(
      (a, b) => Number(a.dataset.pos) - Number(b.dataset.pos),
    );
    const chars = els.map((el) => el.getBoundingClientRect());
    const gaps = new Map<number, { x: number; y: number; height: number }>();
    for (let g = 1; g < chars.length; g++) {
      const [prev, cur] = [chars[g - 1], chars[g]];
      const sameRow = Math.abs(prev.top - cur.top) < cur.height / 2;
      const x = sameRow ? (prev.right + cur.left) / 2 : cur.left;
      // ゴーストのピンは本物のピンと同じく、文字の上端から候補の行の下端（文節の箱の下端）の少し上まで伸ばす
      // （本物のピンは下端から PIN_INSET_PX だけ縮めている。styles.css の .pin）
      const box = els[g].closest(".seg-block")?.getBoundingClientRect();
      const bottom = box ? box.bottom - PIN_INSET_PX : cur.bottom;
      // ゴーストのピンを領域の中に置くので、領域の左上を原点にする
      gaps.set(g, { x: x - origin.left, y: cur.top - origin.top + body.scrollTop, height: bottom - cur.top });
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
    wrapAtEdges(e);
    const s = sessionRef.current;
    if (!s.segments) return;
    const target = clampPin(boundariesOf(s.segments), pin, nearestGap(e.clientX, e.clientY), length(s.kana));
    if (target === null) return;
    const p = gapPositions().get(target);
    if (p) setDrag({ pin, target, ...p });
  };

  /** ピンの上で左ボタンを押したところ。すぐに、ほとんど動かさずに離したらクリックとみなす */
  const pinDownRef = useRef<{ pin: number; x: number; y: number; at: number } | null>(null);

  const onPinUp = (pin: number, e: React.PointerEvent) => {
    const s = sessionRef.current;
    const down = pinDownRef.current;
    pinDownRef.current = null;
    const clicked =
      down?.pin === pin &&
      performance.now() - down.at < PIN_CLICK_MS &&
      Math.hypot(e.clientX - down.x, e.clientY - down.y) < PIN_DRAG_START_PX;
    if (clicked && s.segments) {
      // ピンの位置（文節の区切り）に入力カーソルを置く
      const gap = boundariesOf(s.segments)[pin];
      setDrag(null);
      moveCaret(() => gap);
      return;
    }
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

  // 打ったり入力カーソルを動かしたりしたら、入力カーソルのある文節の箱（候補の行まで）を見える範囲に入れる。
  // 長い文で最後の行が隠れたままにならないように。変換結果は打鍵より後に届くので、文節が変わったときにも合わせ直す。
  // 候補をホイールで替えただけでは動かさない（スクロールして選んでいる文節から引き戻さない）
  useLayoutEffect(() => {
    const body = bodyRef.current;
    const caret = body?.querySelector<HTMLElement>(".caret");
    if (!body || !caret) return;
    keepInView(body, caret.closest<HTMLElement>(".seg-block") ?? caret);
  }, [session.kana, session.caret, session.pending, session.segments?.map((seg) => seg.end).join()]);

  // 見た目は起動時に読んでおき、設定画面で変えたらイベントで受け取る
  useEffect(() => {
    settings.getAppearance().then(applyAppearance);
    const unlisten = [listen<Appearance>("appearance", (e) => applyAppearance(e.payload))];
    return () => {
      unlisten.forEach((u) => u.then((f) => f()));
    };
  }, []);

  const handlers = useRef({
    type,
    typeLetter,
    backspace,
    deleteForward,
    deleteChunk,
    applyKey,
    paste,
    pasteText,
    commit,
    cancel,
    moveCaret,
    moveVertical,
    toggleMode,
    setInputMode,
  });
  handlers.current = {
    type,
    typeLetter,
    backspace,
    deleteForward,
    deleteChunk,
    applyKey,
    paste,
    pasteText,
    commit,
    cancel,
    moveCaret,
    moveVertical,
    toggleMode,
    setInputMode,
  };

  useEffect(() => {
    const unlisten = listen<Summon>("summon", (e) => {
      closedRef.current = false;
      const id = ++summonRef.current;
      setShown(false);
      // 新しい状態を描き、ウィンドウの移動が反映されるまでもう1フレーム待ってから見せる
      afterPaint().then((how) => {
        if (id !== summonRef.current || closedRef.current) return;
        setShown(true);
        engine.debugLog(`show: ${how}`);
      });
      update(e.payload.mode === "previous" ? previousRef.current : emptySession);
      inputModeRef.current = e.payload.input;
      setInputModeState(e.payload.input);
      setListFor(null);
      setDrag(null);
      setRange(null);
      hoverRef.current = null;
      window.focus();
    });
    // Alt+F4 などで閉じられたら、Esc と同じく閉じる
    const unlistenClose = listen("close-requested", () => handlers.current.cancel(true));
    // macOS の英数・かなキーは入力メソッドが先に取ってしまうので、Rust 側で受け取ってイベントで届く
    const unlistenLang = listen<InputMode>("lang-key", (e) => handlers.current.setInputMode(e.payload));
    const onKey = (e: KeyboardEvent) => {
      // 英数・かなのキー。名前が付かず、物理的な位置（Lang2・Lang1）でしか見分けられないことがある
      const lang = langMode(e);
      if (lang) {
        handlers.current.setInputMode(lang);
        e.preventDefault();
        return;
      }
      // 貼り付け。Windows は Ctrl+V、macOS は Cmd+V
      if ((IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey) && !e.altKey && e.code === "KeyV") {
        handlers.current.paste();
        e.preventDefault();
        return;
      }
      // 文節の区切りまでまとめて消す。Windows は Ctrl、macOS は Option（Cmd なら先頭・末尾まで）
      if ((e.key === "Backspace" || e.key === "Delete") && !e.isComposing) {
        const chunk = IS_MAC ? e.altKey && !e.metaKey && !e.ctrlKey : e.ctrlKey && !e.altKey && !e.metaKey;
        const all = IS_MAC && e.metaKey && !e.altKey && !e.ctrlKey;
        if (chunk || all) {
          handlers.current.deleteChunk(e.key === "Delete", all);
          e.preventDefault();
          return;
        }
      }
      // OS の IME が動いているときの打鍵は扱わない
      if (e.isComposing || e.key === "Process" || e.ctrlKey || e.metaKey || e.altKey) return;
      const h = handlers.current;
      if (isFunctionKey(e.key)) {
        h.applyKey(e.key);
        e.preventDefault();
        return;
      }
      switch (e.key) {
        case "Escape":
          h.cancel(true);
          break;
        case "Enter":
          // Shift+Enter は改行を入れる。Enter だけなら確定する
          if (e.shiftKey) h.typeLetter("\n");
          else h.commit();
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
        case "Delete":
          h.deleteForward();
          break;
        case "ArrowLeft":
          h.moveCaret((s) => s.caret - 1);
          break;
        case "ArrowRight":
          h.moveCaret((s) => s.caret + 1);
          break;
        case "ArrowUp":
          h.moveVertical(-1);
          break;
        case "ArrowDown":
          h.moveVertical(1);
          break;
        case "Home":
          h.moveCaret(() => 0);
          break;
        case "End":
          h.moveCaret((s) => length(s.kana));
          break;
        // Windows の半角/全角キーは切り替える
        case "Hankaku":
        case "Zenkaku":
        case "HankakuZenkaku":
          h.setInputMode(inputModeRef.current === "kana" ? "alnum" : "kana");
          break;
        default:
          if (e.key.length !== 1) return;
          // 英数モードは、打ったとおりの文字をそのまま入れる
          if (inputModeRef.current === "alnum") h.typeLetter(e.key);
          // CapsLock の状態によらず、Shift を押しているかで英字かローマ字かを決める
          else if (e.shiftKey && /^[a-z]$/i.test(e.key))
            h.typeLetter(e.key.toUpperCase());
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
          if (!focused) handlers.current.cancel(false);
        })
        .catch(() => handlers.current.cancel(false));
    };
    const noMenu = (e: MouseEvent) => e.preventDefault();
    // macOS では Cmd+V が「編集」メニューの「ペースト」に取られ、キーの処理に届かないことがある。
    // そのときは paste イベントで受ける（キーの処理で受けたときは既定の動きを止めているので、二重には入らない）
    const onPaste = (e: ClipboardEvent) => {
      e.preventDefault();
      if (!closedRef.current) handlers.current.pasteText(e.clipboardData?.getData("text/plain") ?? "");
    };
    // ホイールは候補の切り替えに使い、スクロールは Ctrl+ホイールかスクロールバーで行う。
    // React の onWheel は passive で止められないため、ここで止める。Ctrl+ホイールの拡大縮小も止まる
    const noWheelScroll = (e: WheelEvent) => {
      e.preventDefault();
      const area = bodyRef.current;
      if (!e.ctrlKey || !area) return;
      const unit = e.deltaMode === 1 ? WHEEL_LINE_PX : e.deltaMode === 2 ? area.clientHeight : 1;
      area.scrollTop += e.deltaY * unit;
    };
    const panel = panelRef.current!;
    panel.addEventListener("wheel", noWheelScroll, { passive: false });
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onBlur);
    window.addEventListener("contextmenu", noMenu);
    window.addEventListener("paste", onPaste);
    return () => {
      unlisten.then((f) => f());
      unlistenLang.then((f) => f());
      unlistenClose.then((f) => f());
      panel.removeEventListener("wheel", noWheelScroll);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("contextmenu", noMenu);
    window.removeEventListener("paste", onPaste);
    };
  }, []);

  const caretMark = (pos: number) =>
    session.caret === pos && (
      <span className="caret-wrap" key={`caret-${pos}`}>
        {session.pending && <span className="pending">{session.pending}</span>}
        <span className="caret" />
      </span>
    );

  const inRange = (pos: number) =>
    rangeSel !== null && pos >= Math.min(rangeSel.from, rangeSel.to) && pos < Math.max(rangeSel.from, rangeSel.to);

  const chars = (text: string, offset: number, isLast: boolean) => (
    <>
      {Array.from(text).map((c, j) => {
        const pos = offset + j;
        return (
          <Fragment key={pos}>
            {caretMark(pos)}
            <span
              className={"char" + (inRange(pos) ? " range-select" : "")}
              data-pos={pos}
              onPointerDown={(e) => onCharRightDown(pos, e)}
            >
              {c}
            </span>
          </Fragment>
        );
      })}
      {isLast && caretMark(offset + length(text))}
    </>
  );

  /** 候補の行に、候補一覧を出している印と、登録しようとしている印を付ける。マウスが乗っている行の強調は CSS の :hover で行う */
  const candidateClass = (i: number) =>
    "seg seg-candidate" + (listFor === i ? " active" : "") + (registering(i) ? " register-select" : "");

  /** i〜j 番目の文節（逆順でもよい）を、今の候補のままユーザー辞書に登録する */
  const registerSegments = async (i: number, j: number) => {
    const segs = sessionRef.current.segments;
    if (!segs) return;
    const picked = segs.slice(Math.min(i, j), Math.max(i, j) + 1);
    const reading = picked.map((s) => s.reading).join("");
    const surface = picked.map((s) => s.candidates[s.selected]).join("");
    try {
      const word = await userDictionary.registerSegment(reading, surface);
      showNotice(`${REGISTERED_BEFORE}${word.surface}${REGISTERED_AFTER}`, word);
    } catch (e) {
      showNotice(String(e), null);
    }
  };

  /** 知らせを左クリックしたら、登録を取り消して消す */
  const onNoticeClick = (e: React.MouseEvent) => {
    if (e.button !== 0 || (IS_MAC && e.ctrlKey)) return;
    const word = notice?.word;
    hideNotice();
    if (word) userDictionary.remove(word).catch((err) => showNotice(String(err), null));
  };

  /** 候補の行で右ボタンを押したら、離したところまでの文節を登録する。動かさずに離したらその文節だけ */
  const onRegisterDown = (i: number, e: React.PointerEvent) => {
    if (!isRightButton(e) || !sessionRef.current.segments) return;
    setRegisterSel({ from: i, to: i });
    bodyRef.current?.setPointerCapture(e.pointerId);
  };

  const onRegisterMove = (e: React.PointerEvent) => {
    const r = registerSelRef.current;
    if (!r) return;
    const el = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-seg]");
    if (!el || !bodyRef.current?.contains(el)) return;
    const to = Number(el.dataset.seg);
    if (to !== r.to) setRegisterSel({ ...r, to });
  };

  const onRegisterUp = () => {
    const r = registerSelRef.current;
    if (!r) return;
    setRegisterSel(null);
    registerSegments(r.from, r.to);
  };

  /** 領域の上での右ドラッグ。範囲を1文節にまとめる（ひらがなの行）か、辞書に登録する（候補の行）かのどちらか一方だけが動く */
  const onAreaMove = (e: React.PointerEvent) => (registerSelRef.current ? onRegisterMove(e) : onRangeMove(e));
  const onAreaUp = () => (registerSelRef.current ? onRegisterUp() : onRangeUp());
  const onAreaCancel = () => {
    pinPressRef.current = null;
    setRange(null);
    setRegisterSel(null);
  };

  const registering = (i: number) =>
    registerSel !== null && Math.min(registerSel.from, registerSel.to) <= i && i <= Math.max(registerSel.from, registerSel.to);

  /** 文節の箱（ひらがなと候補の両方）のマウス操作 */
  const segHandlers = (i: number) => ({
    onWheel: (e: React.WheelEvent) => onWheel(i, e),
    onMouseEnter: () => {
      hoverRef.current = i;
    },
    onMouseLeave: () => {
      hoverRef.current = null;
      if (listFor === i) setListFor(null);
    },
  });

  const segments = session.segments;
  return (
    <div className={"frame" + (shown ? "" : " concealed")}>
      <ResizeHandles />
      {/* 上の縁の掴みどころより手前に出すため、パネルの外に置く */}
      <div className="top-bar">
        <InputModeButton
          label="a"
          title="英数"
          active={inputMode === "alnum"}
          onSelect={() => setInputMode("alnum")}
          onHover={(on) => setTopHover(on ? "input" : null)}
        />
        <div
          className="grabber"
          onPointerDown={onGrabberDown}
          onPointerEnter={() => setTopHover("grabber")}
          onPointerLeave={() => setTopHover(null)}
        />
        <InputModeButton
          label="あ"
          title="かな"
          active={inputMode === "kana"}
          onSelect={() => setInputMode("kana")}
          onHover={(on) => setTopHover(on ? "input" : null)}
        />
      </div>
      <div className="panel" ref={panelRef} onPointerDown={startMove}>
        <div
          className="area"
          ref={bodyRef}
          onClick={onReadingClick}
          onPointerMove={onAreaMove}
          onPointerUp={onAreaUp}
          onPointerCancel={onAreaCancel}
        >
          <div className="reading">
            {segments
              ? segments.map((seg, i) => (
                  <Fragment key={`${seg.start}:${seg.end}`}>
                    {/* 改行の前後は行が変わるので区切りは明らか。ピンは出さない（区切りとしては残る） */}
                    {i > 0 && !isLineBreak(segments[i - 1]) && !isLineBreak(seg) && (
                      <span
                        className={"pin" + (drag?.pin === i - 1 ? " dragging" : "")}
                        title="ドラッグで区切りを移動、右クリックで削除、右ドラッグで1文節にまとめる"
                        onPointerDown={(e) => {
                          if (e.button === 0 && !(IS_MAC && e.ctrlKey)) {
                            pinDownRef.current = { pin: i - 1, x: e.clientX, y: e.clientY, at: performance.now() };
                            e.currentTarget.setPointerCapture(e.pointerId);
                          } else onPinRightDown(i - 1, e);
                        }}
                        onPointerMove={(e) => onPinMove(i - 1, e)}
                        onPointerUp={(e) => onPinUp(i - 1, e)}
                      />
                    )}
                    {isLineBreak(seg) ? (
                      // 改行は箱に入れず、そのまま行を改める
                      chars(seg.reading, seg.start, i === segments.length - 1)
                    ) : (
                      // ひらがなと候補を縦に積んだ箱。箱は途中で折り返さないので、文節が行をまたがない
                      <span
                        className={"seg-block" + (i === segments.length - 1 ? " last" : "")}
                        data-seg={i}
                        {...segHandlers(i)}
                      >
                        <span className="seg seg-reading">
                          {chars(seg.reading, seg.start, i === segments.length - 1)}
                        </span>
                        <span
                          className={candidateClass(i)}
                          ref={(el) => {
                            candidateRefs.current[i] = el;
                          }}
                          onPointerDown={(e) => onRegisterDown(i, e)}
                          onDoubleClick={(e) => {
                            if (e.button === 0) bake(i);
                          }}
                        >
                          {seg.candidates[seg.selected]}
                        </span>
                      </span>
                    )}
                  </Fragment>
                ))
              : chars(session.kana, 0, true)}
          </div>
          {drag && <div className="ghost-pin" style={{ left: drag.x, top: drag.y, height: drag.height }} />}
        </div>
        {notice && (
          <div className="notice-anchor">
            <div
              className={"notice" + (noticeHover ? " hover" : "")}
              key={notice.id}
              onClick={onNoticeClick}
              onMouseEnter={onNoticeEnter}
              onMouseLeave={onNoticeLeave}
              title={notice.word ? "クリックで登録を取り消す" : undefined}
            >
              <svg className="notice-timer" key={noticeRun} viewBox="0 0 20 20" aria-hidden="true">
                <circle className="notice-timer-track" cx="10" cy="10" r="9" />
                <circle className="notice-timer-pie" cx="10" cy="10" r="4.5" />
              </svg>
              {notice.word ? (
                <>
                  <span className="notice-fixed">{REGISTERED_BEFORE}</span>
                  <span className="notice-word">{notice.word.surface}</span>
                  <span className="notice-fixed">{REGISTERED_AFTER}</span>
                </>
              ) : (
                <span className="notice-word">{notice.text}</span>
              )}
            </div>
          </div>
        )}
        {segments && listFor !== null && segments[listFor] && (
          <CandidateList segment={segments[listFor]} anchor={() => candidateRefs.current[listFor]} />
        )}
        <div className="footer">
          <div className="hint">
            {topHover === "grabber" ? (
              <>
                <MouseIcon part="left" drag /> ドラッグで移動{SEP}
                <MouseIcon part="left" /> ダブルクリックで中央に戻す
              </>
            ) : topHover === "input" ? (
              <>
                <MouseIcon part="left" /> 英数・かなの切り替え{SEP}
                {IS_MAC ? "英数キー・かなキー" : "半角/全角キー・無変換キー・変換キー"}でも切り替え
              </>
            ) : segments ? (
              <>
                <MouseIcon part="wheel" /> 候補{SEP}
                <MouseIcon part="left" drag /> 文節区切り移動{SEP}
                <MouseIcon part="right" /> ピン追加削除{SEP}
                <MouseIcon part="right" drag /> 文節まとめ{SEP}
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

/**
 * 掴みどころの左右に置く、入力モードの切り替え。今のモードを強調する。
 * 押してもフォーカスを取らない（Enter や Space が押したことにならないように）
 */
function InputModeButton({
  label,
  title,
  active,
  onSelect,
  onHover,
}: {
  label: string;
  title: string;
  active: boolean;
  onSelect: () => void;
  onHover: (on: boolean) => void;
}) {
  return (
    <button
      type="button"
      tabIndex={-1}
      className={"input-mode" + (active ? " active" : "")}
      aria-label={title}
      aria-pressed={active}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        onSelect();
      }}
      onPointerEnter={() => onHover(true)}
      onPointerLeave={() => onHover(false)}
    >
      {label}
    </button>
  );
}

/**
 * el の最後の行が area の見える範囲から上下にはみ出していたら、area を最小限スクロールして入れる。
 * 文節の箱はふつう1行に収まるが、行の幅より長い文節は箱の中で折り返す。その全体を見ると上端に合わせて
 * 先頭まで戻ってしまう（打つたびに上へ飛んで戻る）ので、最後の行だけを見る。
 * scrollIntoView はパネルなど外側の要素までスクロールさせるので使わない
 */
function keepInView(area: HTMLElement, el: HTMLElement) {
  const a = area.getBoundingClientRect();
  const rects = el.getClientRects();
  const r = rects.length > 0 ? rects[rects.length - 1] : el.getBoundingClientRect();
  if (r.bottom > a.bottom) area.scrollTop += r.bottom - a.bottom;
  else if (r.top < a.top) area.scrollTop -= a.top - r.top;
}

/**
 * 英数・かなのモードを決めるキーなら、そのモード。
 * Lang2・Lang1 は Mac の英数・かなキーと、Windows で無変換・変換に IME オフ・オンを割り当てたとき
 * （VK_IME_OFF・VK_IME_ON。key は Unidentified で届く）。割り当てていない無変換・変換は名前で届く
 */
function langMode(e: KeyboardEvent): InputMode | null {
  if (e.code === "Lang2" || e.keyCode === 0x1a || e.key === "Eisu" || e.key === "NonConvert") return "alnum";
  if (e.code === "Lang1" || e.keyCode === 0x16 || e.key === "KanaMode" || e.key === "Convert") return "kana";
  return null;
}

/** 右ボタンか。macOS の Control+クリックも右クリックとして扱う */
function isRightButton(e: React.PointerEvent) {
  return e.button === 2 || (IS_MAC && e.button === 0 && e.ctrlKey);
}

/** 改行だけの文節か。改行は空白と同じく、それだけで1文節になる */
function isLineBreak(seg: Segment) {
  return seg.reading === "\n";
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

/** 候補の行の文節の下に出す候補一覧。下に入りきらなければ上に出す */
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

/** ピンを文節の箱の上下から縮める長さ（styles.css の .pin と合わせる） */
const PIN_INSET_PX = 4;

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
