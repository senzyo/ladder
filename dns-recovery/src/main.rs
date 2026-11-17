#![windows_subsystem = "windows"]

// 与桌面程序共享恢复格式和日志, 链接器移除助手用不到的功能。
#[allow(dead_code)]
#[path = "../../src/core.rs"]
mod core;
#[allow(dead_code)]
#[path = "../../src/dns.rs"]
mod dns;
#[allow(dead_code)]
#[path = "../../src/i18n.rs"]
mod i18n;
#[allow(dead_code)]
#[path = "../../src/logging.rs"]
mod logging;

use std::ffi::{OsStr, OsString};
use std::fs::{File, TryLockError};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::process::ExitCode;

use windows_sys::Win32::UI::Shell::{CSIDL_PROGRAM_FILES, SHGetFolderPathW};

use dns::DnsLease;

const BOOT_ARGUMENT: &str = "--restore-dns-on-boot";

fn recovery_directory() -> Result<PathBuf, String> {
    let mut buffer = [0u16; 260];
    // SHGetFolderPathW 要求 MAX_PATH 个字符, 成功时写入以 NUL 结尾的路径。
    let status = unsafe {
        SHGetFolderPathW(
            std::ptr::null_mut(),
            CSIDL_PROGRAM_FILES as i32,
            std::ptr::null_mut(),
            0,
            buffer.as_mut_ptr(),
        )
    };
    if status < 0 {
        return Err(format!("无法读取 Windows 系统目录 (HRESULT {status:#x})"));
    }
    let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Ok(PathBuf::from(OsString::from_wide(&buffer[..length])).join("Ladder DNS Recovery"))
}

fn restore_on_boot_with(lease: &DnsLease) -> Result<(), String> {
    let (directory, _) = lease.recovery_paths();
    let session = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("dns-session.lock"))
        .map_err(|error| format!("无法打开 DNS 恢复锁: {error}"))?;
    match session.try_lock() {
        Ok(()) => {
            // 锁定后才覆盖日志, 活跃桌面会话持有快照时不能读取或恢复。
            let _ = logging::init(&directory.join("app.log"));
            logging::info("dns", "开始开机 DNS 恢复");
            let result = lease.restore_if_pending();
            match &result {
                Ok(()) => logging::info("dns", "开机 DNS 恢复完成"),
                Err(message) => logging::error("dns", message),
            }
            result
        }
        Err(TryLockError::WouldBlock) => Ok(()),
        Err(TryLockError::Error(error)) => Err(format!("无法取得 DNS 恢复锁: {error}")),
    }
}

fn accepts_arguments(arguments: &[OsString]) -> bool {
    arguments.len() == 1 && arguments[0] == OsStr::new(BOOT_ARGUMENT)
}

fn main() -> ExitCode {
    i18n::set_current(i18n::Language::system());
    logging::install_panic_hook();
    if !accepts_arguments(&std::env::args_os().skip(1).collect::<Vec<_>>()) {
        return ExitCode::FAILURE;
    }
    match recovery_directory()
        .and_then(|directory| restore_on_boot_with(&DnsLease::new(&directory)))
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn requires_only_the_boot_argument() {
        assert!(accepts_arguments(&[BOOT_ARGUMENT.into()]));
        assert!(!accepts_arguments(&[]));
        assert!(!accepts_arguments(&["--unknown".into()]));
        assert!(!accepts_arguments(&[BOOT_ARGUMENT.into(), "extra".into()]));
    }

    #[test]
    fn boot_respects_session_lock_and_preserves_failed_records() {
        const CHILD_DIRECTORY: &str = "LADDER_TEST_STANDALONE_BOOT_DIRECTORY";
        if let Some(directory) = std::env::var_os(CHILD_DIRECTORY) {
            let directory = Path::new(&directory);
            let journal = directory.join("dns-recovery.json");
            let log = directory.join("app.log");
            fs::write(&journal, b"active session record").unwrap();
            fs::write(&log, b"active session log").unwrap();
            let desktop = File::options()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(directory.join("dns-session.lock"))
                .unwrap();
            desktop.lock().unwrap();
            let lease = DnsLease::for_tests(directory);

            restore_on_boot_with(&lease).unwrap();
            assert_eq!(fs::read(&journal).unwrap(), b"active session record");
            assert_eq!(fs::read(&log).unwrap(), b"active session log");
            drop(desktop);

            // 空快照及模拟网卡恢复都不访问本机 DNS。
            fs::write(&journal, b"[]").unwrap();
            restore_on_boot_with(&lease).unwrap();
            assert!(!journal.exists());
            lease.apply().unwrap();
            restore_on_boot_with(&lease).unwrap();
            assert!(!journal.exists());
            let completed = fs::read_to_string(&log).unwrap();
            assert!(completed.contains("开始开机 DNS 恢复"));
            assert!(completed.contains("开机 DNS 恢复完成"));
            assert!(!completed.contains("active session log"));

            fs::write(&journal, b"broken journal").unwrap();
            assert!(restore_on_boot_with(&lease).is_err());
            assert_eq!(fs::read(&journal).unwrap(), b"broken journal");
            assert!(fs::read_to_string(log).unwrap().contains("格式无效"));
            return;
        }

        // 恢复入口初始化全局日志, 因此在独立子进程验证。
        let directory = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::boot_respects_session_lock_and_preserves_failed_records",
                "--nocapture",
            ])
            .env(CHILD_DIRECTORY, directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
