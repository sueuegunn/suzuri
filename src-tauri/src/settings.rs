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

/// オーバーレイの見た目。どちらも百分率
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Appearance {
    /// 文字の大きさ（標準を 100 とする）
    pub text_scale: u32,
    /// 背景の不透明度。下げると、うしろの画面を透かして見ながら打てる
    pub opacity: u32,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            text_scale: 100,
            opacity: 86,
        }
    }
}

impl Appearance {
    pub const TEXT_SCALE: std::ops::RangeInclusive<u32> = 70..=200;
    /// 低すぎると、うしろの文字と重なって読めなくなる
    pub const OPACITY: std::ops::RangeInclusive<u32> = 30..=100;

    /// 手で書き換えた設定ファイルでも画面が壊れないよう、範囲に収める
    pub fn clamped(self) -> Self {
        Appearance {
            text_scale: self.text_scale.clamp(*Self::TEXT_SCALE.start(), *Self::TEXT_SCALE.end()),
            opacity: self.opacity.clamp(*Self::OPACITY.start(), *Self::OPACITY.end()),
        }
    }
}

/// 項目が欠けていても既定値で補い、古い設定ファイルも読めるようにする
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SettingsFile {
    pub shortcuts: Bindings,
    pub letter_case: LetterCase,
    pub appearance: Appearance,
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
        file.appearance = file.appearance.clamped();
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
        assert!(file.appearance == Appearance::default());
    }

    #[test]
    fn clamps_appearance_out_of_range() {
        let a = Appearance {
            text_scale: 1000,
            opacity: 0,
        }
        .clamped();
        assert_eq!((a.text_scale, a.opacity), (200, 30));
    }
}
