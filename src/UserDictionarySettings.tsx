import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { userDictionary, type UserPos, type UserWord } from "./engine";

/** 品詞の選択肢。並びは画面に出す順 */
const POS_OPTIONS: { value: UserPos; title: string }[] = [
  { value: "noun", title: "名詞" },
  { value: "person", title: "人名" },
  { value: "place", title: "地名" },
  { value: "organization", title: "組織名" },
];

const posTitle = (pos: UserPos) => POS_OPTIONS.find((o) => o.value === pos)?.title ?? pos;

const wordKey = (w: UserWord) => `${w.reading}	${w.surface}	${w.pos}`;

function PosSelect({ value, onChange }: { value: UserPos; onChange: (pos: UserPos) => void }) {
  return (
    <select value={value} onChange={(e) => onChange(e.target.value as UserPos)} aria-label="品詞">
      {POS_OPTIONS.map((o) => (
        <option key={o.value} value={o.value}>
          {o.title}
        </option>
      ))}
    </select>
  );
}

/** 設定画面の「ユーザー辞書」タブ。語を登録すると、変換で1語として出やすくなる */
export default function UserDictionarySettings() {
  const [words, setWords] = useState<UserWord[] | null>(null);
  const [reading, setReading] = useState("");
  const [surface, setSurface] = useState("");
  const [pos, setPos] = useState<UserPos>("noun");
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
  /** 書き換えている語と、書き換えかけの内容。一度に1語だけ */
  const [editing, setEditing] = useState<{ old: UserWord; word: UserWord } | null>(null);

  useEffect(() => {
    const load = () =>
      userDictionary
        .list()
        .then(setWords)
        .catch((e) => setMessage({ text: String(e), error: true }));
    load();
    // 設定画面を開いたまま、オーバーレイの下段から登録・取り消しをしたときも一覧を読み直す
    const unlisten = listen("user-dictionary", load);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const add = async () => {
    try {
      setWords(await userDictionary.add({ reading, surface, pos }));
      setMessage({ text: `「${surface.trim()}」を登録しました`, error: false });
      setReading("");
      setSurface("");
    } catch (e) {
      setMessage({ text: String(e), error: true });
    }
  };

  const save = async () => {
    if (!editing) return;
    try {
      setWords(await userDictionary.update(editing.old, editing.word));
      setMessage({ text: `「${editing.word.surface.trim()}」に書き換えました`, error: false });
      setEditing(null);
    } catch (e) {
      // 書き換えかけの内容は残し、直してもう一度保存できるようにする
      setMessage({ text: String(e), error: true });
    }
  };

  const remove = async (word: UserWord) => {
    try {
      setWords(await userDictionary.remove(word));
      setMessage({ text: `「${word.surface}」を消しました`, error: false });
    } catch (e) {
      setMessage({ text: String(e), error: true });
    }
  };

  return (
    <div className="user-dictionary">
      <form
        className="user-word-form"
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <input
          placeholder="読み（ひらがな）"
          value={reading}
          onChange={(e) => setReading(e.target.value)}
          aria-label="読み"
        />
        <input placeholder="表記" value={surface} onChange={(e) => setSurface(e.target.value)} aria-label="表記" />
        <PosSelect value={pos} onChange={setPos} />
        <button type="submit" disabled={words === null}>
          登録
        </button>
      </form>
      <p className={"settings-message" + (message?.error ? " error" : "")}>{message?.text ?? ""}</p>
      <div className="user-word-list">
        {words?.length === 0 && <p className="user-word-empty">まだ登録した語はありません</p>}
        {/* 新しく登録したものを上に出す */}
        {words
          ?.slice()
          .reverse()
          .map((w) =>
            editing && wordKey(editing.old) === wordKey(w) ? (
              <form
                className="user-word editing"
                key={wordKey(w)}
                onSubmit={(e) => {
                  e.preventDefault();
                  save();
                }}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setEditing(null);
                }}
              >
                <input
                  value={editing.word.surface}
                  onChange={(e) => setEditing({ ...editing, word: { ...editing.word, surface: e.target.value } })}
                  aria-label="表記"
                  autoFocus
                />
                <input
                  value={editing.word.reading}
                  onChange={(e) => setEditing({ ...editing, word: { ...editing.word, reading: e.target.value } })}
                  aria-label="読み"
                />
                <PosSelect
                  value={editing.word.pos}
                  onChange={(pos) => setEditing({ ...editing, word: { ...editing.word, pos } })}
                />
                <button type="submit">保存</button>
                <button type="button" onClick={() => setEditing(null)}>
                  やめる
                </button>
              </form>
            ) : (
              <div className="user-word" key={wordKey(w)}>
                <span className="user-word-surface">{w.surface}</span>
                <span className="user-word-reading">{w.reading}</span>
                <span className="user-word-pos">{posTitle(w.pos)}</span>
                <button onClick={() => setEditing({ old: w, word: { ...w } })} aria-label={`${w.surface} を書き換える`}>
                  編集
                </button>
                <button onClick={() => remove(w)} aria-label={`${w.surface} を消す`}>
                  消す
                </button>
              </div>
            ),
          )}
      </div>
    </div>
  );
}
