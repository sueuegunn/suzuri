// リリースビルドでコンソールウィンドウを出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod overlay;
mod platform;
mod settings;
mod shortcuts;
mod user_store;

use std::sync::{Arc, Mutex};

use suzuri_engine::dict::Dictionary;
use suzuri_engine::history::UserHistory;
use suzuri_engine::context::ContextHistory;
use suzuri_engine::learned::LearnedWords;
use suzuri_engine::{Engine, UserData};
use settings::{StartInputMode, Theme};
use tauri::{Emitter, Manager};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, Submenu};
use tauri::path::BaseDirectory;
use tauri::tray::TrayIconBuilder;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

pub struct AppState {
    pub engine: Engine,
    /// 学習とユーザー辞書。変換のたびにエンジンへ渡す
    pub user: Arc<Mutex<UserData>>,
    pub saver: user_store::Saver,
    /// ショートカットで呼び出す前に前面にあったウィンドウ（貼り付けの戻り先）
    pub caller: Mutex<Option<platform::Caller>>,
    /// ショートカットで呼び出したとき、呼び出し元の IME が日本語入力だったか。オーバーレイの入力モードの初期値
    pub ime_japanese: Mutex<Option<bool>>,
    pub settings: settings::Settings,
    /// オーバーレイを表示したときに置いた位置と大きさ
    pub placement: Mutex<Option<overlay::Placement>>,
    pub shortcuts: shortcuts::Shortcuts,
}

const SETTINGS_LABEL: &str = "settings";
const THEME_MENU_PREFIX: &str = "theme_";
const INPUT_MENU_PREFIX: &str = "input_";

fn input_menu_id(mode: StartInputMode) -> String {
    let name = match mode {
        StartInputMode::MatchIme => "match_ime",
        StartInputMode::Kana => "kana",
        StartInputMode::Alnum => "alnum",
    };
    format!("{INPUT_MENU_PREFIX}{name}")
}

fn theme_menu_id(theme: Theme) -> String {
    let name = match theme {
        Theme::System => "system",
        Theme::Light => "light",
        Theme::Dark => "dark",
    };
    format!("{THEME_MENU_PREFIX}{name}")
}

/// 設定ウィンドウを開く。開いていれば前に出す。必要なときだけ作り、閉じたら破棄する
fn open_settings(app: &tauri::AppHandle) {
    platform::before_show(app);
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let built = tauri::WebviewWindowBuilder::new(app, SETTINGS_LABEL, tauri::WebviewUrl::App("index.html#settings".into()))
        .title("スズリの設定")
        .inner_size(480.0, 600.0)
        .resizable(false)
        .center()
        .build();
    match built {
        Ok(window) => {
            let _ = window.set_focus();
        }
        Err(e) => eprintln!("設定ウィンドウを開けません: {e}"),
    }
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
            if let Some(overlay) = app.get_webview_window(overlay::LABEL) {
                platform::forward_lang_keys(&overlay);
            }

            let dict_path = app.path().resolve("suzuri.dict", BaseDirectory::Resource)?;
            let dict = Dictionary::open(&dict_path).map_err(|e| {
                format!(
                    "{e}（{}）。先に `cargo run -p suzuri-dictgen --release` で辞書を作ってください",
                    dict_path.display()
                )
            })?;
            let data_dir = app.path().app_data_dir()?;
            let paths = user_store::Paths {
                history: data_dir.join("history.tsv"),
                learned: data_dir.join("learned_words.tsv"),
                context: data_dir.join("context_history.tsv"),
                dictionary: data_dir.join("user_dictionary.tsv"),
            };
            let user = Arc::new(Mutex::new(user_store::load(&paths)));
            let saver = user_store::spawn(paths, user.clone());
            let settings = settings::Settings::load(data_dir.join("settings.json"));
            let shortcuts = shortcuts::Shortcuts::new(settings.get().shortcuts);
            app.manage(AppState {
                engine: Engine::new(dict),
                user,
                saver,
                caller: Mutex::new(None),
                ime_japanese: Mutex::new(None),
                settings,
                placement: Mutex::new(None),
                shortcuts,
            });

            let theme = app.state::<AppState>().settings.get().theme;
            app.handle().set_theme(theme.to_tauri());

            for e in app.state::<AppState>().shortcuts.register(app.handle()) {
                eprintln!("ショートカットを登録できません: {e}");
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
            // 3つから1つを選ぶ。チェックは選んだときに付け直す
            let themes = [
                (Theme::System, "OS に合わせる"),
                (Theme::Light, "ライト"),
                (Theme::Dark, "ダーク"),
            ]
            .into_iter()
            .map(|(t, label)| {
                CheckMenuItem::with_id(app, theme_menu_id(t), label, true, t == theme, None::<&str>).map(|item| (t, item))
            })
            .collect::<Result<Vec<_>, _>>()?;
            let appearance = Submenu::with_items(app, "外観", true, &[&themes[0].1, &themes[1].1, &themes[2].1])?;
            let start_input = app.state::<AppState>().settings.get().start_input_mode;
            let inputs = [
                (StartInputMode::Kana, "かな"),
                (StartInputMode::Alnum, "英数"),
                (StartInputMode::MatchIme, "IME に合わせる"),
            ]
            .into_iter()
            .map(|(m, label)| {
                CheckMenuItem::with_id(app, input_menu_id(m), label, true, m == start_input, None::<&str>)
                    .map(|item| (m, item))
            })
            .collect::<Result<Vec<_>, _>>()?;
            let input_mode =
                Submenu::with_items(app, "起動時の入力モード", true, &[&inputs[0].1, &inputs[1].1, &inputs[2].1])?;
            let settings = MenuItem::with_id(app, "settings", "設定…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().ok_or("アイコンがありません")?)
                .tooltip("スズリ")
                .menu(&Menu::with_items(app, &[&open, &settings, &input_mode, &appearance, &autostart, &reset, &quit])?)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "open" => {
                        // トレイから開いたときは戻り先がないので、貼り付けはしない
                        let state = app.state::<AppState>();
                        *state.caller.lock().unwrap() = None;
                        *state.ime_japanese.lock().unwrap() = None;
                        overlay::show(app, overlay::Mode::New);
                    }
                    "reset_history" => {
                        let state = app.state::<AppState>();
                        // ユーザー辞書は学習ではないので消さない
                        {
                            let mut user = state.user.lock().unwrap();
                            user.history = UserHistory::default();
                            user.learned = LearnedWords::default();
                            user.context = ContextHistory::default();
                        }
                        state.saver.request_history();
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
                    "settings" => open_settings(app),
                    id if id.starts_with(THEME_MENU_PREFIX) => {
                        let Some(&(theme, _)) = themes.iter().find(|(t, _)| theme_menu_id(*t) == id) else {
                            return;
                        };
                        for (t, item) in &themes {
                            let _ = item.set_checked(*t == theme);
                        }
                        if let Err(e) = app.state::<AppState>().settings.update(|f| f.theme = theme) {
                            eprintln!("{e}");
                        }
                        app.set_theme(theme.to_tauri());
                        let _ = app.emit("theme", theme);
                    }
                    id if id.starts_with(INPUT_MENU_PREFIX) => {
                        let Some(&(mode, _)) = inputs.iter().find(|(m, _)| input_menu_id(*m) == id) else {
                            return;
                        };
                        for (m, item) in &inputs {
                            let _ = item.set_checked(*m == mode);
                        }
                        if let Err(e) = app.state::<AppState>().settings.update(|f| f.start_input_mode = mode) {
                            eprintln!("{e}");
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // オーバーレイは Alt+F4 などで閉じられても、隠すだけにして使い回す。Esc と同じく画面側で
            // パネルを消してから隠させる（そのまま隠すと、次に出したときに前の内容が一瞬見える）
            tauri::WindowEvent::CloseRequested { api, .. } if window.label() != SETTINGS_LABEL => {
                api.prevent_close();
                let _ = window.emit("close-requested", ());
            }
            // キーの記録中に設定ウィンドウを閉じても、ショートカットが外れたままにならないようにする
            tauri::WindowEvent::Destroyed if window.label() == SETTINGS_LABEL => {
                let app = window.app_handle();
                app.state::<AppState>().shortcuts.resume(app);
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::convert,
            commands::convert_with_boundaries,
            commands::resegment,
            commands::resegment_ranges,
            commands::commit,
            commands::cancel,
            commands::get_shortcuts,
            commands::set_shortcuts,
            commands::suspend_shortcuts,
            commands::resume_shortcuts,
            commands::get_theme,
            commands::get_appearance,
            commands::set_appearance,
            commands::reset_window_geometry,
            commands::get_keep_pasted_in_clipboard,
            commands::set_keep_pasted_in_clipboard,
            commands::list_user_words,
            commands::add_user_word,
            commands::update_user_word,
            commands::remove_user_word,
            commands::register_segment,
            commands::center_overlay,
            commands::suggest,
            commands::read_clipboard,
        ])
        .run(tauri::generate_context!())
        .expect("スズリを起動できません");
}
