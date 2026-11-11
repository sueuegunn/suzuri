//! 学習データとユーザー辞書の読み込みと保存。
//! 保存は専用のスレッドで行い、変換の処理をファイル入出力で待たせない。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use suzuri_engine::history::{DEFAULT_CAPACITY, UserHistory};
use suzuri_engine::{UserData, UserDictionary};

pub struct Paths {
    pub history: PathBuf,
    pub dictionary: PathBuf,
}

/// ファイルがない・壊れているときは空から始める。起動は失敗させない。
/// ユーザー辞書は壊れていても上書きしないよう、読めなかったら退避してから空で始める
pub fn load(paths: &Paths) -> UserData {
    let history = match fs::read_to_string(&paths.history) {
        Ok(text) => UserHistory::deserialize(&text, DEFAULT_CAPACITY).unwrap_or_else(|e| {
            eprintln!("{e}。学習データを空にして始めます");
            UserHistory::default()
        }),
        Err(_) => UserHistory::default(),
    };
    let dictionary = match fs::read_to_string(&paths.dictionary) {
        Ok(text) => UserDictionary::deserialize(&text).unwrap_or_else(|e| {
            // 手で登録した語は作り直せないので、捨てずに残す
            let backup = paths.dictionary.with_extension("tsv.broken");
            eprintln!("{e}。{} に退避して、空のユーザー辞書で始めます", backup.display());
            let _ = fs::rename(&paths.dictionary, backup);
            UserDictionary::default()
        }),
        Err(_) => UserDictionary::default(),
    };
    UserData { history, dictionary }
}

#[derive(Clone, Copy)]
enum Target {
    History,
    Dictionary,
}

pub struct Saver(Sender<Target>);

impl Saver {
    /// 学習データの保存を頼む。すぐに戻る
    pub fn request_history(&self) {
        let _ = self.0.send(Target::History);
    }

    /// ユーザー辞書の保存を頼む。すぐに戻る
    pub fn request_dictionary(&self) {
        let _ = self.0.send(Target::Dictionary);
    }
}

pub fn spawn(paths: Paths, user: Arc<Mutex<UserData>>) -> Saver {
    let (tx, rx) = mpsc::channel::<Target>();
    thread::spawn(move || {
        while let Ok(first) = rx.recv() {
            // 溜まった依頼はまとめて、それぞれ1回で済ませる
            let (mut history, mut dictionary) = (false, false);
            for target in std::iter::once(first).chain(rx.try_iter()) {
                match target {
                    Target::History => history = true,
                    Target::Dictionary => dictionary = true,
                }
            }
            if history {
                let text = user.lock().unwrap().history.serialize();
                if let Err(e) = save(&paths.history, &text) {
                    eprintln!("学習データを保存できません: {e}");
                }
            }
            if dictionary {
                let text = user.lock().unwrap().dictionary.serialize();
                if let Err(e) = save(&paths.dictionary, &text) {
                    eprintln!("ユーザー辞書を保存できません: {e}");
                }
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
