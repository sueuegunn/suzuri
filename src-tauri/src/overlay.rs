//! オーバーレイの表示と非表示。
//! ウィンドウは起動時に1つだけ作り、表示を切り替えて使い回す（毎回作ると WebView の起動待ちが出る）。

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutEvent, ShortcutState};

const LABEL: &str = "overlay";

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// 新しく入力する
    New,
    /// 前回の状態を編集する
    Previous,
}

/// [新規入力, 前回の編集]。Alt（macOS は Option）+J。
/// J は標準のメニューでほぼ使われず、ホームポジションで押せるため選んだ
pub fn shortcuts() -> [Shortcut; 2] {
    [
        Shortcut::new(Some(Modifiers::ALT), Code::KeyJ),
        Shortcut::new(Some(Modifiers::ALT | Modifiers::SHIFT), Code::KeyJ),
    ]
}

pub fn on_shortcut(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() != ShortcutState::Pressed {
        return;
    }
    let [new, previous] = shortcuts();
    if *shortcut == new {
        show(app, Mode::New);
    } else if *shortcut == previous {
        show(app, Mode::Previous);
    }
}

pub fn show(app: &AppHandle, mode: Mode) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    center_on_cursor_monitor(app, &window);
    // 表示する前に状態を切り替えさせ、前の内容が一瞬見えないようにする
    let _ = window.emit("summon", mode);
    let _ = window.show();
    let _ = window.set_focus();
}

pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
}

/// マウスカーソルがあるモニターの中央に置く
fn center_on_cursor_monitor(app: &AppHandle, window: &tauri::WebviewWindow) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten());
    let (Some(monitor), Ok(size)) = (monitor, window.outer_size()) else {
        return;
    };
    let origin = monitor.position();
    let area = monitor.size();
    let x = origin.x + (area.width as i32 - size.width as i32) / 2;
    let y = origin.y + (area.height as i32 - size.height as i32) / 2;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}
