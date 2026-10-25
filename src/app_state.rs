//! 全局应用状态、菜单忙标志与操作标志守卫。

use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

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
pub static APP: OnceLock<Mutex<AppState>> = OnceLock::new();

/// 布尔标志的 RAII 守卫: 获取成功即置位, Drop 时清零。
///
/// 用于"同一类操作同时只允许一个"的场景。手工配对 `store(false)` 时, 任何
/// 一条提前返回路径漏掉释放, 标志就永久置位、对应功能彻底失效; 交给 Drop
/// 之后, 提前返回、线程结束和闭包被丢弃都会自动释放。
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

/// 获取只读应用状态。Mutex 中毒时恢复并继续使用。
pub fn app_state() -> Option<MutexGuard<'static, AppState>> {
    Some(APP.get()?.lock().unwrap_or_else(|e| e.into_inner()))
}

/// 获取可变应用状态。语义与 `app_state()` 相同,
/// Mutex::lock 返回 `MutexGuard` 总是可变的。
pub fn app_state_mut() -> Option<MutexGuard<'static, AppState>> {
    app_state()
}

/// 获取 exe 所在目录。
pub fn exe_dir() -> Result<PathBuf, AppError> {
    app_state()
        .map(|app| app.exe_dir.clone())
        .ok_or_else(|| AppError::Msg("应用状态不可用".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(|| {
            let _guard = acquire_busy().expect("panic 测试应能获取");
            panic!("测试用 panic");
        });
        std::panic::set_hook(prev_hook);
        assert!(result.is_err(), "闭包应当 panic");
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
