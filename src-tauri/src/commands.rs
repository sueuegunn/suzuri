//! フロントエンドから呼ぶコマンド

use serde::{Deserialize, Serialize};
use suzuri_engine::{Committed, Conversion};
use tauri::{AppHandle, State};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::AppState;
use crate::overlay;

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

/// 変換結果をクリップボードに入れて閉じ、選んだ候補を学習する
#[tauri::command]
pub fn commit(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
    segments: Vec<CommittedDto>,
) -> Result<(), String> {
    overlay::hide(&app);
    app.clipboard().write_text(text).map_err(|e| e.to_string())?;
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
