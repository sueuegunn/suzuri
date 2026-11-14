//! フロントエンドから呼ぶコマンド

use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use suzuri_engine::{CommitSegment, Conversion, UserPos, UserWord};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::settings::{Appearance, Theme};
use crate::shortcuts::Bindings;
use crate::{AppState, overlay, platform};

/// フォーカスを呼び出し元に戻してから貼り付けのキーを送るまでの待ち時間。
/// 戻りきる前に送ると、貼り付けが空振りするアプリがある
const PASTE_DELAY: Duration = Duration::from_millis(100);

/// 貼り付けのキーを送ってから、クリップボードを元に戻すまでの待ち時間。
/// 貼り付け先のアプリがクリップボードを読み終える前に戻すと、元の中身が貼られてしまう
const RESTORE_DELAY: Duration = Duration::from_millis(500);

/// しばらく待ってから、クリップボードを貼り付ける前の中身に戻す（None なら空にする）。
/// 待つ間にほかのものがコピーされていたら、そちらを残す
fn restore_clipboard_later(app: AppHandle, pasted: String, previous: Option<String>) {
    thread::spawn(move || {
        thread::sleep(RESTORE_DELAY);
        let main = app.clone();
        let done = app.run_on_main_thread(move || {
            let clipboard = main.clipboard();
            // Windows は改行を CRLF にして返すことがあるので、改行をそろえて比べる
            let current = clipboard.read_text().unwrap_or_default();
            if current.replace("\r\n", "\n") != pasted.replace("\r\n", "\n") {
                return;
            }
            let restored = match previous {
                Some(text) => clipboard.write_text(text),
                None => clipboard.clear(),
            };
            if let Err(e) = restored {
                eprintln!("クリップボードを元に戻せません: {e}");
            }
        });
        if let Err(e) = done {
            eprintln!("クリップボードを元に戻せません: {e}");
        }
    });
}

#[derive(Serialize)]
pub struct SegmentDto {
    start: usize,
    end: usize,
    reading: String,
    candidates: Vec<String>,
}

fn to_dto(c: Conversion) -> Vec<SegmentDto> {
    c.segments
        .into_iter()
        .map(|s| SegmentDto {
            start: s.start,
            end: s.end,
            reading: s.reading,
            candidates: s.candidates,
        })
        .collect()
}

/// 続きの候補（画面とのやりとり用）
#[derive(Serialize)]
pub struct SuggestionDto {
    matched: usize,
    reading: String,
    surface: String,
}

/// 打っている語の続きを1つ探す。`starts` は語が始まりうる位置（最後の文節とその1つ前の文節の先頭）。なければ None
#[tauri::command(async)]
pub fn suggest(state: State<'_, AppState>, reading: String, starts: Vec<usize>) -> Option<SuggestionDto> {
    let user = state.user.lock().unwrap();
    state.engine.suggest(&reading, &starts, &user).map(|s| SuggestionDto {
        matched: s.matched,
        reading: s.reading,
        surface: s.surface,
    })
}

#[tauri::command(async)]
pub fn convert(state: State<'_, AppState>, reading: String) -> Vec<SegmentDto> {
    let user = state.user.lock().unwrap();
    to_dto(state.engine.convert(&reading, &user))
}

#[tauri::command(async)]
pub fn convert_with_boundaries(
    state: State<'_, AppState>,
    reading: String,
    boundaries: Vec<usize>,
) -> Vec<SegmentDto> {
    let user = state.user.lock().unwrap();
    to_dto(state.engine.convert_with_boundaries(&reading, &boundaries, &user))
}

#[tauri::command(async)]
pub fn resegment(
    state: State<'_, AppState>,
    reading: String,
    boundaries: Vec<usize>,
    start: usize,
    end: usize,
) -> Vec<SegmentDto> {
    let user = state.user.lock().unwrap();
    to_dto(state.engine.resegment(&reading, &boundaries, start..end, &user))
}

/// `ranges` のそれぞれの内側だけ自動で区切り直す（右ドラッグで選んだ範囲を1文節にしたとき）
#[tauri::command(async)]
pub fn resegment_ranges(
    state: State<'_, AppState>,
    reading: String,
    boundaries: Vec<usize>,
    ranges: Vec<(usize, usize)>,
) -> Vec<SegmentDto> {
    let user = state.user.lock().unwrap();
    let ranges: Vec<_> = ranges.into_iter().map(|(start, end)| start..end).collect();
    to_dto(state.engine.resegment_ranges(&reading, &boundaries, &ranges, &user))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommittedDto {
    reading: String,
    surface: String,
    was_first: bool,
    /// 手で置いたピンに接するか（区切りの学習の対象）
    #[serde(default)]
    manual: bool,
}

/// 変換結果をクリップボードに入れて閉じ、選んだ候補を学習する。
/// `paste` なら呼び出し元のウィンドウにフォーカスを戻して貼り付ける。
/// 貼り付けたあとは、設定で残すことにしていなければ、クリップボードを元の中身に戻す。
/// 貼り付けのキーを送れなかったときは、手で貼れるようクリップボードに残す
#[tauri::command]
pub fn commit(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
    segments: Vec<CommittedDto>,
    paste: bool,
) -> Result<(), String> {
    // 戻すときのために、書く前の中身を覚えておく。文字でなければ（空・画像など）None で、戻すときは空にする
    let restore = (paste && !state.settings.get().keep_pasted_in_clipboard)
        .then(|| (text.clone(), app.clipboard().read_text().ok()));
    let written = app.clipboard().write_text(text).map_err(|e| e.to_string());
    // 呼び出し元のウィンドウが閉じられていたら、貼り付けはせずクリップボードに入れるだけにする
    let caller = state
        .caller
        .lock()
        .unwrap()
        .take()
        .filter(|&c| paste && written.is_ok() && platform::return_focus(&app, c));
    overlay::hide(&app);
    if let Some(caller) = caller {
        let main = app.clone();
        // 待つのは別スレッドで行い、メインスレッドを止めない
        thread::spawn(move || {
            thread::sleep(PASTE_DELAY);
            // キーの送信はメインスレッドで行う。macOS では、キーボード配列から「v」のキーを調べる
            // 入力ソースの API がメインスレッド以外から呼ばれると、アプリが強制終了する
            let app = main.clone();
            let sent = main.run_on_main_thread(move || {
                // 待っている間に前面が別のウィンドウに変わっていたら送らない
                if !platform::is_foreground(caller) {
                    return;
                }
                match platform::send_paste() {
                    Ok(()) => {
                        if let Some((pasted, previous)) = restore {
                            restore_clipboard_later(app, pasted, previous);
                        }
                    }
                    Err(e) => eprintln!("貼り付けのキーを送れません: {e}"),
                }
            });
            if let Err(e) = sent {
                eprintln!("貼り付けのキーを送れません: {e}");
            }
        });
    }
    written?;
    if !segments.is_empty() {
        let committed: Vec<CommitSegment> = segments
            .into_iter()
            .map(|s| CommitSegment {
                reading: s.reading,
                surface: s.surface,
                was_first: s.was_first,
                manual: s.manual,
            })
            .collect();
        state.engine.commit(&mut state.user.lock().unwrap(), &committed);
        state.saver.request_history();
    }
    Ok(())
}

/// ユーザー辞書の語（画面とのやりとり用）
#[derive(Serialize, Deserialize)]
pub struct UserWordDto {
    reading: String,
    surface: String,
    /// "noun"・"person"・"place"・"organization"
    pos: String,
}

impl From<&UserWord> for UserWordDto {
    fn from(w: &UserWord) -> Self {
        UserWordDto {
            reading: w.reading.clone(),
            surface: w.surface.clone(),
            pos: w.pos.name().to_owned(),
        }
    }
}

fn user_words(dictionary: &suzuri_engine::UserDictionary) -> Vec<UserWordDto> {
    dictionary.words().iter().map(UserWordDto::from).collect()
}

fn parse_pos(pos: &str) -> Result<UserPos, String> {
    UserPos::from_name(pos).ok_or_else(|| format!("品詞が違います: {pos}"))
}

/// 登録した順に返す
#[tauri::command]
pub fn list_user_words(state: State<'_, AppState>) -> Vec<UserWordDto> {
    user_words(&state.user.lock().unwrap().dictionary)
}

/// 語を登録し、登録後の一覧を返す。読みや表記が正しくなければ、画面に出す理由を返す
#[tauri::command]
pub fn add_user_word(app: AppHandle, state: State<'_, AppState>, word: UserWordDto) -> Result<Vec<UserWordDto>, String> {
    let pos = parse_pos(&word.pos)?;
    let mut user = state.user.lock().unwrap();
    user.dictionary
        .add(&word.reading, &word.surface, pos)
        .map_err(|e| e.to_string())?;
    state.saver.request_dictionary();
    // 開いている設定画面の一覧を読み直させる
    let _ = app.emit("user-dictionary", ());
    Ok(user_words(&user.dictionary))
}

/// オーバーレイの候補の行の文節（いくつかをつないだものでもよい）を、名詞としてユーザー辞書に登録する。
/// 末尾の助詞・助動詞は除く（「スズリで」は「スズリ」）。登録した語を返し、取り消すときに remove_user_word へ渡す
#[tauri::command]
pub fn register_segment(app: AppHandle, state: State<'_, AppState>, reading: String, surface: String) -> Result<UserWordDto, String> {
    let mut user = state.user.lock().unwrap();
    let (reading, surface) = state.engine.content_word(&reading, &surface, &user);
    let word = user
        .dictionary
        .add(&reading, &surface, UserPos::Noun)
        .map_err(|e| e.to_string())?;
    state.saver.request_dictionary();
    // 開いている設定画面の一覧を読み直させる
    let _ = app.emit("user-dictionary", ());
    Ok(UserWordDto::from(&word))
}

/// 登録した語 `old` を `word` に書き換え、書き換えたあとの一覧を返す。正しくなければ、画面に出す理由を返す
#[tauri::command]
pub fn update_user_word(
    app: AppHandle,
    state: State<'_, AppState>,
    old: UserWordDto,
    word: UserWordDto,
) -> Result<Vec<UserWordDto>, String> {
    let target = UserWord {
        reading: old.reading,
        surface: old.surface,
        pos: parse_pos(&old.pos)?,
    };
    let pos = parse_pos(&word.pos)?;
    let mut user = state.user.lock().unwrap();
    user.dictionary
        .replace(&target, &word.reading, &word.surface, pos)
        .map_err(|e| e.to_string())?;
    state.saver.request_dictionary();
    // 開いている設定画面の一覧を読み直させる
    let _ = app.emit("user-dictionary", ());
    Ok(user_words(&user.dictionary))
}

/// 語を消し、消したあとの一覧を返す
#[tauri::command]
pub fn remove_user_word(app: AppHandle, state: State<'_, AppState>, word: UserWordDto) -> Result<Vec<UserWordDto>, String> {
    let target = UserWord {
        reading: word.reading,
        surface: word.surface,
        pos: parse_pos(&word.pos)?,
    };
    let mut user = state.user.lock().unwrap();
    if user.dictionary.remove(&target) {
        state.saver.request_dictionary();
        // 開いている設定画面の一覧を読み直させる
        let _ = app.emit("user-dictionary", ());
    }
    Ok(user_words(&user.dictionary))
}

/// 閉じる。return_focus なら呼び出し元にフォーカスを戻す。
/// macOS はウィンドウを隠すだけではスズリが前面に残り、元のアプリに戻らないことがある
#[tauri::command]
pub fn cancel(app: AppHandle, state: State<'_, AppState>, return_focus: bool) {
    // 戻すのは貼り付けと同じく、オーバーレイがまだ前面にあるうちに行う
    if let Some(caller) = state.caller.lock().unwrap().take().filter(|_| return_focus) {
        platform::return_focus(&app, caller);
    }
    overlay::hide(&app);
}

#[tauri::command]
pub fn get_shortcuts(state: State<'_, AppState>) -> Bindings {
    state.shortcuts.bindings()
}

/// ショートカットを変えて保存する。`bindings` を省くと既定に戻す
#[tauri::command]
pub fn set_shortcuts(app: AppHandle, state: State<'_, AppState>, bindings: Option<Bindings>) -> Result<Bindings, String> {
    let bindings = state.shortcuts.set(&app, bindings.unwrap_or_default())?;
    state.settings.update(|f| f.shortcuts = bindings.clone())?;
    Ok(bindings)
}

#[tauri::command]
pub fn get_theme(state: State<'_, AppState>) -> Theme {
    state.settings.get().theme
}

#[tauri::command]
pub fn get_appearance(state: State<'_, AppState>) -> Appearance {
    state.settings.get().appearance
}

/// 文字の大きさと背景の不透明度を変えて保存し、オーバーレイに知らせる。範囲外の値は範囲に収める
#[tauri::command]
pub fn set_appearance(app: AppHandle, state: State<'_, AppState>, appearance: Appearance) -> Result<Appearance, String> {
    let appearance = appearance.clamped();
    state.settings.update(|f| f.appearance = appearance)?;
    let _ = app.emit("appearance", appearance);
    Ok(appearance)
}

#[tauri::command]
pub fn get_keep_pasted_in_clipboard(state: State<'_, AppState>) -> bool {
    state.settings.get().keep_pasted_in_clipboard
}

#[tauri::command]
pub fn set_keep_pasted_in_clipboard(state: State<'_, AppState>, keep: bool) -> Result<(), String> {
    state.settings.update(|f| f.keep_pasted_in_clipboard = keep)
}

/// 覚えたオーバーレイの位置と大きさを忘れる。次に開くときは既定の大きさで、マウスのあるモニターの中央に出す
#[tauri::command]
pub fn reset_window_geometry(state: State<'_, AppState>) -> Result<(), String> {
    state.settings.update(|f| {
        f.window_positions.clear();
        f.window_size = None;
    })
}

/// クリップボードの文字（オーバーレイに貼り付けるとき）。文字でなければ空
#[tauri::command]
pub fn read_clipboard(app: AppHandle) -> String {
    app.clipboard().read_text().unwrap_or_default()
}

/// オーバーレイを今いるモニターの中央に戻す（掴みどころのダブルクリック）
/// 画面側から調べるためのログを標準エラーに出す（tauri dev のターミナルに出る）
#[tauri::command]
pub fn debug_log(message: String) {
    eprintln!("[overlay] {message}");
}

#[tauri::command]
pub fn center_overlay(app: AppHandle) {
    overlay::center(&app);
}

/// 設定画面でキーを記録する間、ショートカットの登録を外す
#[tauri::command]
pub fn suspend_shortcuts(app: AppHandle, state: State<'_, AppState>) {
    state.shortcuts.suspend(&app);
}

#[tauri::command]
pub fn resume_shortcuts(app: AppHandle, state: State<'_, AppState>) {
    state.shortcuts.resume(&app);
}
