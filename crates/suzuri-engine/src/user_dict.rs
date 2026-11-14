//! ユーザー辞書。ユーザーが登録した語を、システム辞書の語と同じくラティスに足す（docs/design.md）。
//!
//! 変換中は読みで引くハッシュ表だけを見る。ファイルの読み書きは呼び出し側が変換の外で行う。

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

use crate::kana::katakana_to_hiragana;

const HEADER: &str = "suzuri-user-dictionary 1";
/// 読みの長さの上限（文字数）。ラティスを作るとき、位置ごとにこの長さまで引く
pub const MAX_READING_CHARS: usize = 32;
/// 表記の長さの上限（文字数）
pub const MAX_SURFACE_CHARS: usize = 64;

/// 登録した語に付ける品詞。数を絞り、登録する画面を簡単にする
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserPos {
    /// 名詞（一般）
    Noun = 0,
    /// 人名
    Person = 1,
    /// 地名
    Place = 2,
    /// 組織名
    Organization = 3,
}

impl UserPos {
    /// 辞書のバイナリに品詞IDを並べる順
    pub const ALL: [UserPos; 4] = [UserPos::Noun, UserPos::Person, UserPos::Place, UserPos::Organization];

    /// 保存と画面とのやりとりに使う名前
    pub fn name(self) -> &'static str {
        match self {
            UserPos::Noun => "noun",
            UserPos::Person => "person",
            UserPos::Place => "place",
            UserPos::Organization => "organization",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        UserPos::ALL.into_iter().find(|p| p.name() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserWord {
    /// ひらがな（と「ー」・英数字）の読み
    pub reading: String,
    pub surface: String,
    pub pos: UserPos,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserDictError {
    /// 登録しようとした語が正しくない。画面にそのまま出す
    Invalid(String),
    /// 保存したファイルが壊れている
    Broken(String),
}

impl fmt::Display for UserDictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UserDictError::Invalid(msg) => write!(f, "{msg}"),
            UserDictError::Broken(msg) => write!(f, "ユーザー辞書が壊れています: {msg}"),
        }
    }
}

impl std::error::Error for UserDictError {}

#[derive(Default)]
pub struct UserDictionary {
    /// 登録した順
    words: Vec<UserWord>,
    /// 読み → words の番号
    by_reading: HashMap<String, Vec<u32>>,
    /// 登録した読みのうち一番長いものの文字数。ラティスでこれより長くは引かない
    max_chars: usize,
}

impl UserDictionary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn words(&self) -> &[UserWord] {
        &self.words
    }

    pub fn word(&self, index: u32) -> &UserWord {
        &self.words[index as usize]
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// 登録した読みのうち一番長いものの文字数
    pub fn max_chars(&self) -> usize {
        self.max_chars
    }

    /// 読みが完全に一致する語の番号。英字の大文字・小文字は区別しない（「npm」で登録した語を Shift で打った「NPM」でも引く）。
    /// 小文字にそろえた読みを渡せば、文字列を作り直さない
    pub fn lookup(&self, reading: &str) -> &[u32] {
        let key = if reading.bytes().any(|b| b.is_ascii_uppercase()) {
            Cow::Owned(reading.to_ascii_lowercase())
        } else {
            Cow::Borrowed(reading)
        };
        self.by_reading.get(key.as_ref()).map_or(&[], Vec::as_slice)
    }

    /// 語を登録する。読みのカタカナはひらがなにそろえる。返すのは登録した語
    pub fn add(&mut self, reading: &str, surface: &str, pos: UserPos) -> Result<UserWord, UserDictError> {
        let word = UserWord {
            reading: normalize_reading(reading)?,
            surface: normalize_surface(surface)?,
            pos,
        };
        if self.words.contains(&word) {
            return Err(UserDictError::Invalid("同じ語がすでに登録されています".into()));
        }
        self.words.push(word.clone());
        self.reindex();
        Ok(word)
    }

    /// 語を消す。登録されていなければ false
    pub fn remove(&mut self, word: &UserWord) -> bool {
        let Some(i) = self.words.iter().position(|w| w == word) else {
            return false;
        };
        self.words.remove(i);
        self.reindex();
        true
    }

    /// 登録した語 `old` を書き換える。一覧の位置（登録した順）は保つ。返すのは書き換えた語。
    /// ほかの登録済みの語と同じになるとき、`old` が見つからないときは失敗する
    pub fn replace(&mut self, old: &UserWord, reading: &str, surface: &str, pos: UserPos) -> Result<UserWord, UserDictError> {
        let word = UserWord {
            reading: normalize_reading(reading)?,
            surface: normalize_surface(surface)?,
            pos,
        };
        let Some(i) = self.words.iter().position(|w| w == old) else {
            return Err(UserDictError::Invalid("書き換える語が見つかりません".into()));
        };
        if self.words.iter().enumerate().any(|(j, w)| j != i && *w == word) {
            return Err(UserDictError::Invalid("同じ語がすでに登録されています".into()));
        }
        self.words[i] = word.clone();
        self.reindex();
        Ok(word)
    }

    /// 登録や削除は設定画面から人の手で行うので、そのたびに作り直しても間に合う
    fn reindex(&mut self) {
        self.by_reading.clear();
        for (i, w) in self.words.iter().enumerate() {
            self.by_reading.entry(w.reading.to_ascii_lowercase()).or_default().push(i as u32);
        }
        self.max_chars = self.words.iter().map(|w| w.reading.chars().count()).max().unwrap_or(0);
    }

    /// 保存用のテキストにする。1行に「読み TAB 表記 TAB 品詞」
    pub fn serialize(&self) -> String {
        let mut out = String::from(HEADER);
        out.push('\n');
        for w in &self.words {
            out.push_str(&format!("{}\t{}\t{}\n", w.reading, w.surface, w.pos.name()));
        }
        out
    }

    pub fn deserialize(text: &str) -> Result<Self, UserDictError> {
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return Err(UserDictError::Broken("見出し行が違います".into()));
        }
        let mut dict = UserDictionary::new();
        for (i, line) in lines.enumerate() {
            let broken = |msg: &str| UserDictError::Broken(format!("{}行目{msg}", i + 2));
            let mut cols = line.split('\t');
            let (Some(r), Some(s), Some(p), None) = (cols.next(), cols.next(), cols.next(), cols.next()) else {
                return Err(broken("の形式が違います"));
            };
            let pos = UserPos::from_name(p).ok_or_else(|| broken("の品詞が違います"))?;
            let word = UserWord {
                reading: normalize_reading(r).map_err(|e| broken(&format!(": {e}")))?,
                surface: normalize_surface(s).map_err(|e| broken(&format!(": {e}")))?,
                pos,
            };
            // 手で書き換えて重なった行は1つにする
            if !dict.words.contains(&word) {
                dict.words.push(word);
            }
        }
        dict.reindex();
        Ok(dict)
    }
}

/// 読みをそろえて確かめる。前後の空白を除き、カタカナをひらがなに、全角英数字を半角にする（設定画面で IME が
/// 全角のまま打っても登録できるように）。打った読みに出てくる文字（ひらがな・「ー」・英数字）だけを受け付ける
fn normalize_reading(reading: &str) -> Result<String, UserDictError> {
    let reading: String = katakana_to_hiragana(reading.trim())
        .chars()
        .map(|c| match c {
            'Ａ'..='Ｚ' | 'ａ'..='ｚ' | '０'..='９' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .collect();
    if reading.is_empty() {
        return Err(UserDictError::Invalid("読みを入れてください".into()));
    }
    if reading.chars().count() > MAX_READING_CHARS {
        return Err(UserDictError::Invalid(format!("読みは {MAX_READING_CHARS} 文字までです")));
    }
    let ok = |c: char| matches!(c, 'ぁ'..='ゖ' | 'ー') || c.is_ascii_alphanumeric();
    if !reading.chars().all(ok) {
        return Err(UserDictError::Invalid("読みはひらがなと英数字で入れてください".into()));
    }
    Ok(reading)
}

fn normalize_surface(surface: &str) -> Result<String, UserDictError> {
    let surface = surface.trim();
    if surface.is_empty() {
        return Err(UserDictError::Invalid("表記を入れてください".into()));
    }
    if surface.chars().count() > MAX_SURFACE_CHARS {
        return Err(UserDictError::Invalid(format!("表記は {MAX_SURFACE_CHARS} 文字までです")));
    }
    if surface.chars().any(char::is_control) {
        return Err(UserDictError::Invalid("表記に改行やタブは入れられません".into()));
    }
    Ok(surface.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_and_looks_up_by_reading() {
        let mut d = UserDictionary::new();
        let w = d.add("すずり", "スズリ", UserPos::Noun).unwrap();
        d.add("すずり", "硯", UserPos::Noun).unwrap();
        assert_eq!(d.lookup("すずり").len(), 2);
        assert!(d.lookup("すず").is_empty());
        assert_eq!(d.max_chars(), 3);
        assert!(d.remove(&w));
        assert_eq!(d.lookup("すずり").len(), 1);
        assert!(!d.remove(&w));
    }

    #[test]
    fn normalizes_and_rejects_readings() {
        let mut d = UserDictionary::new();
        // カタカナの読みはひらがなにそろえる
        assert_eq!(d.add(" スズリ ", "スズリ", UserPos::Organization).unwrap().reading, "すずり");
        assert!(d.add("すずり", "スズリ", UserPos::Organization).is_err());
        assert!(d.add("", "空", UserPos::Noun).is_err());
        assert!(d.add("漢字", "漢字", UserPos::Noun).is_err());
        assert!(d.add("よみ", "", UserPos::Noun).is_err());
        assert!(d.add("よみ", "改\n行", UserPos::Noun).is_err());
        assert!(d.add("npm", "npm", UserPos::Noun).is_ok());
    }

    #[test]
    fn accepts_alphanumeric_readings() {
        let mut d = UserDictionary::new();
        // 全角は半角にそろえ、大文字・小文字は区別せずに引く
        assert_eq!(d.add("ｎｐｍ", "npm", UserPos::Noun).unwrap().reading, "npm");
        assert_eq!(d.lookup("NPM").len(), 1);
        assert_eq!(d.lookup("npm").len(), 1);
        assert_eq!(d.add("Tしゃつ", "Tシャツ", UserPos::Noun).unwrap().reading, "Tしゃつ");
        assert_eq!(d.lookup("tしゃつ").len(), 1);
        assert_eq!(d.add("mp３", "MP3", UserPos::Noun).unwrap().reading, "mp3");
        assert!(d.add("c++", "C++", UserPos::Noun).is_err());
    }

    #[test]
    fn replaces_in_place() {
        let mut d = UserDictionary::new();
        let a = d.add("すずり", "スズリ", UserPos::Noun).unwrap();
        let b = d.add("たなか", "田中", UserPos::Person).unwrap();
        // 位置を保ち、読みの表も引き直す
        let a2 = d.replace(&a, "スズリン", "スズリン", UserPos::Organization).unwrap();
        assert_eq!(a2.reading, "すずりん");
        assert_eq!(d.words()[0], a2);
        assert!(d.lookup("すずり").is_empty());
        assert_eq!(d.lookup("すずりん").len(), 1);
        assert_eq!(d.max_chars(), 4);
        // 変えないまま保存してもよい
        assert!(d.replace(&b, "たなか", "田中", UserPos::Person).is_ok());
        // ほかの語と重なる・元の語がない・読みが正しくないときは変えない
        assert!(d.replace(&b, "すずりん", "スズリン", UserPos::Organization).is_err());
        assert!(d.replace(&a, "すずり", "硯", UserPos::Noun).is_err());
        assert!(d.replace(&a2, "漢字", "漢字", UserPos::Noun).is_err());
        assert_eq!(d.words(), &[a2, b]);
    }

    #[test]
    fn roundtrips_through_text() {
        let mut d = UserDictionary::new();
        d.add("すずり", "スズリ", UserPos::Organization).unwrap();
        d.add("たなか", "田中", UserPos::Person).unwrap();
        let restored = UserDictionary::deserialize(&d.serialize()).unwrap();
        assert_eq!(restored.words(), d.words());
        assert_eq!(restored.lookup("たなか").len(), 1);
    }

    #[test]
    fn rejects_broken_text() {
        assert!(UserDictionary::deserialize("").is_err());
        assert!(UserDictionary::deserialize("suzuri-user-dictionary 1\nすずり\tスズリ\n").is_err());
        assert!(UserDictionary::deserialize("suzuri-user-dictionary 1\nすずり\tスズリ\tverb\n").is_err());
    }
}
