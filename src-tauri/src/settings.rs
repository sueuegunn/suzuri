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

/// 画面の配色。トレイメニューの「外観」で選ぶ
#[derive(Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    /// OS の設定に合わせる
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    /// ウィンドウの枠やメニューに渡す値。None は OS に合わせる
    pub fn to_tauri(self) -> Option<tauri::Theme> {
        match self {
            Theme::System => None,
            Theme::Light => Some(tauri::Theme::Light),
            Theme::Dark => Some(tauri::Theme::Dark),
        }
    }
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
            opacity: 85,
        }
    }
}

impl Appearance {
    pub const TEXT_SCALE: std::ops::RangeInclusive<u32> = 70..=200;
    /// 低すぎると、うしろの文字と重なって読めなくなる
    pub const OPACITY: std::ops::RangeInclusive<u32> = 30..=100;
    /// 不透明度は 5% 刻みで選ぶ
    pub const OPACITY_STEP: u32 = 5;

    /// 手で書き換えた設定ファイルでも画面が壊れないよう、範囲と刻みに収める
    pub fn clamped(self) -> Self {
        let step = Self::OPACITY_STEP;
        let opacity = (self.opacity + step / 2) / step * step;
        Appearance {
            text_scale: self.text_scale.clamp(*Self::TEXT_SCALE.start(), *Self::TEXT_SCALE.end()),
            opacity: opacity.clamp(*Self::OPACITY.start(), *Self::OPACITY.end()),
        }
    }
}

/// 手で動かしたオーバーレイの位置。モニターごとに、そのモニターの左上からの物理ピクセルで持つ
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowPosition {
    /// どのモニターか。名前と配置と解像度から作る（`overlay::monitor_key`）
    pub monitor: String,
    pub x: i32,
    pub y: i32,
}

/// 手で変えたオーバーレイの大きさ。拡大率の違うモニターでも同じ見た目になるよう論理ピクセルで持つ
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: f64,
    pub height: f64,
}

/// 項目が欠けていても既定値で補い、古い設定ファイルも読めるようにする
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SettingsFile {
    pub shortcuts: Bindings,
    pub letter_case: LetterCase,
    pub appearance: Appearance,
    pub theme: Theme,
    /// 手で動かした位置をモニターごとに持つ。マウスのあるモニターの分がなければ、そのモニターの中央に出す。
    /// 以前の、モニターを区別しない `windowPosition` は読まずに捨てる
    pub window_positions: Vec<WindowPosition>,
    /// 手で大きさを変えていなければ None。そのときは既定の大きさで出す
    pub window_size: Option<WindowSize>,
}

impl SettingsFile {
    /// モニター `monitor` で覚えた位置（モニターの左上から）
    pub fn window_position(&self, monitor: &str) -> Option<(i32, i32)> {
        self.window_positions
            .iter()
            .find(|p| p.monitor == monitor)
            .map(|p| (p.x, p.y))
    }

    /// モニター `monitor` で覚えた位置を置き換える。None なら忘れる
    pub fn set_window_position(&mut self, monitor: &str, position: Option<(i32, i32)>) {
        self.window_positions.retain(|p| p.monitor != monitor);
        if let Some((x, y)) = position {
            self.window_positions.push(WindowPosition {
                monitor: monitor.to_string(),
                x,
                y,
            });
        }
    }
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
    fn remembers_window_position_per_monitor() {
        // 以前の、モニターを区別しない位置は捨てる
        let mut file: SettingsFile = serde_json::from_str(r#"{"windowPosition":{"x":10,"y":20}}"#).unwrap();
        assert!(file.window_positions.is_empty());
        file.set_window_position("left", Some((10, 20)));
        file.set_window_position("right", Some((30, 40)));
        file.set_window_position("left", Some((50, 60)));
        assert_eq!(file.window_position("left"), Some((50, 60)));
        assert_eq!(file.window_position("right"), Some((30, 40)));
        file.set_window_position("right", None);
        assert_eq!(file.window_position("right"), None);
        assert_eq!(file.window_positions.len(), 1);
    }

    #[test]
    fn clamps_appearance_out_of_range() {
        let a = Appearance {
            text_scale: 1000,
            opacity: 0,
        }
        .clamped();
        assert_eq!((a.text_scale, a.opacity), (200, 30));
        // 刻みから外れた不透明度（以前の既定の 86 など）は、近い 5% 刻みに丸める
        let a = Appearance {
            text_scale: 100,
            opacity: 86,
        }
        .clamped();
        assert_eq!(a.opacity, 85);
    }
}
