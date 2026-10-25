//! panic 钩子与进程终止前的清理入口。

use std::thread;
use tracing::error;

use crate::process;

/// 安装 panic 钩子, 在进程终止前恢复被本程序改动的系统状态。
///
/// release 构建用 `panic = "abort"` (原因见 Cargo.toml) , panic 不展开, Drop 和
/// catch_unwind 都不会执行。但 panic 钩子会在 abort 之前被调用, 所以把必须做的
/// 清理放在这里。
///
/// 关键是物理网卡 DNS: xray 的 TUN 模式期间它被劫持到 127.0.0.1, 进程直接消失
/// 的话用户既上不了网、又无从察觉原因, 只能等下次启动或开机任务恢复。
///
/// 钩子里刻意不调用 `process::stop_all`: 那条路径要取 `AppState` 的 Mutex, 而
/// panic 完全可能发生在持锁期间, 再取同一把锁就是自死锁。改用不涉及 Mutex 的
/// `kill_cores_without_state`。同理也不弹 Toast: 那要走 COM, 且失败时会回退到
/// 阻塞式 MessageBox, 在即将 abort 的进程里风险大于收益。
pub(crate) fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = thread::current();
        let name = thread.name().unwrap_or("<未命名>");
        error!("线程 [{name}] panic, 终止前恢复网络状态: {info}");
        process::kill_cores_without_state();
        default_hook(info);
    }));
}
