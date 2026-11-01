//! 崩潰守門：程序崩潰時只寫一份 dump 到 `hachimi\crash\`，然後直接結束程序。
//!
//! 遊戲裡會註冊 unhandled exception filter 的有 Unity（通知 UnityCrashHandler64）、Komoe SDK
//! （PCGameKomoeSDK.dll，寫 dump 後起 PcGameBugReport.exe 上傳），以及 Windows 自己的 WER。
//! 這裡在最早的時候裝自己的 filter，並 hook `SetUnhandledExceptionFilter`，讓之後別人註冊的
//! filter 只被記下來、不生效。崩潰時寫完 dump 就 `TerminateProcess`，所以那些上報路徑都不會走到。
//!
//! filter 跑在已經壞掉的執行緒上，所以這裡不寫 log（log 的鎖可能正被崩潰的執行緒拿著）。

use std::{
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        OnceLock,
    },
};

use windows::{
    core::{s, w, PCWSTR},
    Win32::{
        Foundation::{CloseHandle, GENERIC_WRITE},
        Storage::FileSystem::{CreateFileW, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE},
        System::{
            Diagnostics::Debug::{
                MiniDumpWithIndirectlyReferencedMemory, MiniDumpWithThreadInfo, MiniDumpWriteDump, SetUnhandledExceptionFilter,
                EXCEPTION_POINTERS, LPTOP_LEVEL_EXCEPTION_FILTER, MINIDUMP_EXCEPTION_INFORMATION, MINIDUMP_TYPE,
            },
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            SystemInformation::GetLocalTime,
            Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, TerminateProcess},
        },
    },
};

use crate::core::Hachimi;

/// 只保留最近幾份 dump
const KEEP_DUMPS: usize = 5;

static DUMP_DIR: OnceLock<PathBuf> = OnceLock::new();
static CRASHING: AtomicBool = AtomicBool::new(false);
/// 別人想註冊的 filter（只記不裝），回給他們下次呼叫時當「前一個」
static REQUESTED_FILTER: AtomicUsize = AtomicUsize::new(0);

type FilterFn = unsafe extern "system" fn(*const EXCEPTION_POINTERS) -> i32;

unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
    let process = GetCurrentProcess();
    let code = if info.is_null() || (*info).ExceptionRecord.is_null() {
        1
    }
    else {
        (*(*info).ExceptionRecord).ExceptionCode.0 as u32
    };

    // 寫 dump 途中又崩：不再嘗試，直接結束
    if !CRASHING.swap(true, Ordering::AcqRel) {
        write_dump(info);
    }
    _ = TerminateProcess(process, code);
    // EXCEPTION_EXECUTE_HANDLER；TerminateProcess 失敗時至少不往下交給 WER
    1
}

unsafe fn write_dump(info: *const EXCEPTION_POINTERS) {
    let Some(dir) = DUMP_DIR.get() else { return };
    let t = GetLocalTime();
    let name = format!(
        "crash_{:04}{:02}{:02}_{:02}{:02}{:02}_{}.dmp",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, GetCurrentProcessId()
    );
    let path: Vec<u16> = dir.join(name).as_os_str().encode_wide().chain(Some(0)).collect();
    let Ok(file) = CreateFileW(PCWSTR(path.as_ptr()), GENERIC_WRITE.0, FILE_SHARE_NONE, None, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, None)
    else {
        return;
    };

    let exception = MINIDUMP_EXCEPTION_INFORMATION {
        ThreadId: GetCurrentThreadId(),
        ExceptionPointers: info as *mut EXCEPTION_POINTERS,
        ClientPointers: false.into(),
    };
    let kind = MINIDUMP_TYPE(MiniDumpWithThreadInfo.0 | MiniDumpWithIndirectlyReferencedMemory.0);
    _ = MiniDumpWriteDump(
        GetCurrentProcess(),
        GetCurrentProcessId(),
        file,
        kind,
        if info.is_null() { None } else { Some(&exception) },
        None,
        None,
    );
    _ = CloseHandle(file);
}

// ---- 不讓別人蓋掉 ----

unsafe extern "system" fn SetUnhandledExceptionFilter_hook(new_filter: LPTOP_LEVEL_EXCEPTION_FILTER) -> LPTOP_LEVEL_EXCEPTION_FILTER {
    let new = new_filter.map(|f| f as usize).unwrap_or(0);
    let prev = REQUESTED_FILTER.swap(new, Ordering::AcqRel);
    // 回傳他們以為的「前一個」，這樣成對的 set/restore 行為不變；實際生效的一直是我們的
    if prev == 0 { None } else { Some(std::mem::transmute::<usize, FilterFn>(prev)) }
}

fn cleanup_old_dumps(dir: &PathBuf) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut dumps: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "dmp"))
        .collect();
    if dumps.len() <= KEEP_DUMPS {
        return;
    }
    // 檔名含時間，字串排序＝時間排序
    dumps.sort_by_key(|e| e.file_name());
    for e in &dumps[..dumps.len() - KEEP_DUMPS] {
        _ = std::fs::remove_file(e.path());
    }
}

pub fn init() {
    let hachimi = Hachimi::instance();
    if hachimi.config.load().disabled_hooks.contains("SetUnhandledExceptionFilter") {
        info!("[CrashGuard] disabled by config");
        return;
    }

    let dir = hachimi.game.data_dir.join("crash");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        error!("[CrashGuard] create {}: {}", dir.display(), e);
        return;
    }
    cleanup_old_dumps(&dir);
    _ = DUMP_DIR.set(dir);

    unsafe {
        // 先裝自己的，再 hook；hook 之後別人的註冊都只會被記下來
        let prev = SetUnhandledExceptionFilter(Some(filter));
        if let Some(prev) = prev {
            REQUESTED_FILTER.store(prev as usize, Ordering::Release);
        }

        let Ok(kernel32) = GetModuleHandleW(w!("kernel32.dll")) else {
            error!("[CrashGuard] kernel32 not found");
            return;
        };
        let Some(addr) = GetProcAddress(kernel32, s!("SetUnhandledExceptionFilter")) else {
            error!("[CrashGuard] SetUnhandledExceptionFilter not found");
            return;
        };
        match hachimi.interceptor.hook(addr as usize, SetUnhandledExceptionFilter_hook as usize) {
            Ok(_) => info!("[CrashGuard] installed, dumps -> hachimi\\crash"),
            Err(e) => error!("[CrashGuard] hook SetUnhandledExceptionFilter: {}", e),
        }
    }
}
