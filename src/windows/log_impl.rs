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
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "[{:<5}] {}: {}", record.level(), record.target(), record.args());
            let _ = f.flush();
        }
    }
    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }
}

pub fn init(filter_level: log::LevelFilter) {
    let Some(level) = filter_level.to_level() else { return };
    // 檔名帶 exe 名，避免 launcher(komoemumamusume) 與遊戲本體(komoeumamusume) 共寫同一 log。
    let exe = crate::windows::utils::get_exec_path();
    let stem = exe.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
    let path = crate::windows::utils::get_game_dir().join(format!("hachimi_tw_{stem}.log"));
    if let Ok(file) = OpenOptions::new().create(true).write(true).truncate(true).open(path) {
        if log::set_boxed_logger(Box::new(FileLogger { file: Mutex::new(file), level })).is_ok() {
            log::set_max_level(filter_level);
        }
    }
}
