//! オーバーレイの表示と非表示。
//! ウィンドウは起動時に1つだけ作り、表示を切り替えて使い回す（毎回作ると WebView の起動待ちが出る）。
//! 手で動かした位置と大きさは、隠すときに覚えて次から同じところに出す。

use std::thread;

use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition};
use tauri_plugin_global_shortcut::{Shortcut, ShortcutEvent, ShortcutState};

use crate::settings::WindowGeometry;
use crate::{AppState, platform};

const LABEL: &str = "overlay";
/// 手で大きさを変えていないときの大きさ（論理ピクセル）
const DEFAULT_SIZE: LogicalSize<f64> = LogicalSize::new(900.0, 560.0);
/// 覚えた位置に出すとき、ウィンドウの左上からこれだけ内側がどこかのモニターに入っていることを求める。
/// モニターを外したあとに、画面の外へ出てしまわないようにする
const VISIBLE_MARGIN: i32 = 80;

/// 表示するときに置いた位置と大きさ。隠すときに比べて、手で動かされたかを見る
#[derive(Clone, Copy)]
pub struct Placement {
    position: PhysicalPosition<i32>,
    size: LogicalSize<f64>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// 新しく入力する
    New,
    /// 前回の状態を編集する
    Previous,
}

pub fn on_shortcut(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() != ShortcutState::Pressed {
        return;
    }
    let state = app.state::<AppState>();
    let Some(mode) = state.shortcuts.mode_of(shortcut) else {
        return;
    };
    // オーバーレイが出ている間に押し直したときは、最初の戻り先を保つ
    let visible = app
        .get_webview_window(LABEL)
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    if !visible {
        *state.caller.lock().unwrap() = platform::caller();
    }
    show(app, mode);
}

pub fn show(app: &AppHandle, mode: Mode) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    platform::before_show(app);
    place(app, &window);
    // 表示する前に状態を切り替えさせ、前の内容が一瞬見えないようにする
    let _ = window.emit("summon", mode);
    let _ = window.show();
    let _ = window.set_focus();
}

pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        remember_geometry(app, &window);
        let _ = window.hide();
    }
}

/// 覚えた位置と大きさに置く。覚えていない、または覚えた位置がどのモニターにも入らないときは、
/// マウスカーソルがあるモニターの中央に置く
fn place(app: &AppHandle, window: &tauri::WebviewWindow) {
    let state = app.state::<AppState>();
    let saved = state.settings.get().window;
    let size = saved.map_or(DEFAULT_SIZE, |g| LogicalSize::new(g.width, g.height));
    let _ = window.set_size(size);
    let position = saved
        .map(|g| PhysicalPosition::new(g.x, g.y))
        .filter(|p| on_screen(app, *p))
        .or_else(|| centered_on_cursor_monitor(app, window, size));
    if let Some(position) = position {
        let _ = window.set_position(position);
        *state.placement.lock().unwrap() = Some(Placement { position, size });
    }
}

fn on_screen(app: &AppHandle, p: PhysicalPosition<i32>) -> bool {
    app.monitor_from_point((p.x + VISIBLE_MARGIN).into(), (p.y + VISIBLE_MARGIN).into())
        .ok()
        .flatten()
        .is_some()
}

fn centered_on_cursor_monitor(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    size: LogicalSize<f64>,
) -> Option<PhysicalPosition<i32>> {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten())?;
    let size = size.to_physical::<i32>(monitor.scale_factor());
    let origin = monitor.position();
    let area = monitor.size();
    Some(PhysicalPosition::new(
        origin.x + (area.width as i32 - size.width) / 2,
        origin.y + (area.height as i32 - size.height) / 2,
    ))
}

/// 表示中に手で動かされていたら、その位置と大きさを覚える。
/// 確定の処理を待たせないよう、保存は別スレッドで行う
fn remember_geometry(app: &AppHandle, window: &tauri::WebviewWindow) {
    let Some(placed) = app.state::<AppState>().placement.lock().unwrap().take() else {
        return;
    };
    let (Ok(position), Ok(size), Ok(scale)) = (window.outer_position(), window.inner_size(), window.scale_factor())
    else {
        return;
    };
    let size = size.to_logical::<f64>(scale);
    let moved = position != placed.position;
    // 論理ピクセルに直すときの丸めの差は、動かしたとみなさない
    let resized = (size.width - placed.size.width).abs() > 1.0 || (size.height - placed.size.height).abs() > 1.0;
    if !moved && !resized {
        return;
    }
    let geometry = WindowGeometry {
        x: position.x,
        y: position.y,
        width: size.width.round(),
        height: size.height.round(),
    };
    let app = app.clone();
    thread::spawn(move || {
        if let Err(e) = app.state::<AppState>().settings.update(|f| f.window = Some(geometry)) {
            eprintln!("{e}");
        }
    });
}
