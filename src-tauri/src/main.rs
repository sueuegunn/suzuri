// リリースビルドでコンソールウィンドウを出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod history_store;
mod overlay;
mod platform;

use std::sync::{Arc, Mutex};

use suzuri_engine::Engine;
use suzuri_engine::dict::Dictionary;
use suzuri_engine::history::UserHistory;
use tauri::Manager;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, Submenu};
use tauri::path::BaseDirectory;
use tauri::tray::TrayIconBuilder;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

pub struct AppState {
    pub engine: Engine,
    pub history: Arc<Mutex<UserHistory>>,
    pub saver: history_store::Saver,
    /// ショートカットで呼び出す前に前面にあったウィンドウ（貼り付けの戻り先）
    pub caller: Mutex<Option<platform::Caller>>,
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        // ログイン時の起動。Windows はレジストリの Run、macOS は LaunchAgent に登録する
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(overlay::on_shortcut)
                .build(),
        )
        .setup(|app| {
            platform::setup(app);

            let dict_path = app.path().resolve("suzuri.dict", BaseDirectory::Resource)?;
            let dict = Dictionary::open(&dict_path).map_err(|e| {
                format!(
                    "{e}（{}）。先に `cargo run -p suzuri-dictgen --release` で辞書を作ってください",
                    dict_path.display()
                )
            })?;
            let history_path = app.path().app_data_dir()?.join("history.tsv");
            let history = Arc::new(Mutex::new(history_store::load(&history_path)));
            let saver = history_store::spawn(history_path, history.clone());
            app.manage(AppState {
                engine: Engine::new(dict),
                history,
                saver,
                caller: Mutex::new(None),
            });

            for shortcut in overlay::shortcuts() {
                if let Err(e) = app.global_shortcut().register(shortcut) {
                    eprintln!("ショートカット {shortcut} を登録できません: {e}");
                }
            }

            let open = MenuItem::with_id(app, "open", "開く", true, None::<&str>)?;
            // 押し間違いで消さないよう、サブメニューの中の項目を選んだときだけリセットする
            let reset = Submenu::with_items(
                app,
                "学習をリセット",
                true,
                &[&MenuItem::with_id(app, "reset_history", "すべての学習を消す", true, None::<&str>)?],
            )?;
            let autostart = CheckMenuItem::with_id(
                app,
                "autostart",
                "ログイン時に起動",
                true,
                app.autolaunch().is_enabled().unwrap_or(false),
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().ok_or("アイコンがありません")?)
                .tooltip("スズリ")
                .menu(&Menu::with_items(app, &[&open, &autostart, &reset, &quit])?)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "open" => {
                        // トレイから開いたときは戻り先がないので、貼り付けはしない
                        *app.state::<AppState>().caller.lock().unwrap() = None;
                        overlay::show(app, overlay::Mode::New);
                    }
                    "reset_history" => {
                        let state = app.state::<AppState>();
                        *state.history.lock().unwrap() = UserHistory::default();
                        state.saver.request();
                    }
                    "autostart" => {
                        let launcher = app.autolaunch();
                        let enabled = launcher.is_enabled().unwrap_or(false);
                        let result = if enabled { launcher.disable() } else { launcher.enable() };
                        if let Err(e) = result {
                            eprintln!("ログイン時の起動を切り替えられません: {e}");
                        }
                        // チェックは実際の登録状態に合わせる（切り替えに失敗したときも食い違わないように）
                        let _ = autostart.set_checked(launcher.is_enabled().unwrap_or(false));
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Alt+F4 などで閉じられても、隠すだけにして使い回す
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::convert,
            commands::convert_with_boundaries,
            commands::resegment,
            commands::commit,
            commands::cancel,
        ])
        .run(tauri::generate_context!())
        .expect("スズリを起動できません");
}
