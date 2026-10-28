//! 設定ファイル（settings.json）の読み書き。
//! 変換中には読み書きせず、設定を変えたときだけ保存する。

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::shortcuts::Bindings;

/// Shift を押しながら打った英字を、大文字と小文字のどちらで入れるか
#[derive(Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LetterCase {
    #[default]
    Upper,
    Lower,
}

/// 項目が欠けていても既定値で補い、古い設定ファイルも読めるようにする
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SettingsFile {
    pub shortcuts: Bindings,
    pub letter_case: LetterCase,
}

pub struct Settings {
    path: PathBuf,
    file: Mutex<SettingsFile>,
}

impl Settings {
    /// 設定がない・壊れているときは既定に戻す。起動は失敗させない
    pub fn load(path: PathBuf) -> Self {
        let mut file = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<SettingsFile>(&text).ok())
            .unwrap_or_default();
        if !file.shortcuts.is_valid() {
            file.shortcuts = Bindings::default();
        }
        Settings {
            path,
            file: Mutex::new(file),
        }
    }

    pub fn get(&self) -> SettingsFile {
        self.file.lock().unwrap().clone()
    }

    /// 設定を変えて保存する
    pub fn update(&self, change: impl FnOnce(&mut SettingsFile)) -> Result<(), String> {
        let mut file = self.file.lock().unwrap();
        change(&mut file);
        self.save(&file).map_err(|e| format!("設定を保存できません: {e}"))
    }

    fn save(&self, file: &SettingsFile) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(file)?;
        // 書きかけで落ちても元の設定を壊さないよう、一時ファイルから置き換える
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_old_settings_without_letter_case() {
        let file: SettingsFile =
            serde_json::from_str(r#"{"shortcuts":{"new":"Ctrl+KeyK","previous":"Ctrl+Shift+KeyK"}}"#).unwrap();
        assert_eq!(file.shortcuts.new, "Ctrl+KeyK");
        assert!(file.letter_case == LetterCase::Upper);
        let file: SettingsFile = serde_json::from_str(r#"{"letterCase":"lower"}"#).unwrap();
        assert!(file.letter_case == LetterCase::Lower);
        assert!(file.shortcuts == Bindings::default());
    }
}
