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
    // 每個 exe 固定一份檔、每次啟動用 truncate 清空重寫（見下方 open flags），這樣不會愈積愈多
    // 垃圾檔，有人回報問題就丟這份最新的即可。
    //
    // 取捨：不再帶 pid。代價是同一個 exe 同時跑兩份時（罕見），兩邊都 truncate 開同一個檔，
    // 後啟動的會把先啟動的內容清掉、而先啟動的還在舊 offset 上寫，留下一份看似「寫到一半就停」
    // 的 log（程式其實好好的）。一般玩家不會同時開兩份，用這風險換乾淨的檔案數量。
    let exe = crate::windows::utils::get_exec_path();
    let stem = exe.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
    let path = crate::windows::utils::get_game_dir().join(format!("hachimi_tw_{stem}.log"));
    if let Ok(file) = OpenOptions::new().create(true).write(true).truncate(true).open(path) {
        if log::set_boxed_logger(Box::new(FileLogger { file: Mutex::new(file), level })).is_ok() {
            log::set_max_level(filter_level);
        }
    }
}
