//! 状态中毒终止与 panic 钩子共用的无日志紧急清理。

use crate::core::Core;
use crate::{dns, process};

/// 清理步骤不读取应用状态或 Child, 按核心名称尝试停止后再恢复网络。
enum CleanupStep {
    StopCore(Core),
    RestoreDns,
    FlushDns,
}

fn cleanup_with(mut perform: impl FnMut(CleanupStep)) {
    for core in Core::ALL {
        perform(CleanupStep::StopCore(core));
    }
    perform(CleanupStep::RestoreDns);
    perform(CleanupStep::FlushDns);
}

/// 全链路不取应用状态锁、不调用 tracing 或 UI。系统调用失败时尽力继续。
fn cleanup() {
    cleanup_with(|step| match step {
        CleanupStep::StopCore(core) => {
            process::kill_processes_by_name_without_logging(core.exe_name());
        }
        CleanupStep::RestoreDns => dns::restore_dns_to_dhcp_without_logging(),
        CleanupStep::FlushDns => process::flush_dns_without_logging(),
    });
}

/// 调用前必须已释放中毒错误携带的锁守卫; 不读取中毒状态, 不通过 panic 间接清理。
pub(crate) fn terminate_after_state_poison() -> ! {
    terminate_with(cleanup)
}

fn terminate_with(cleanup: fn()) -> ! {
    cleanup();
    std::process::exit(1);
}

/// 安装 panic 钩子, 在进程终止前恢复被本程序改动的系统状态。
///
/// release 构建用 `panic = "abort"` (原因见 Cargo.toml) , panic 不展开, Drop 和
/// catch_unwind 都不会执行。但 panic 钩子会在 abort 之前被调用, 所以把必须做的
/// 清理放在这里。
///
/// 关键是物理网卡 DNS: xray 的 TUN 模式期间它被劫持到 127.0.0.1, 进程直接消失
/// 的话用户既上不了网、又无从察觉原因, 只能等下次启动或开机任务恢复。
///
/// 钩子里不取应用状态锁或写 tracing, 防止 panic 发生在持锁期间时重入死锁。
/// 同理不弹 Toast 或 MessageBox。清理后仍调用安装前的默认诊断钩子。
/// 清理是尽力尝试, 不保证系统调用、再次 panic 或系统直接终止时一定完成。
pub(crate) fn install_panic_hook() {
    install_panic_hook_with(cleanup);
}

fn install_panic_hook_with(cleanup: fn()) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        cleanup();
        default_hook(info);
    }));
}

/// 仅供独立测试进程使用。所有清理入口都注入文件记录替身, 不调用生产清理。
#[cfg(test)]
pub(crate) mod test_support {
    use super::{CleanupStep, cleanup_with, terminate_with};
    use crate::core::Core;
    use std::fs::{self, File, OpenOptions};
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    const CHILD_TEST: &str = "LADDER_ISOLATED_EMERGENCY_TEST";
    const RECORD_PATH: &str = "LADDER_ISOLATED_EMERGENCY_RECORD";

    pub(crate) struct ChildResult {
        pub status: ExitStatus,
        pub records: Vec<String>,
        pub stderr: String,
        pub stdout: String,
    }

    // 即使父测试在等待途中失败, 也回收自己的测试子进程。
    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if !matches!(self.0.try_wait(), Ok(Some(_))) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    #[must_use]
    pub(crate) fn is_child(test_name: &str) -> bool {
        std::env::var(CHILD_TEST).as_deref() == Ok(test_name)
    }

    #[must_use = "必须检查测试子进程的退出状态和清理记录"]
    pub(crate) fn run_child(test_name: &str) -> ChildResult {
        let directory = tempfile::tempdir().expect("应能创建隔离进程测试目录");
        let records = directory.path().join("steps.txt");
        let stdout = directory.path().join("stdout.txt");
        let stderr = directory.path().join("stderr.txt");
        fs::write(&records, "").expect("应能创建清理步骤记录文件");
        let mut child = ChildGuard(
            Command::new(std::env::current_exe().expect("应能定位当前测试程序"))
                .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
                .env(CHILD_TEST, test_name)
                .env(RECORD_PATH, &records)
                .stdin(Stdio::null())
                .stdout(File::create(&stdout).expect("应能创建子进程 stdout 文件"))
                .stderr(File::create(&stderr).expect("应能创建子进程 stderr 文件"))
                .spawn()
                .expect("应能启动当前测试程序的独立子进程"),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.0.try_wait().expect("应能查询测试子进程状态") {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.0.kill();
                child.0.wait().expect("超时后应能回收测试子进程");
                panic!("隔离测试超时, 可能发生锁重入: {test_name}");
            }
            thread::sleep(Duration::from_millis(10));
        };
        ChildResult {
            status,
            records: fs::read_to_string(records)
                .expect("应能读取清理步骤记录")
                .lines()
                .map(str::to_owned)
                .collect(),
            stderr: fs::read_to_string(stderr).expect("应能读取子进程 stderr"),
            stdout: fs::read_to_string(stdout).expect("应能读取子进程 stdout"),
        }
    }

    pub(crate) fn record(step: &str) {
        use std::io::Write;

        let path = std::env::var_os(RECORD_PATH).expect("记录替身仅能在隔离测试子进程运行");
        let mut file = OpenOptions::new()
            .append(true)
            .open(path)
            .expect("应能打开清理步骤记录文件");
        writeln!(file, "{step}").expect("应能记录清理步骤");
    }

    pub(super) fn record_cleanup() {
        cleanup_with(|step| {
            record(match step {
                CleanupStep::StopCore(Core::SingBox) => "stop-sing-box",
                CleanupStep::StopCore(Core::Xray) => "stop-xray",
                CleanupStep::RestoreDns => "restore-dhcp-dns",
                CleanupStep::FlushDns => "flush-dns",
            });
        });
    }

    pub(crate) fn terminate_with_recorded_cleanup() -> ! {
        terminate_with(record_cleanup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::settings::Settings;
    use crate::tray::StatusIcons;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tracing::{Event, Subscriber};
    use tracing_subscriber::layer::{Context, Layer};

    /// 钩子若误写 tracing, 就会重入当前持有的日志锁; 父进程负责超时回收。
    struct LockedLogLayer(Arc<Mutex<()>>);

    impl<S: Subscriber> Layer<S> for LockedLogLayer {
        fn on_event(&self, _event: &Event<'_>, _context: Context<'_, S>) {
            let _guard = self.0.lock().expect("测试日志锁不应中毒");
            test_support::record("unexpected-tracing-event");
        }
    }

    #[test]
    fn test_panic_hook_cleans_before_default_with_locks_held() {
        use tracing_subscriber::layer::SubscriberExt;

        const TEST_NAME: &str =
            "emergency::tests::test_panic_hook_cleans_before_default_with_locks_held";
        if !test_support::is_child(TEST_NAME) {
            let child = test_support::run_child(TEST_NAME);
            assert_eq!(
                child.status.code(),
                Some(101),
                "{}\n{}",
                child.stdout,
                child.stderr
            );
            assert_eq!(
                child.records,
                [
                    "stop-sing-box",
                    "stop-xray",
                    "restore-dhcp-dns",
                    "flush-dns",
                    "default-hook"
                ],
                "必须先完成四步清理替身再调用原诊断钩子"
            );
            assert!(
                child.stderr.contains("isolated-panic-with-locks-held"),
                "原默认钩子必须输出 panic 诊断: {}",
                child.stderr
            );
            return;
        }

        let state = Mutex::new(AppState {
            exe_dir: PathBuf::new(),
            icons: StatusIcons::default(),
            settings: Settings::default(),
            child_sing_box: None,
            child_xray: None,
        });
        let log_lock = Arc::new(Mutex::new(()));
        let subscriber = tracing_subscriber::registry().with(LockedLogLayer(Arc::clone(&log_lock)));
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            test_support::record("default-hook");
            default_hook(info);
        }));
        install_panic_hook_with(test_support::record_cleanup);

        tracing::subscriber::with_default(subscriber, || {
            let _state = state.lock().expect("局部应用状态应可锁定");
            let _log = log_lock.lock().expect("局部日志状态应可锁定");
            panic!("isolated-panic-with-locks-held");
        });
    }
}
