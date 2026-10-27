//! スズリのかな漢字変換エンジン。
//!
//! Tauri には依存させず、UI なしでテストとベンチマークを回せるようにしている。

mod convert;
pub mod dict;
pub mod history;
pub mod kana;

pub use convert::{Conversion, Engine, Segment};
pub use history::{Committed, HistoryError, UserHistory};
