//! 学習データの読み込みと保存。
//! 保存は専用のスレッドで行い、変換の処理をファイル入出力で待たせない。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use suzuri_engine::history::{DEFAULT_CAPACITY, UserHistory};

/// ファイルがない・壊れているときは空から始める。起動は失敗させない
pub fn load(path: &Path) -> UserHistory {
    let Ok(text) = fs::read_to_string(path) else {
        return UserHistory::default();
    };
    UserHistory::deserialize(&text, DEFAULT_CAPACITY).unwrap_or_else(|e| {
        eprintln!("{e}。学習データを空にして始めます");
        UserHistory::default()
    })
}

pub struct Saver(Sender<()>);

impl Saver {
    /// 保存を頼む。すぐに戻る
    pub fn request(&self) {
        let _ = self.0.send(());
    }
}

pub fn spawn(path: PathBuf, history: Arc<Mutex<UserHistory>>) -> Saver {
    let (tx, rx) = mpsc::channel::<()>();
    thread::spawn(move || {
        while rx.recv().is_ok() {
            // 溜まった依頼はまとめて1回で済ませる
            while rx.try_recv().is_ok() {}
            let text = history.lock().unwrap().serialize();
            if let Err(e) = save(&path, &text) {
                eprintln!("学習データを保存できません: {e}");
            }
        }
    });
    Saver(tx)
}

/// 一時ファイルに書いてから置き換え、途中で落ちても元のファイルを壊さない
fn save(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tsv.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}
