// 把 log 寫到遊戲目錄的 hachimi_tw.log（TW spike 用；OutputDebugString 需 DebugView 才看得到，改寫檔省事）。
use std::{fs::OpenOptions, io::Write, sync::Mutex};

struct FileLogger {
    file: Mutex<std::fs::File>,
    level: log::Level,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level
    }
    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // 用 into_inner 而不是 `if let Ok(..)`：只要有任何執行緒在持有這把鎖時 panic，
        // 鎖就會被標記為 poisoned，往後每次 lock() 都回 Err——結果是 log 從那一刻起
        // 完全靜音，遊戲卻還在跑。查問題時最不需要的就是這個。
        let mut f = self.file.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(f, "[{:<5}] {}: {}", record.level(), record.target(), record.args());
        let _ = f.flush();
    }
    fn flush(&self) {
        let mut f = self.file.lock().unwrap_or_else(|e| e.into_inner());
        let _ = f.flush();
    }
}

pub fn init(filter_level: log::LevelFilter) {
    let Some(level) = filter_level.to_level() else { return };
    // 檔名帶 exe 名，避免 launcher(komoemumamusume) 與遊戲本體(komoeumamusume) 共寫同一 log。
    //
    // 再帶上 pid：同一個 exe 同時跑兩份時（實際發生過），兩邊都用 truncate 開同一個檔，
    // 後啟動的那個會把先啟動的內容清掉，而先啟動的還在自己的舊 offset 上寫——最後留下
    // 一份看起來「寫到一半就停了」的 log，實際上程式一直好好的。
    let exe = crate::windows::utils::get_exec_path();
    let stem = exe.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
    let pid = std::process::id();
    let path = crate::windows::utils::get_game_dir().join(format!("hachimi_tw_{stem}_{pid}.log"));
    if let Ok(file) = OpenOptions::new().create(true).write(true).truncate(true).open(path) {
        if log::set_boxed_logger(Box::new(FileLogger { file: Mutex::new(file), level })).is_ok() {
            log::set_max_level(filter_level);
        }
    }
}
