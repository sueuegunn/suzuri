//! OS ごとに違う処理。OS で分岐するコードはここにまとめる。

use tauri::App;
use tauri_plugin_global_shortcut::Modifiers;

/// 呼び出しショートカットの修飾キー（Space と組み合わせる）。
/// macOS の Ctrl+Option+Space は入力ソースの切り替えと重なるため、Cmd も足している（仮。実機で確認して決める）
pub fn shortcut_modifiers() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER
    } else {
        Modifiers::CONTROL | Modifiers::ALT
    }
}

/// 起動時の設定。macOS では Dock にアイコンを出さず、メニューバーだけに置く
pub fn setup(app: &mut App) {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}
