//! 呼び出しのショートカットの設定と登録。
//! 設定は settings.json に保存し、変えたらその場で登録し直す。

use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut};

use crate::overlay::Mode;

/// 既定は Alt（macOS は Option）+J。J は標準のメニューでほぼ使われず、ホームポジションで押せるため
const DEFAULT_NEW: &str = "Alt+KeyJ";
const DEFAULT_PREVIOUS: &str = "Alt+Shift+KeyJ";

/// ショートカットの組み合わせ。文字列は「修飾キー+…+キーのコード」（例: `Alt+Shift+KeyJ`）
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bindings {
    /// 新しく入力する
    pub new: String,
    /// 前回の状態を直す
    pub previous: String,
}

impl Default for Bindings {
    fn default() -> Self {
        Bindings {
            new: DEFAULT_NEW.into(),
            previous: DEFAULT_PREVIOUS.into(),
        }
    }
}

impl Bindings {
    fn parse(&self) -> Result<[Shortcut; 2], String> {
        Ok([parse(&self.new)?, parse(&self.previous)?])
    }
}

#[derive(Serialize, Deserialize, Default)]
struct SettingsFile {
    shortcuts: Bindings,
}

pub struct Shortcuts {
    path: PathBuf,
    bindings: Mutex<Bindings>,
    /// 設定画面でキーを記録している間は登録を外し、押したキーが設定画面に届くようにする
    suspended: Mutex<bool>,
}

fn parse(accelerator: &str) -> Result<Shortcut, String> {
    let shortcut = Shortcut::from_str(accelerator).map_err(|_| format!("{accelerator} は使えないキーです"))?;
    // Shift だけでは、普段の打鍵（大文字や記号）を奪ってしまう
    if !shortcut.mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER) {
        return Err("Ctrl・Alt・Cmd（Windows キー）のどれかと組み合わせてください".into());
    }
    Ok(shortcut)
}

impl Shortcuts {
    /// 設定がない・壊れている・使えない組み合わせが入っているときは既定に戻す。起動は失敗させない
    pub fn load(path: PathBuf) -> Self {
        let bindings = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<SettingsFile>(&text).ok())
            .map(|f| f.shortcuts)
            .filter(|b| b.parse().is_ok() && b.new != b.previous)
            .unwrap_or_default();
        Shortcuts {
            path,
            bindings: Mutex::new(bindings),
            suspended: Mutex::new(false),
        }
    }

    pub fn bindings(&self) -> Bindings {
        self.bindings.lock().unwrap().clone()
    }

    /// 押されたショートカットがどちらの呼び出しか
    pub fn mode_of(&self, shortcut: &Shortcut) -> Option<Mode> {
        let [new, previous] = self.bindings().parse().ok()?;
        if *shortcut == new {
            Some(Mode::New)
        } else if *shortcut == previous {
            Some(Mode::Previous)
        } else {
            None
        }
    }

    /// 今の設定を登録する。登録できなかったものの説明を返す
    pub fn register(&self, app: &AppHandle) -> Vec<String> {
        let Ok(shortcuts) = self.bindings().parse() else {
            return vec!["ショートカットの設定が読めません".into()];
        };
        shortcuts
            .into_iter()
            .filter_map(|s| app.global_shortcut().register(s).err().map(|e| format!("{s}: {e}")))
            .collect()
    }

    pub fn suspend(&self, app: &AppHandle) {
        *self.suspended.lock().unwrap() = true;
        let _ = app.global_shortcut().unregister_all();
    }

    pub fn resume(&self, app: &AppHandle) {
        let mut suspended = self.suspended.lock().unwrap();
        if *suspended {
            *suspended = false;
            for e in self.register(app) {
                eprintln!("ショートカットを登録できません: {e}");
            }
        }
    }

    /// 組み合わせを変えて保存する。ほかのアプリが使っているなどで登録できなければ、元に戻してエラーを返す
    pub fn set(&self, app: &AppHandle, next: Bindings) -> Result<Bindings, String> {
        let shortcuts = next.parse()?;
        if next.new == next.previous {
            return Err("2つに同じ組み合わせは使えません".into());
        }
        let global = app.global_shortcut();
        let _ = global.unregister_all();
        let failed = shortcuts.iter().any(|&s| global.register(s).is_err());
        let _ = global.unregister_all();
        if failed {
            if !*self.suspended.lock().unwrap() {
                self.register(app);
            }
            return Err("この組み合わせは、ほかのアプリか OS が使っているため登録できません".into());
        }
        *self.bindings.lock().unwrap() = next.clone();
        if !*self.suspended.lock().unwrap() {
            self.register(app);
        }
        self.save(&next).map_err(|e| format!("設定を保存できません: {e}"))?;
        Ok(next)
    }

    fn save(&self, bindings: &Bindings) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&SettingsFile {
            shortcuts: bindings.clone(),
        })?;
        // 書きかけで落ちても元の設定を壊さないよう、一時ファイルから置き換える
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_global_shortcut::Code;

    #[test]
    fn parses_accelerators_from_the_settings_screen() {
        assert_eq!(parse(DEFAULT_NEW).unwrap(), Shortcut::new(Some(Modifiers::ALT), Code::KeyJ));
        assert_eq!(
            parse("Ctrl+Shift+Semicolon").unwrap(),
            Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Semicolon)
        );
        assert_eq!(parse("Super+Digit1").unwrap(), Shortcut::new(Some(Modifiers::SUPER), Code::Digit1));
    }

    #[test]
    fn rejects_shortcuts_without_ctrl_alt_or_cmd() {
        assert!(parse("Shift+KeyJ").is_err());
        assert!(parse("KeyJ").is_err());
        assert!(parse("Alt+NoSuchKey").is_err());
    }
}
