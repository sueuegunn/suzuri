//! スズリのかな漢字変換エンジン。
//!
//! Tauri には依存させず、UI なしでテストとベンチマークを回せるようにしている。

mod convert;
pub mod dict;
pub mod history;
pub mod kana;
pub mod user_dict;

pub use convert::{Conversion, Engine, Segment};
pub use history::{Committed, HistoryError, UserHistory};
pub use user_dict::{UserDictError, UserDictionary, UserPos, UserWord};

/// ユーザーごとのデータ。変換のたびにエンジンへ渡す
#[derive(Default)]
pub struct UserData {
    /// 選んだ候補の学習
    pub history: UserHistory,
    /// ユーザーが登録した語
    pub dictionary: UserDictionary,
}
