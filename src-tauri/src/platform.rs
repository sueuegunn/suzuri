//! OS ごとに違う処理。OS で分岐するコードはここにまとめる。

use tauri::App;

/// 起動時の設定。macOS では Dock にアイコンを出さず、メニューバーだけに置く
pub fn setup(app: &mut App) {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}
