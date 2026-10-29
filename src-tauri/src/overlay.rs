//! オーバーレイの表示と非表示。
//! ウィンドウは起動時に1つだけ作り、表示を切り替えて使い回す（毎回作ると WebView の起動待ちが出る）。
//! 手で動かした位置と大きさは、隠すときに覚えて次から同じところに出す。位置はモニターごとに覚える。

use std::thread;

use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, Monitor, PhysicalPosition};
use tauri_plugin_global_shortcut::{Shortcut, ShortcutEvent, ShortcutState};

use crate::settings::WindowSize;
use crate::{AppState, platform};

const LABEL: &str = "overlay";
/// 手で大きさを変えていないときの大きさ（論理ピクセル）。
/// 見えるパネルは 900×560 で、まわりに大きさを変える掴みどころ用の透明な余白が 8px ずつある
const DEFAULT_SIZE: LogicalSize<f64> = LogicalSize::new(916.0, 576.0);
/// 置いた位置と隠すときの位置がこれだけずれていなければ、動かしていないとみなす。
/// macOS は論理ピクセルで置くため、拡大率が 2 のモニターでは物理ピクセルで 1 ずれて読めることがある
const MOVE_TOLERANCE: i32 = 2;
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

/// 覚えた大きさで、マウスカーソルがあるモニターに置く。そのモニターで位置を覚えていない、
/// または覚えた位置がどのモニターにも入らないときは、そのモニターの中央に置く
fn place(app: &AppHandle, window: &tauri::WebviewWindow) {
    let state = app.state::<AppState>();
    let saved = state.settings.get();
    let size = saved
        .window_size
        .map_or(DEFAULT_SIZE, |s| LogicalSize::new(s.width, s.height));
    let _ = window.set_size(size);
    let Some(monitor) = cursor_monitor(app, window) else {
        return;
    };
    let origin = monitor.position();
    let position = saved
        .window_position(&monitor_key(&monitor))
        .map(|(x, y)| PhysicalPosition::new(origin.x + x, origin.y + y))
        .filter(|p| on_screen(app, *p))
        .unwrap_or_else(|| centered_on(&monitor, size));
    let _ = window.set_position(position);
    *state.placement.lock().unwrap() = Some(Placement { position, size });
}

/// 位置を覚えるときにモニターを見分ける名前。同じ機種のモニターを並べても分かれるよう配置を、
/// 解像度を変えたあとに覚えた位置で画面からはみ出さないよう解像度を含める
fn monitor_key(monitor: &Monitor) -> String {
    let (p, s) = (monitor.position(), monitor.size());
    let name = monitor.name().map_or("", |n| n.as_str());
    format!("{name} {},{} {}x{}", p.x, p.y, s.width, s.height)
}

fn on_screen(app: &AppHandle, p: PhysicalPosition<i32>) -> bool {
    app.monitor_from_point((p.x + VISIBLE_MARGIN).into(), (p.y + VISIBLE_MARGIN).into())
        .ok()
        .flatten()
        .is_some()
}

/// マウスカーソルがあるモニター。分からなければメインのモニター
fn cursor_monitor(app: &AppHandle, window: &tauri::WebviewWindow) -> Option<Monitor> {
    app.cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten())
}

fn centered_on(monitor: &Monitor, size: LogicalSize<f64>) -> PhysicalPosition<i32> {
    let size = size.to_physical::<i32>(monitor.scale_factor());
    let origin = monitor.position();
    let area = monitor.size();
    PhysicalPosition::new(
        origin.x + (area.width as i32 - size.width) / 2,
        origin.y + (area.height as i32 - size.height) / 2,
    )
}

/// 表示中に手で動かされていたら、その位置と大きさを覚える。位置は、隠すときにウィンドウがいるモニターの分として覚える
fn remember_geometry(app: &AppHandle, window: &tauri::WebviewWindow) {
    let Some(placed) = app.state::<AppState>().placement.lock().unwrap().take() else {
        return;
    };
    let (Ok(position), Ok(size), Ok(scale)) = (window.outer_position(), window.inner_size(), window.scale_factor())
    else {
        return;
    };
    let size = size.to_logical::<f64>(scale);
    let moved = (position.x - placed.position.x).abs() > MOVE_TOLERANCE
        || (position.y - placed.position.y).abs() > MOVE_TOLERANCE;
    // 論理ピクセルに直すときの丸めの差は、動かしたとみなさない
    let resized = (size.width - placed.size.width).abs() > 1.0 || (size.height - placed.size.height).abs() > 1.0;
    if !moved && !resized {
        return;
    }
    // 別のモニターへ動かしたなら、そのモニターの位置として覚える
    let position = window.current_monitor().ok().flatten().map(|m| {
        let origin = m.position();
        (monitor_key(&m), (position.x - origin.x, position.y - origin.y))
    });
    let size = WindowSize {
        width: size.width.round(),
        height: size.height.round(),
    };
    save_in_background(app, move |f| {
        if let (true, Some((monitor, offset))) = (moved, &position) {
            f.set_window_position(monitor, Some(*offset));
        }
        if resized {
            f.window_size = Some(size);
        }
    });
}

/// 大きさはそのままで、今いるモニターの中央に動かし、そのモニターで覚えた位置を忘れる
pub fn center(app: &AppHandle) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let state = app.state::<AppState>();
    let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) else {
        return;
    };
    let size = size.to_logical::<f64>(scale);
    // ダブルクリックしたマウスはオーバーレイの上にあるので、オーバーレイがいるモニターの中央になる
    let Some(monitor) = cursor_monitor(app, &window) else {
        return;
    };
    let position = centered_on(&monitor, size);
    let _ = window.set_position(position);
    // 隠すときに「手で動かした」と判断しないよう、置いた位置を更新しておく
    if let Some(placed) = state.placement.lock().unwrap().as_mut() {
        placed.position = position;
    }
    let key = monitor_key(&monitor);
    save_in_background(app, move |f| f.set_window_position(&key, None));
}

/// 確定や表示の処理を待たせないよう、設定の保存は別スレッドで行う
fn save_in_background(app: &AppHandle, change: impl FnOnce(&mut crate::settings::SettingsFile) + Send + 'static) {
    let app = app.clone();
    thread::spawn(move || {
        if let Err(e) = app.state::<AppState>().settings.update(change) {
            eprintln!("{e}");
        }
    });
}
