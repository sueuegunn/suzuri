//! OS ごとに違う処理。OS で分岐するコードはここにまとめる。

use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use tauri::{App, AppHandle, WebviewWindow};

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

/// 呼び出し元で OS の IME が日本語入力になっているか。英字入力なら false、
/// 読めないときや日本語の IME を使っていないときは None。オーバーレイを出す前に呼ぶ
pub fn ime_is_japanese(caller: Option<Caller>) -> Option<bool> {
    #[cfg(windows)]
    {
        windows_ime::is_japanese(caller?)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = caller;
        macos_ime::is_japanese()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = caller;
        None
    }
}

#[cfg(windows)]
mod windows_ime {
    use std::mem;
    use std::ptr;

    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Input::Ime::{IME_CMODE_NATIVE, ImmGetDefaultIMEWnd};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GUITHREADINFO, GetGUIThreadInfo, GetWindowThreadProcessId, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
        WM_IME_CONTROL,
    };

    use super::Caller;

    // WM_IME_CONTROL の wParam。windows-sys に定義がないので imm.h の値を書く
    const IMC_GETCONVERSIONMODE: usize = 0x0001;
    const IMC_GETOPENSTATUS: usize = 0x0005;
    /// 日本語の言語 ID
    const LANG_JAPANESE: usize = 0x0411;
    /// 応答しないアプリでオーバーレイを出すのが遅れないよう、待つのはここまで
    const TIMEOUT_MS: u32 = 30;

    /// 前面のウィンドウのうち入力を受けている部分の IME に、オンか・日本語の変換モードかを問い合わせる。
    /// 別のプロセスのウィンドウにも使えるが、一部のアプリ（UWP など）はオンでも 0 を返す
    pub fn is_japanese(caller: Caller) -> Option<bool> {
        let ime = ime_window(caller)?;
        let open = send(ime, IMC_GETOPENSTATUS, 0)? != 0;
        // IME がオンでも、半角英数の変換モードなら英字入力
        Some(open && send(ime, IMC_GETCONVERSIONMODE, 0)? as u32 & IME_CMODE_NATIVE != 0)
    }

    /// 呼び出し元のうち入力を受けている部分の IME ウィンドウ。日本語の IME を使っていなければ None
    fn ime_window(caller: Caller) -> Option<HWND> {
        let hwnd = caller.0 as HWND;
        // SAFETY: どの関数も、無効なウィンドウやスレッドを渡すと失敗（0 や null）を返すだけ
        unsafe {
            let thread = GetWindowThreadProcessId(hwnd, ptr::null_mut());
            if thread == 0 {
                return None;
            }
            // 英語の配列だけで使っている人は、IME の状態によらず日本語入力で始める
            if GetKeyboardLayout(thread) as usize & 0xffff != LANG_JAPANESE {
                return None;
            }
            let mut info: GUITHREADINFO = mem::zeroed();
            info.cbSize = mem::size_of::<GUITHREADINFO>() as u32;
            let focus = if GetGUIThreadInfo(thread, &mut info) != 0 && !info.hwndFocus.is_null() {
                info.hwndFocus
            } else {
                hwnd
            };
            let ime = ImmGetDefaultIMEWnd(focus);
            (!ime.is_null()).then_some(ime)
        }
    }

    /// IME ウィンドウに WM_IME_CONTROL を送る。応答がなければ None
    fn send(ime: HWND, command: usize, value: isize) -> Option<usize> {
        let mut result = 0usize;
        // SAFETY: 無効なウィンドウなら失敗（0）を返すだけ
        let ok = unsafe {
            SendMessageTimeoutW(ime, WM_IME_CONTROL, command, value, SMTO_ABORTIFHUNG, TIMEOUT_MS, &mut result)
        };
        (ok != 0).then_some(result)
    }
}

#[cfg(target_os = "macos")]
mod macos_ime {
    use std::ffi::{CStr, c_char, c_void};

    type CFTypeRef = *const c_void;

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        static kTISPropertyInputSourceID: CFTypeRef;
        fn TISCopyCurrentKeyboardInputSource() -> CFTypeRef;
        fn TISGetInputSourceProperty(source: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetCString(string: CFTypeRef, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFRelease(cf: CFTypeRef);
    }

    const UTF8: u32 = 0x0800_0100;

    /// 今の入力ソースの ID で見分ける（`classify_input_source`）。入力ソースは全体で1つなので、呼び出し元によらない。
    /// TIS はメインスレッドから呼ぶ必要がある（ショートカットの処理はメインスレッドで動く）
    pub fn is_japanese() -> Option<bool> {
        // SAFETY: Copy で得たものは必ず解放し、Get で得たものは解放しない（CoreFoundation の決まり）
        let id = unsafe {
            let source = TISCopyCurrentKeyboardInputSource();
            if source.is_null() {
                return None;
            }
            let value = TISGetInputSourceProperty(source, kTISPropertyInputSourceID);
            let mut buffer = [0 as c_char; 256];
            let ok = !value.is_null() && CFStringGetCString(value, buffer.as_mut_ptr(), buffer.len() as isize, UTF8) != 0;
            CFRelease(source);
            if !ok {
                return None;
            }
            CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned()
        };
        super::classify_input_source(&id)
    }
}

/// macOS の入力ソースの ID が日本語入力か。英字入力なら false、日本語の入力ソースでなければ None。
/// 日本語の入力ソースは ID に Japanese を含み（ことえり・Google 日本語入力・ATOK）、その英字モードは Roman で終わる
/// （…Roman・…FullWidthRoman）。キーボード配列だけ（com.apple.keylayout.ABC など）も英字。
/// OS によらずテストできるよう、macOS のモジュールの外に置く
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn classify_input_source(id: &str) -> Option<bool> {
    if id.ends_with("Roman") || id.starts_with("com.apple.keylayout.") {
        Some(false)
    } else if id.contains("Japanese") {
        Some(true)
    } else {
        None
    }
}

/// 英数・かなのキーを、OS の入力メソッドより先に受け取ってオーバーレイに "lang-key" イベント（"alnum" か "kana"）で渡す。
/// macOS の WebView では入力メソッドがこれらのキーを先に取って入力ソースを切り替えてしまい、JS の keydown に届かないため。
/// Windows は keydown で届くので、フロントエンドで見分ける
pub fn forward_lang_keys(overlay: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    macos_keys::forward(overlay);
    #[cfg(not(target_os = "macos"))]
    let _ = overlay;
}

#[cfg(target_os = "macos")]
mod macos_keys {
    use std::ptr::{self, NonNull};

    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSEvent, NSEventMask};
    use tauri::{Emitter, WebviewWindow};

    /// kVK_JIS_Eisu・kVK_JIS_Kana
    const KEY_EISU: u16 = 0x66;
    const KEY_KANA: u16 = 0x68;

    /// アプリ内のキー入力を、ウィンドウや入力メソッドに渡る前に見るモニタを置く。
    /// オーバーレイに来た英数・かなキーだけを止め、ほかのキーやほかのウィンドウ（設定など）はそのまま通す。
    /// 止めたキーは入力メソッドに届かないので、OS の入力ソースは変わらない（呼び出し元の IME と同期しない設計に合う）
    pub fn forward(overlay: &WebviewWindow) {
        let Ok(ns_window) = overlay.ns_window() else {
            return;
        };
        let target = ns_window as usize;
        let overlay = overlay.clone();
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit が渡すイベントは、ハンドラを抜けるまで有効
            let e = unsafe { event.as_ref() };
            let mode = match e.keyCode() {
                KEY_EISU => "alnum",
                KEY_KANA => "kana",
                _ => return event.as_ptr(),
            };
            // ローカルモニタはメインスレッドで呼ばれる
            let Some(mtm) = MainThreadMarker::new() else {
                return event.as_ptr();
            };
            if !e.window(mtm).is_some_and(|w| Retained::as_ptr(&w) as usize == target) {
                return event.as_ptr();
            }
            let _ = overlay.emit("lang-key", mode);
            ptr::null_mut()
        });
        // SAFETY: ハンドラは受け取ったイベントか null だけを返す
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler) };
        // アプリが終わるまで外さない
        std::mem::forget(monitor);
    }
}

/// オーバーレイを出すときの準備。macOS では隠したアプリを表に戻す
pub fn before_show(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.show();
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// フォーカスを呼び出し元に戻し、戻せたかを返す。呼び出し元のウィンドウが閉じられていたら何もしない。
/// Windows は前面のプロセスしか前面を譲れないので、オーバーレイがまだ前面にあるうちに呼ぶ
pub fn return_focus(app: &AppHandle, to: Caller) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, IsWindowVisible, SetForegroundWindow};
        let _ = app;
        let hwnd = to.0 as _;
        // SAFETY: どの関数も、無効なウィンドウを渡すと失敗（0）を返すだけ
        unsafe { IsWindow(hwnd) != 0 && IsWindowVisible(hwnd) != 0 && SetForegroundWindow(hwnd) != 0 }
    }
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        let _ = to;
        // アプリを隠すときに出ていたウィンドウは、次にアプリを表に戻したとき（before_show）に元の位置で出てしまう。
        // 先にオーバーレイを隠しておき、呼び出し直したときに前の位置で一瞬見えないようにする
        if let Some(window) = app.get_webview_window(crate::overlay::LABEL) {
            let _ = window.hide();
        }
        app.hide().is_ok()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (app, to);
        false
    }
}

/// 呼び出し元がいま前面にあるか。貼り付けのキーを送る直前に確かめ、別のウィンドウに送らないようにする。
/// macOS は前面のアプリを確かめる手段を持たないので、常に true
pub fn is_foreground(caller: Caller) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        // SAFETY: 引数のない読み取りだけの API
        unsafe { GetForegroundWindow() as isize == caller.0 }
    }
    #[cfg(not(windows))]
    {
        let _ = caller;
        true
    }
}

/// 貼り付けのキー（Windows は Ctrl+V、macOS は Cmd+V）を送る。メインスレッドから呼ぶこと。
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

#[cfg(test)]
mod tests {
    use super::classify_input_source;

    #[test]
    fn classifies_macos_input_sources() {
        // ことえり
        assert_eq!(classify_input_source("com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese"), Some(true));
        assert_eq!(classify_input_source("com.apple.inputmethod.Kotoeri.RomajiTyping.Roman"), Some(false));
        assert_eq!(classify_input_source("com.apple.keylayout.ABC"), Some(false));
        // Google 日本語入力。全角英数も英字入力とみなす
        assert_eq!(classify_input_source("com.google.inputmethod.Japanese.base"), Some(true));
        assert_eq!(classify_input_source("com.google.inputmethod.Japanese.Roman"), Some(false));
        assert_eq!(classify_input_source("com.google.inputmethod.Japanese.FullWidthRoman"), Some(false));
        // 日本語以外の入力ソース
        assert_eq!(classify_input_source("com.apple.inputmethod.SCIM.ITABC"), None);
    }
}
