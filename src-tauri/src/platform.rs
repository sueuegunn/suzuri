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
        let hwnd = caller.0 as _;
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
            if ime.is_null() {
                return None;
            }
            let ask = |command: usize| {
                let mut result = 0usize;
                let ok = SendMessageTimeoutW(ime, WM_IME_CONTROL, command, 0, SMTO_ABORTIFHUNG, TIMEOUT_MS, &mut result);
                (ok != 0).then_some(result)
            };
            let open = ask(IMC_GETOPENSTATUS)? != 0;
            // IME がオンでも、半角英数の変換モードなら英字入力
            Some(open && ask(IMC_GETCONVERSIONMODE)? as u32 & IME_CMODE_NATIVE != 0)
        }
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

    /// 今の入力ソースの ID で見分ける。入力ソースは全体で1つなので、呼び出し元によらない。
    /// 日本語の入力ソースは ID に Japanese を含み（ことえり・Google 日本語入力・ATOK）、その英字モードは Roman で終わる。
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
        classify(&id)
    }

    fn classify(id: &str) -> Option<bool> {
        if id.ends_with(".Roman") || id.starts_with("com.apple.keylayout.") {
            Some(false)
        } else if id.contains("Japanese") {
            Some(true)
        } else {
            None
        }
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
        let _ = to;
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
