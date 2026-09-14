//! WinDivert 工作线程的通用骨架
//!
//! 只读观察与重定向共用同一套句柄生命周期协议。这套协议里有几处不写下来就会被
//! 抄错的地方（先摘牌再关闭、stop 持锁期间调用 Shutdown），集中在这里实现一次。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use super::{load, locate, RawHandle, Windivert};

/// 锁中毒时沿用内部数据：这里的数据都是可再生的，因一次 panic 永久失效不值得
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 把 WinDivert 的错误码翻译成能照着做的提示
pub(crate) fn open_error_message(code: u32) -> String {
    match code {
        super::ERROR_ACCESS_DENIED => "打开 WinDivert 失败：需要管理员权限".to_string(),
        super::ERROR_FILE_NOT_FOUND => {
            "打开 WinDivert 设备失败（错误 2）：驱动未被系统接受，请确认 WinDivert64.sys 与 WinDivert.dll 同目录且未被安全软件拦截"
                .to_string()
        }
        other => format!("打开 WinDivert 失败（错误 {}）", other),
    }
}

/// 句柄 + 工作线程的生命周期管理
pub(crate) struct Worker {
    exe_dir: PathBuf,
    thread_name: &'static str,
    stop_flag: Arc<AtomicBool>,
    /// 线程是否已结束；初始为 true，避免「从未启动」被当成「正在运行」
    exited: Arc<AtomicBool>,
    handle: Arc<Mutex<Option<RawHandle>>>,
    library: Mutex<Option<Arc<Windivert>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Worker {
    pub(crate) fn new(exe_dir: PathBuf, thread_name: &'static str) -> Self {
        Self {
            exe_dir,
            thread_name,
            stop_flag: Arc::new(AtomicBool::new(false)),
            exited: Arc::new(AtomicBool::new(true)),
            handle: Arc::new(Mutex::new(None)),
            library: Mutex::new(None),
            thread: Mutex::new(None),
        }
    }

    /// 打开句柄并启动工作线程
    ///
    /// 句柄在调用线程上打开：失败必须立刻能作为命令结果返回，不能等线程里再报，
    /// 否则界面只会看到「已启动」然后永远没有数据。
    pub(crate) fn start<F>(
        &self,
        filter: &str,
        layer: i32,
        flags: u64,
        run: F,
    ) -> Result<(), String>
    where
        F: FnOnce(&Windivert, RawHandle, &AtomicBool) + Send + 'static,
    {
        let library = Arc::new(load(&self.exe_dir)?);
        let handle = library
            .open(filter, layer, 0, flags)
            .map_err(open_error_message)?;

        self.stop_flag.store(false, Ordering::SeqCst);
        *lock(&self.handle) = Some(handle);
        // 退出标志必须在派生线程之前复位：线程启动后可能立刻结束，
        // 若复位晚了，两者之间的一次状态查询会把已结束的线程报成运行中
        self.exited.store(false, Ordering::SeqCst);
        *lock(&self.library) = Some(Arc::clone(&library));

        let stop_flag = Arc::clone(&self.stop_flag);
        let exited = Arc::clone(&self.exited);
        let slot = Arc::clone(&self.handle);
        let thread = thread::Builder::new()
            .name(self.thread_name.to_string())
            .spawn(move || {
                run(&library, handle, &stop_flag);

                // 先摘牌再关闭句柄：stop() 只在持有槽位锁时调用 Shutdown，
                // 因此「摘牌」一定发生在关闭之前
                *lock(&slot) = None;
                library.close(handle);
                exited.store(true, Ordering::SeqCst);
            })
            .map_err(|error| format!("创建{}线程失败：{}", self.thread_name, error))?;

        *lock(&self.thread) = Some(thread);
        Ok(())
    }

    /// 停止并回收线程。幂等：未运行时直接返回。
    pub(crate) fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);

        // 先叫醒阻塞中的 recv，再回收线程
        let library = lock(&self.library).clone();
        if let Some(library) = library {
            let slot = lock(&self.handle);
            if let Some(handle) = *slot {
                library.shutdown(handle);
            }
        }

        if let Some(thread) = lock(&self.thread).take() {
            let _ = thread.join();
        }
        *lock(&self.handle) = None;
        *lock(&self.library) = None;
    }

    /// 回收已自然退出的线程，避免它被当成「仍在运行」
    pub(crate) fn reap_if_finished(&self) {
        if !self.exited.load(Ordering::SeqCst) {
            return;
        }
        if let Some(thread) = lock(&self.thread).take() {
            let _ = thread.join();
        }
        *lock(&self.handle) = None;
        *lock(&self.library) = None;
    }

    pub(crate) fn is_running(&self) -> bool {
        !self.exited.load(Ordering::SeqCst)
    }

    /// 运行库是否就位（用于界面提示，不触发加载）
    pub(crate) fn library_available(&self) -> bool {
        locate(&self.exe_dir).is_some()
    }
}
