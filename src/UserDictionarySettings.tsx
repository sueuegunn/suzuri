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

/** 設定画面の「ユーザー辞書」タブ。語を登録すると、変換で1語として出やすくなる */
export default function UserDictionarySettings() {
  const [words, setWords] = useState<UserWord[] | null>(null);
  const [reading, setReading] = useState("");
  const [surface, setSurface] = useState("");
  const [pos, setPos] = useState<UserPos>("noun");
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);

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
        <select value={pos} onChange={(e) => setPos(e.target.value as UserPos)} aria-label="品詞">
          {POS_OPTIONS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.title}
            </option>
          ))}
        </select>
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
          .map((w) => (
            <div className="user-word" key={`${w.reading}\t${w.surface}\t${w.pos}`}>
              <span className="user-word-surface">{w.surface}</span>
              <span className="user-word-reading">{w.reading}</span>
              <span className="user-word-pos">{posTitle(w.pos)}</span>
              <button onClick={() => remove(w)} aria-label={`${w.surface} を消す`}>
                消す
              </button>
            </div>
          ))}
      </div>
    </div>
  );
}
