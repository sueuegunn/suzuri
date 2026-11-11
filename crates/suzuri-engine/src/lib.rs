//! スズリのかな漢字変換エンジン。
//!
//! Tauri には依存させず、UI なしでテストとベンチマークを回せるようにしている。

pub mod context;
mod convert;
pub mod dict;
pub mod history;
pub mod kana;
pub mod learned;
pub mod user_dict;

pub use convert::{CommitSegment, Conversion, Engine, Segment};
pub use history::{Committed, HistoryError, UserHistory};
pub use user_dict::{UserDictError, UserDictionary, UserPos, UserWord};

/// ユーザーごとのデータ。変換のたびにエンジンへ渡す
#[derive(Default)]
pub struct UserData {
    /// 選んだ候補の学習
    pub history: UserHistory,
    /// ユーザーが登録した語
    pub dictionary: UserDictionary,
    /// 手で区切った文節（区切りの学習）
    pub learned: learned::LearnedWords,
    /// 前に出た自立語ごとに選んだ候補（文脈つきの学習）
    pub context: context::ContextHistory,
}
