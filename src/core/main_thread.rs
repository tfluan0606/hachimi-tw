//! 從「沒有 attach 到 il2cpp 的執行緒」排程工作到遊戲主執行緒。
//!
//! `Thread::main_thread().schedule` 會配置 il2cpp 物件，呼叫端必須是 attach 過的執行緒。
//! 獨立設定視窗的執行緒刻意不 attach：attach 後 Unity 的 GC 每次都會把它強制暫停，若剛好停在
//! 它握著系統 heap 鎖時（建字型貼圖會大量配置記憶體），遊戲主執行緒就永遠等不到那把鎖，
//! 實測設定視窗先卡、遊戲跟著卡。所以這類執行緒用 `post` 排隊，由遊戲內 GUI 的執行緒
//! （已 attach）每幀 `drain` 時代為排程。延遲一幀，對套用設定沒差。

use std::sync::Mutex;

use crate::il2cpp::symbols::Thread;

static QUEUE: Mutex<Vec<fn()>> = Mutex::new(Vec::new());

/// 排一個要在遊戲主執行緒跑的工作。任何執行緒都能呼叫。
pub fn post(f: fn()) {
    QUEUE.lock().unwrap().push(f);
}

/// 把排隊的工作交給遊戲主執行緒。只能在 attach 過的執行緒呼叫（遊戲內 GUI 每幀呼叫）。
pub fn drain() {
    let jobs = std::mem::take(&mut *QUEUE.lock().unwrap());
    for job in jobs {
        Thread::main_thread().schedule(job);
    }
}
