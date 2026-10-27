//! フロントエンドから呼ぶコマンド

use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use suzuri_engine::{Committed, Conversion};
use tauri::{AppHandle, State};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::{AppState, overlay, platform};

/// フォーカスを呼び出し元に戻してから貼り付けのキーを送るまでの待ち時間。
/// 戻りきる前に送ると、貼り付けが空振りするアプリがある
const PASTE_DELAY: Duration = Duration::from_millis(100);

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

#[tauri::command(async)]
pub fn convert(state: State<'_, AppState>, reading: String) -> Vec<SegmentDto> {
    let history = state.history.lock().unwrap();
    to_dto(state.engine.convert(&reading, &history))
}

#[tauri::command(async)]
pub fn convert_with_boundaries(
    state: State<'_, AppState>,
    reading: String,
    boundaries: Vec<usize>,
) -> Vec<SegmentDto> {
    let history = state.history.lock().unwrap();
    to_dto(state.engine.convert_with_boundaries(&reading, &boundaries, &history))
}

#[tauri::command(async)]
pub fn resegment(
    state: State<'_, AppState>,
    reading: String,
    boundaries: Vec<usize>,
    start: usize,
    end: usize,
) -> Vec<SegmentDto> {
    let history = state.history.lock().unwrap();
    to_dto(state.engine.resegment(&reading, &boundaries, start..end, &history))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommittedDto {
    reading: String,
    surface: String,
    was_first: bool,
}

/// 変換結果をクリップボードに入れて閉じ、選んだ候補を学習する。
/// `paste` なら呼び出し元のウィンドウにフォーカスを戻して貼り付ける。
/// 貼り付けに失敗しても手で貼れるよう、クリップボードには残す
#[tauri::command]
pub fn commit(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
    segments: Vec<CommittedDto>,
    paste: bool,
) -> Result<(), String> {
    let written = app.clipboard().write_text(text).map_err(|e| e.to_string());
    let caller = state.caller.lock().unwrap().take().filter(|_| paste && written.is_ok());
    if let Some(caller) = caller {
        platform::return_focus(&app, caller);
    }
    overlay::hide(&app);
    if caller.is_some() {
        thread::spawn(|| {
            thread::sleep(PASTE_DELAY);
            if let Err(e) = platform::send_paste() {
                eprintln!("貼り付けのキーを送れません: {e}");
            }
        });
    }
    written?;
    if !segments.is_empty() {
        let committed: Vec<Committed> = segments
            .into_iter()
            .map(|s| Committed {
                reading: s.reading,
                surface: s.surface,
                was_first: s.was_first,
            })
            .collect();
        state.history.lock().unwrap().commit(&committed);
        state.saver.request();
    }
    Ok(())
}

#[tauri::command]
pub fn cancel(app: AppHandle) {
    overlay::hide(&app);
}
