//! 全局应用状态、菜单忙标志与操作标志守卫。

use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::app_error::AppError;
use crate::core::Core;
use crate::settings::Settings;
use crate::tray::StatusIcons;

/// 全局应用状态。
pub struct AppState {
    /// 可执行文件所在目录, 所有相对路径以此为基准。
    pub exe_dir: PathBuf,
    /// 托盘菜单里表示核心状态的图标。
    pub icons: StatusIcons,
    pub settings: Settings,
    /// 子进程句柄, 用于直接 kill。
    pub child_sing_box: Option<Child>,
    pub child_xray: Option<Child>,
}

impl AppState {
    /// 取出核心的子进程句柄, 同时从状态中移除。
    pub fn take_child(&mut self, core: Core) -> Option<Child> {
        match core {
            Core::SingBox => self.child_sing_box.take(),
            Core::Xray => self.child_xray.take(),
        }
    }

    /// 记录核心的子进程句柄。
    pub fn set_child(&mut self, core: Core, child: Child) {
        match core {
            Core::SingBox => self.child_sing_box = Some(child),
            Core::Xray => self.child_xray = Some(child),
        }
    }
}

/// 全局应用状态, 通过 OnceLock + Mutex 实现线程安全的单例。
static APP: OnceLock<Mutex<AppState>> = OnceLock::new();

/// 安装应用状态。重复初始化时保持原状态并返回错误。
pub fn initialize(app: AppState) -> Result<(), AppError> {
    APP.set(Mutex::new(app))
        .map_err(|_| AppError::Msg("初始化状态失败".into()))
}

/// 布尔标志的 RAII 守卫: 获取成功即置位, Drop 时清零。
///
/// 用于"同一类操作同时只允许一个"的场景。手工配对 `store(false)` 时, 任何
/// 一条提前返回路径漏掉释放, 标志就永久置位、对应功能彻底失效; 交给 Drop
/// 之后, 提前返回、线程结束和闭包被丢弃都会自动释放。
#[must_use = "必须持有守卫直到操作结束, 丢弃守卫会立即释放标志"]
pub struct FlagGuard(&'static AtomicBool);

impl FlagGuard {
    /// 尝试置位标志。已被占用时返回 `None`。
    pub fn acquire(flag: &'static AtomicBool) -> Option<Self> {
        if flag.swap(true, Ordering::SeqCst) {
            None
        } else {
            Some(Self(flag))
        }
    }
}

impl Drop for FlagGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// 菜单操作忙标志, 防止并发执行冲突操作。true 表示有操作正在执行。
///
/// 刻意不对外暴露, 只能通过 `acquire_busy` 占用。
static BUSY: AtomicBool = AtomicBool::new(false);

/// 占用菜单操作忙标志, 已有操作在执行时返回 `None`。
///
/// 托盘命令分发有近十条提前返回路径 (取不到 exe 目录、应用状态不可用、
/// 保存配置失败、创建后台线程失败……) , 所以必须用 RAII 而不是手工释放。
pub fn acquire_busy() -> Option<FlagGuard> {
    FlagGuard::acquire(&BUSY)
}

/// 在锁内只读访问状态, 未初始化时不执行闭包并返回 `None`。
///
/// 状态借用不能逃出闭包。闭包内不得再次访问全局状态; 耗时操作应在取得快照后执行。
pub fn with_app_state<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&AppState) -> R,
{
    with_state(&APP, |app| f(app))
}

/// 在锁内修改状态, 未初始化时不执行闭包并返回 `None`。
///
/// 闭包可以移入或取出拥有的值, 但不得再次获取状态锁。设置修改及保存必须在同一次
/// 访问中完成, 保存失败不回滚内存修改; 调用方负责处理闭包返回的错误。
pub fn with_mut_app_state<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut AppState) -> R,
{
    with_state(&APP, f)
}

fn with_state<F, R>(state: &OnceLock<Mutex<AppState>>, f: F) -> Option<R>
where
    F: FnOnce(&mut AppState) -> R,
{
    // R4 仅收口访问接口, 中毒处理由下一批故障策略统一替换。
    let mut app = state.get()?.lock().unwrap_or_else(|e| e.into_inner());
    Some(f(&mut app))
}

/// 获取 exe 所在目录。
pub fn exe_dir() -> Result<PathBuf, AppError> {
    with_app_state(|app| app.exe_dir.clone()).ok_or_else(|| AppError::Msg("应用状态不可用".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::CoreMode;
    use std::sync::{TryLockError, mpsc};
    use std::thread;
    use std::time::Duration;

    fn local_state(exe_dir: PathBuf) -> OnceLock<Mutex<AppState>> {
        OnceLock::from(Mutex::new(AppState {
            exe_dir,
            icons: StatusIcons::default(),
            settings: Settings::default(),
            child_sing_box: None,
            child_xray: None,
        }))
    }

    #[test]
    fn test_uninitialized_state_does_not_call_closure() {
        let state = OnceLock::new();
        let result = with_state(&state, |_| panic!("未初始化时不得执行状态闭包"));
        assert!(result.is_none(), "未初始化必须与闭包返回值区分");
    }

    #[test]
    fn test_state_closure_moves_owned_values_and_releases_lock() {
        let state = local_state(PathBuf::new());
        let directory = PathBuf::from("owned-directory");
        with_state(&state, |app| app.exe_dir = directory).expect("局部状态应已初始化");
        let directory = with_state(&state, |app| std::mem::take(&mut app.exe_dir))
            .expect("应能从闭包取出拥有的值");
        assert_eq!(directory, PathBuf::from("owned-directory"));
        let app = state
            .get()
            .expect("局部状态应已初始化")
            .try_lock()
            .expect("闭包返回后必须释放锁");
        assert!(app.exe_dir.as_os_str().is_empty(), "取出后原字段应为空");
    }

    #[test]
    fn test_state_mutation_and_save_are_serialized() {
        let directory = tempfile::tempdir().expect("应能创建设置测试目录");
        let state = local_state(directory.path().to_path_buf());
        let (locked_tx, locked_rx) = mpsc::channel();
        let (save_tx, save_rx) = mpsc::channel();
        let (waiting_tx, waiting_rx) = mpsc::channel();

        thread::scope(|scope| {
            let first_state = &state;
            let first = scope.spawn(move || {
                with_state(first_state, |app| {
                    app.settings.core.mode = CoreMode::SingBox;
                    locked_tx.send(()).expect("主线程应等待持锁信号");
                    save_rx
                        .recv_timeout(Duration::from_secs(5))
                        .expect("主线程应允许首次保存");
                    app.settings.save(&app.exe_dir)
                })
                .expect("局部状态应已初始化")
                .expect("首次设置保存应成功");
            });
            locked_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("首个工作线程应已修改状态并持锁");
            assert!(
                matches!(
                    state.get().expect("状态应存在").try_lock(),
                    Err(TryLockError::WouldBlock)
                ),
                "修改后、保存前仍必须持锁"
            );
            let second = scope.spawn(|| {
                waiting_tx.send(()).expect("主线程应等待第二次访问");
                with_state(&state, |app| {
                    app.settings.log.max_size_mb = 19;
                    app.settings.save(&app.exe_dir)
                })
                .expect("局部状态应已初始化")
                .expect("第二次设置保存应成功");
            });
            waiting_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("第二个线程应准备访问状态");
            save_tx.send(()).expect("首个线程应等待保存信号");
            first.join().expect("首个设置线程不应 panic");
            second.join().expect("第二个设置线程不应 panic");
        });

        let saved: Settings = serde_json::from_str(
            &std::fs::read_to_string(directory.path().join("settings.json"))
                .expect("应能读取最终设置"),
        )
        .expect("保存的设置应可解析");
        assert_eq!(
            saved.core.mode,
            CoreMode::SingBox,
            "后一次保存必须保留前一次修改, 不能用旧快照覆盖"
        );
        assert_eq!(saved.log.max_size_mb, 19);
    }

    #[test]
    fn test_state_save_failure_keeps_memory_change_and_returns_error() {
        let directory = tempfile::tempdir().expect("应能创建设置测试目录");
        let state = local_state(directory.path().join("missing-parent"));
        let saved = with_state(&state, |app| {
            app.settings.core.mode = CoreMode::Both;
            app.settings.save(&app.exe_dir)
        })
        .expect("局部状态应已初始化");
        assert!(saved.is_err(), "缺失父目录的保存错误必须传出闭包");
        assert_eq!(
            with_state(&state, |app| app.settings.core.mode),
            Some(CoreMode::Both),
            "保存失败保持已修改的内存状态, 不新增回滚"
        );
    }

    /// BUSY 是全局状态, 相关断言集中在一个测试里, 避免并行执行时互相干扰。
    #[test]
    fn test_busy_guard_lifecycle() {
        let guard = acquire_busy().expect("空闲时应能获取");
        assert!(acquire_busy().is_none(), "已占用时不应重复获取");
        drop(guard);
        let guard = acquire_busy().expect("释放后应能重新获取");
        drop(guard);

        // panic 展开时同样释放 (dev profile 是 unwind) , 否则一次后台操作
        // panic 就会永久锁死菜单
        let result = std::thread::spawn(|| {
            let _guard = acquire_busy().expect("panic 测试应能获取");
            panic!("测试用 panic");
        })
        .join();
        assert!(result.is_err(), "工作线程应当 panic");
        assert!(acquire_busy().is_some(), "panic 展开后应已释放");
    }

    /// FlagGuard 对任意静态标志都应满足同样的契约。
    #[test]
    fn test_flag_guard_on_custom_flag() {
        static FLAG: AtomicBool = AtomicBool::new(false);

        let guard = FlagGuard::acquire(&FLAG).expect("空闲时应能获取");
        assert!(FlagGuard::acquire(&FLAG).is_none(), "已占用时不应重复获取");
        drop(guard);
        assert!(FlagGuard::acquire(&FLAG).is_some(), "释放后应能重新获取");
    }
}
