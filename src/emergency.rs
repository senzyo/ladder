//! panic 后由独立进程恢复 DNS，避免重入出错线程或与仍在运行的 DNS 写入竞争。

use std::ffi::OsString;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessId, TerminateProcess, WaitForSingleObject,
};

use crate::{logging, panic_helper::PanicHelper, scheduler};

pub(crate) const ARGUMENT: &str = "--restore-dns-after-panic";
static INSTALLED: AtomicBool = AtomicBool::new(false);
static PANICKING: AtomicBool = AtomicBool::new(false);
static RECOVERY: Mutex<Option<PanicHelper>> = Mutex::new(None);

pub(crate) fn install_panic_hook() {
    let log_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // 其他线程同时 panic 时，让首个线程先创建助手，避免提前杀掉它。
        if PANICKING.swap(true, Ordering::AcqRel) {
            std::thread::sleep(Duration::from_secs(5));
            terminate();
        }
        log_panic(info);
        if let Ok(mut recovery) = RECOVERY.try_lock()
            && let Some(command) = recovery.as_mut()
        {
            // 创建失败时保留快照，由下次启动或开机恢复。这里不再取日志锁或弹窗。
            let _ = command.spawn();
        }
        // 包括后台线程的 panic 都视为致命错误。退出也关闭托管核心的 Job 句柄。
        terminate();
    }));
    INSTALLED.store(true, Ordering::Release);
}

// 在正常执行路径准备参数与真实进程句柄，panic 中不读取应用状态或初始化恢复目录。
pub(crate) fn prepare(dns: &scheduler::DesktopDns) {
    if !INSTALLED.load(Ordering::Acquire) {
        return;
    }
    let command = dns
        .lease()
        .ok()
        .map(|lease| {
            let (directory, legacy) = lease.recovery_paths();
            let mut arguments = vec![OsString::from(ARGUMENT), directory.as_os_str().to_owned()];
            if let Some(legacy) = legacy {
                arguments.push(legacy.as_os_str().to_owned());
            }
            PanicHelper::prepare(&std::env::current_exe()?, &arguments)
        })
        .transpose();
    let (command, error) = match command {
        Ok(command) => (command, None),
        Err(error) => (None, Some(error)),
    };
    *RECOVERY.lock().unwrap_or_else(|e| e.into_inner()) = command;
    if let Some(error) = error {
        logging::warn(
            "dns",
            format_args!("无法准备 panic DNS 恢复，异常退出后需重新打开 Ladder：{error}"),
        );
    }
}

fn terminate() -> ! {
    // 不进入 Rust/C 退出清理或 DLL detach，避免出错线程遗留的锁挡住父进程终止。
    unsafe { TerminateProcess(GetCurrentProcess(), 1) };
    std::process::abort();
}

fn wait_for_parent() -> io::Result<()> {
    let parent = io::stdin().as_raw_handle();
    let pid = unsafe { GetProcessId(parent) };
    if pid == 0 || pid == std::process::id() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "缺少有效的父进程句柄",
        ));
    }
    // panic 进程应立即退出；无效调用不能留下无限等待的助手。
    if unsafe { WaitForSingleObject(parent, 30_000) } != WAIT_OBJECT_0 {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "等待异常进程退出失败",
        ));
    }
    Ok(())
}

pub(crate) fn restore_after_panic() -> anyhow::Result<()> {
    let mut arguments = std::env::args_os().skip(2);
    let directory = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("缺少 DNS 恢复目录"))?;
    let legacy = arguments.next().map(PathBuf::from);
    anyhow::ensure!(arguments.next().is_none(), "DNS 恢复参数过多");
    anyhow::ensure!(
        directory.is_absolute() && legacy.as_ref().is_none_or(|p| p.is_absolute()),
        "DNS 恢复目录必须是绝对路径"
    );
    wait_for_parent()?;
    scheduler::restore_after_panic(&directory, legacy.as_deref())
}

#[cfg(test)]
pub(crate) fn prepare_test_helper(fail_to_spawn: bool) {
    let exe = if fail_to_spawn {
        PathBuf::from(std::env::var_os("LADDER_TEST_PANIC_ROOT").unwrap())
            .join("missing-helper.exe")
    } else {
        std::env::current_exe().unwrap()
    };
    let helper = PanicHelper::prepare(
        &exe,
        &[
            "--exact".into(),
            "emergency::tests::recovery_helper_process".into(),
            "--nocapture".into(),
        ],
    )
    .unwrap();
    *RECOVERY.lock().unwrap() = Some(helper);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn recovery_helper_process() {
        let Some(root) = std::env::var_os("LADDER_TEST_PANIC_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        fs::write(root.join("helper-started"), []).unwrap();
        // 与生产助手一样，先等待原进程停止，再进入有会话锁的恢复入口。
        wait_for_parent().unwrap();
        let result = scheduler::restore_after_panic(&root, None);
        let completed = root.join("recovery-result.tmp");
        fs::write(&completed, if result.is_ok() { "ok" } else { "failed" }).unwrap();
        fs::rename(completed, root.join("recovery-result")).unwrap();
    }

    #[test]
    fn helper_waits_for_parent_before_touching_the_journal() {
        const ROOT: &str = "LADDER_TEST_PANIC_ROOT";
        if let Some(root) = std::env::var_os(ROOT) {
            let root = PathBuf::from(root);
            crate::dns::DnsLease::for_tests(&root).apply().unwrap();
            let session = fs::File::create(root.join("dns-session.lock")).unwrap();
            session.lock().unwrap();
            prepare_test_helper(false);
            RECOVERY.lock().unwrap().as_mut().unwrap().spawn().unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !root.join("helper-started").exists() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(10));
            }
            std::thread::sleep(Duration::from_millis(150));
            assert!(
                !root.join("recovery-result").exists(),
                "助手不能把仍存活的父进程当成已退出"
            );
            assert!(root.join("dns-recovery.json").exists());
            // 助手启动后父进程仍可能修改记录，必须恢复父进程退出时的最终版本。
            fs::write(root.join("dns-recovery.json"), b"last interrupted write").unwrap();
            terminate();
        }
        let folder = tempfile::Builder::new()
            .prefix("等待 panic ")
            .tempdir()
            .unwrap();
        let root = folder.path();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "emergency::tests::helper_waits_for_parent_before_touching_the_journal",
            ])
            .env(ROOT, root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("父进程未按时退出");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(child.wait().unwrap().code(), Some(1));
        while !root.join("recovery-result").exists() {
            assert!(std::time::Instant::now() < deadline, "助手未完成恢复");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            fs::read_to_string(root.join("recovery-result")).unwrap(),
            "failed"
        );
        assert_eq!(
            fs::read(root.join("dns-recovery.json")).unwrap(),
            b"last interrupted write"
        );
    }
}
