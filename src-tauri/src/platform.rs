//! OS ごとに違う処理。OS で分岐するコードはここにまとめる。

use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use tauri::{App, AppHandle};

/// 起動時の設定。macOS では Dock にアイコンを出さず、メニューバーだけに置く
pub fn setup(app: &mut App) {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// スズリを呼び出す前に前面にあったウィンドウ。貼り付けのときにフォーカスを戻す先
#[cfg(windows)]
#[derive(Clone, Copy)]
pub struct Caller(isize);

/// macOS はスズリを隠せば直前のアプリが前面に戻るので、覚えておくものはない
#[cfg(not(windows))]
#[derive(Clone, Copy)]
pub struct Caller;

/// いま前面にあるウィンドウを返す。オーバーレイを出す前に呼ぶ
pub fn caller() -> Option<Caller> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        // SAFETY: 引数のない読み取りだけの API
        let hwnd = unsafe { GetForegroundWindow() };
        (!hwnd.is_null()).then_some(Caller(hwnd as isize))
    }
    #[cfg(not(windows))]
    Some(Caller)
}

/// オーバーレイを出すときの準備。macOS では隠したアプリを表に戻す
pub fn before_show(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.show();
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// フォーカスを呼び出し元に戻す。
/// Windows は前面のプロセスしか前面を譲れないので、オーバーレイがまだ前面にあるうちに呼ぶ
pub fn return_focus(app: &AppHandle, to: Caller) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
        let _ = app;
        // SAFETY: 失われたウィンドウを渡しても失敗が返るだけ
        unsafe { SetForegroundWindow(to.0 as _) };
    }
    #[cfg(target_os = "macos")]
    {
        let _ = to;
        let _ = app.hide();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = (app, to);
}

/// 貼り付けのキー（Windows は Ctrl+V、macOS は Cmd+V）を送る。
/// macOS ではシステム設定の「アクセシビリティ」でスズリを許可しておく必要がある
pub fn send_paste() -> Result<(), String> {
    let modifier = if cfg!(target_os = "macos") { Key::Meta } else { Key::Control };
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    enigo.key(modifier, Direction::Press).map_err(|e| e.to_string())?;
    let result = enigo.key(Key::Unicode('v'), Direction::Click);
    // 修飾キーは失敗しても必ず離す
    let release = enigo.key(modifier, Direction::Release);
    result.and(release).map_err(|e| e.to_string())
}
