use simplelog::{ConfigBuilder, LevelFilter, WriteLogger};
use std::fs::File;

pub fn init(filter_level: log::LevelFilter, file_logging: bool) {
    if file_logging {
        let mut path = super::utils::get_game_dir();
        path.push("hachimi.log");

        if let Ok(file) = File::create(path) {
            let config = ConfigBuilder::new()
                .set_target_level(LevelFilter::Error)
                .add_filter_ignore_str("sqlparser")
                .set_time_format_rfc3339()
                .build();

            if WriteLogger::init(filter_level, config, file).is_ok() {
                return;
            }
        }
    }

    if let Some(level) = filter_level.to_level() {
        windebug_logger::init_with_level(level).ok();
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
