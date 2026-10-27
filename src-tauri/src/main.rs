// リリースビルドでコンソールウィンドウを出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod history_store;
mod overlay;

use std::sync::{Arc, Mutex};

use suzuri_engine::Engine;
use suzuri_engine::dict::Dictionary;
use suzuri_engine::history::UserHistory;
use tauri::Manager;
use tauri::menu::{Menu, MenuItem};
use tauri::path::BaseDirectory;
use tauri::tray::TrayIconBuilder;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

pub struct AppState {
    pub engine: Engine,
    pub history: Arc<Mutex<UserHistory>>,
    pub saver: history_store::Saver,
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(overlay::on_shortcut)
                .build(),
        )
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

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
            });

            for shortcut in overlay::shortcuts() {
                if let Err(e) = app.global_shortcut().register(shortcut) {
                    eprintln!("ショートカット {shortcut} を登録できません: {e}");
                }
            }

            let open = MenuItem::with_id(app, "open", "開く", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().ok_or("アイコンがありません")?)
                .tooltip("スズリ")
                .menu(&Menu::with_items(app, &[&open, &quit])?)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => overlay::show(app, overlay::Mode::New),
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
