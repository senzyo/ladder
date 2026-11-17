//! 机器级 DNS 恢复。SYSTEM 只访问 Program Files 下的程序和快照。

use crate::i18n::system as t;

use std::ffi::OsString;
use std::fs::{self, File, TryLockError};
use std::io::{self, Write};
use std::os::windows::ffi::OsStringExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
use windows_sys::Win32::UI::Shell::{CSIDL_PROGRAM_FILES, CSIDL_SYSTEM, SHGetFolderPathW};

use crate::dns::DnsLease;
use crate::logging;

pub(crate) const BOOT_ARGUMENT: &str = "--restore-dns-on-boot";
const TASK_NAME: &str = "Reset_DNS_On_Boot";
const RECOVERY_EXECUTABLE: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/ladder-dns-recovery.exe"));

fn system_folder(id: u32) -> Result<PathBuf> {
    let mut buffer = [0u16; 260];
    // SHGetFolderPathW 要求 MAX_PATH 个字符, 成功时返回以 NUL 结尾的绝对路径。
    let status = unsafe {
        SHGetFolderPathW(
            std::ptr::null_mut(),
            id as i32,
            std::ptr::null_mut(),
            0,
            buffer.as_mut_ptr(),
        )
    };
    if status < 0 {
        bail!(t::system_folder_failed(status));
    }
    let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Ok(OsString::from_wide(&buffer[..length]).into())
}

fn recovery_directory() -> Result<PathBuf> {
    Ok(system_folder(CSIDL_PROGRAM_FILES)?.join("Ladder DNS Recovery"))
}

fn prepare_directory(directory: &Path) -> Result<()> {
    check_directory(directory, include_str!("recovery_directory.ps1"))
}

fn check_directory(directory: &Path, script: &str) -> Result<()> {
    let system = system_folder(CSIDL_SYSTEM)?;
    let status = Command::new(system.join(r"WindowsPowerShell\v1.0\powershell.exe"))
        .current_dir(&system)
        .env("LADDER_DNS_RECOVERY_DIRECTORY", directory)
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context(t::recovery_directory_verify())?;
    let reason = match status.code() {
        Some(0) => return Ok(()),
        Some(2) => t::directory_reparse_point(),
        Some(3) => t::directory_permissions(),
        Some(4) => t::directory_unexpected_contents(),
        Some(5) => t::recovery_file_permissions(),
        Some(_) => bail!(t::recovery_directory_check(directory.display())),
        None => bail!(t::recovery_check_interrupted(directory.display())),
    };
    bail!("{}: {reason}", directory.display())
}

fn session_file(directory: &Path) -> io::Result<File> {
    File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("dns-session.lock"))
}

fn lock_desktop_session(session: &File) -> Result<()> {
    // 正常开机时等正在运行的 SYSTEM 恢复交还快照；长期占用时仍能进入普通核心界面。
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match session.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(anyhow::anyhow!(t::dns_acquire_lock(error))),
        }
    }
}

fn portable_dns(directory: &Path) -> Result<DnsLease> {
    let session = session_file(directory).context(t::dns_open_portable_lock())?;
    lock_desktop_session(&session)?;
    Ok(DnsLease::new(directory).with_session(session))
}

pub(crate) enum DesktopDns {
    Machine { lease: DnsLease, _session: File },
    Portable(DnsLease),
    Unavailable(String),
}

impl DesktopDns {
    pub(crate) fn lease(&self) -> Result<&DnsLease, String> {
        match self {
            Self::Machine { lease, .. } | Self::Portable(lease) => Ok(lease),
            Self::Unavailable(error) => Err(error.clone()),
        }
    }
}

// 目录别名也可能指向同一份快照。无法确认便携目录独立时, 等待用户修复机器目录。
fn can_use_portable(directory: &Path, portable: &Path) -> bool {
    match fs::symlink_metadata(directory) {
        Err(error) => error.kind() == io::ErrorKind::NotFound,
        Ok(_) => {
            matches!(same_file::is_same_file(directory, portable), Ok(false))
                && matches!(fs::symlink_metadata(directory.join("dns-recovery.json")),
                    Err(error) if error.kind() == io::ErrorKind::NotFound)
        }
    }
}

pub(crate) fn desktop_dns(portable: &Path) -> (DesktopDns, Option<String>) {
    let unavailable = |error: anyhow::Error| {
        logging::warn("dns", format_args!("机器级 DNS 恢复不可用: {error:#}"));
        (
            DesktopDns::Unavailable(t::dns_machine_unavailable(error)),
            None,
        )
    };
    let directory = match recovery_directory() {
        Ok(path) => path,
        Err(error) => return unavailable(error),
    };
    if let Err(error) = prepare_directory(&directory) {
        return if can_use_portable(&directory, portable) {
            let lease = match portable_dns(portable) {
                Ok(lease) => lease,
                Err(error) => return unavailable(error),
            };
            logging::warn("dns", format_args!("使用便携 DNS 恢复记录: {error:#}"));
            (
                DesktopDns::Portable(lease),
                Some(t::dns_portable_warning(error)),
            )
        } else {
            unavailable(error)
        };
    }
    let session = match session_file(&directory).context(t::dns_open_lock()) {
        Ok(file) => file,
        Err(error) => return unavailable(error),
    };
    if let Err(error) = lock_desktop_session(&session) {
        return unavailable(error);
    }
    let warning = ensure_boot_dns_reset_task(&directory)
        .err()
        .map(t::dns_boot_task_update);
    if let Some(message) = &warning {
        logging::warn("dns", message);
    } else {
        logging::info("dns", "机器级 DNS 恢复目录与开机任务已就绪");
    }
    (
        DesktopDns::Machine {
            lease: DnsLease::with_legacy(&directory, portable),
            _session: session,
        },
        warning,
    )
}

pub(crate) fn restore_on_boot() -> Result<()> {
    restore_on_boot_with(&DnsLease::new(&recovery_directory()?))
}

fn restore_on_boot_with(lease: &DnsLease) -> Result<()> {
    let (directory, _) = lease.recovery_paths();
    let session = session_file(directory)?;
    match session.try_lock() {
        Ok(()) => {
            // 取得快照锁后才覆盖本次日志, 避免与其他恢复进程争用文件。
            let _ = logging::init(&directory.join("app.log"));
            logging::info("dns", "开始开机 DNS 恢复");
            let result = lease.restore_if_pending();
            match &result {
                Ok(()) => logging::info("dns", "开机 DNS 恢复完成"),
                Err(message) => logging::error("dns", message),
            }
            result.map_err(anyhow::Error::msg)
        }
        // 已经打开的 Ladder 会在启动时恢复, 之后由它独占正在运行的核心快照。
        Err(TryLockError::WouldBlock) => Ok(()),
        Err(TryLockError::Error(error)) => Err(error.into()),
    }
}

pub(crate) fn restore_after_panic(directory: &Path, legacy: Option<&Path>) -> Result<()> {
    let session = session_file(directory)?;
    match session.try_lock() {
        Ok(()) => {
            #[cfg(not(test))]
            let lease = match legacy {
                Some(portable) => DnsLease::with_legacy(directory, portable),
                None => DnsLease::new(directory),
            };
            #[cfg(test)]
            let lease = DnsLease::for_tests_with_legacy(directory, legacy);
            lease.restore_if_pending().map_err(anyhow::Error::msg)
        }
        // 新桌面会话先取得锁时由它接管恢复, 不能重放其后产生的记录。
        Err(TryLockError::WouldBlock) => Ok(()),
        Err(TryLockError::Error(error)) => Err(error.into()),
    }
}

fn task_xml(helper: &Path) -> String {
    let command = helper.to_string_lossy().replace('&', "&amp;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><BootTrigger><Enabled>true</Enabled></BootTrigger></Triggers>
  <Principals><Principal id="System"><UserId>S-1-5-18</UserId></Principal></Principals>
  <Settings>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure>
  </Settings>
  <Actions Context="System"><Exec><Command>{command}</Command><Arguments>{BOOT_ARGUMENT}</Arguments></Exec></Actions>
</Task>"#
    )
}

fn install_recovery_helper(directory: &Path) -> Result<PathBuf> {
    let helper = directory.join("ladder-dns-recovery.exe");
    // 内嵌程序来自运行中的映像, 新文件继承受保护目录的权限。
    let staged = helper.with_extension("tmp");
    {
        let mut dest = File::create(&staged)?;
        dest.write_all(RECOVERY_EXECUTABLE)
            .context(t::recovery_helper_write())?;
        dest.sync_all()?;
    }
    fs::rename(&staged, &helper).context(t::recovery_helper_update())?;
    Ok(helper)
}

fn ensure_boot_dns_reset_task(directory: &Path) -> Result<()> {
    let helper = install_recovery_helper(directory)?;
    let xml = directory.join("dns-task.xml");
    let contents: Vec<_> = std::iter::once(0xfeff)
        .chain(task_xml(&helper).encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect();
    fs::write(&xml, contents)?;
    let system = system_folder(CSIDL_SYSTEM)?;
    // 重复启动时更新同一任务, 保持恢复程序和开机动作一致。
    let status = Command::new(system.join("schtasks.exe"))
        .current_dir(&system)
        .args(["/Create", "/TN", TASK_NAME, "/XML"])
        .arg(&xml)
        .arg("/F")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = fs::remove_file(xml);
    let status = status.context(t::task_scheduler_start())?;
    if !status.success() {
        bail!(t::dns_task_register());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_recovery_helper_is_small() {
        let directory = tempfile::tempdir().unwrap();
        let helper = install_recovery_helper(directory.path()).unwrap();
        let size = fs::metadata(&helper).unwrap().len();
        assert!(size < 2 * 1024 * 1024, "DNS 恢复程序过大: {size} 字节");
        // 无效参数必须直接退出, 不能启动桌面或访问真实恢复目录。
        let status = Command::new(helper)
            .arg("--invalid-test-argument")
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(1));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn existing_unprotected_directory_is_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let backup = temp.path().join("dns-recovery.json.bak");
        fs::write(&backup, b"saved").unwrap();
        assert!(
            prepare_directory(temp.path())
                .unwrap_err()
                .to_string()
                .contains("SYSTEM")
        );
        assert_eq!(fs::read(&backup).unwrap(), b"saved");
    }

    #[test]
    fn portable_fallback_requires_a_distinct_directory_and_no_machine_record() {
        let portable = tempfile::tempdir().unwrap();
        let machine = tempfile::tempdir().unwrap();
        assert!(can_use_portable(machine.path(), portable.path()));
        assert!(!can_use_portable(machine.path(), machine.path()));
        fs::write(machine.path().join("dns-recovery.json"), b"saved").unwrap();
        assert!(!can_use_portable(machine.path(), portable.path()));
    }

    #[test]
    fn accepts_ntfs_order_for_directory_and_inherited_file_permissions() {
        let temp = tempfile::tempdir().unwrap();
        let machine = temp.path().join("machine");
        // 普通测试令牌不能指派 Administrators 所有者或使用其权限创建文件。
        // 用测试账户替代该组, 保留 Windows 对继承项及 OWNER RIGHTS 的实际处理。
        let script = include_str!("recovery_directory.ps1")
            .replace("O:BAD:P", "D:P")
            .replace(
                ";;;BA)",
                ";;;' + [Security.Principal.WindowsIdentity]::GetCurrent().User.Value + ')",
            )
            .replace(
                "function Sorted-Aces",
                // Windows 会将内置管理员测试账户的原始 SID 规范为 LA。
                "$descriptor = [Security.AccessControl.RawSecurityDescriptor]::new('D:' + $aces)\n\
                 $aces = $descriptor.GetSddlForm([Security.AccessControl.AccessControlSections]::Access).Substring(2)\n\
                 function Sorted-Aces",
            );
        check_directory(&machine, &script).unwrap();
        fs::write(machine.join("dns-recovery.json"), b"[]").unwrap();
        fs::write(machine.join("app.log"), b"test session\n").unwrap();
        check_directory(&machine, &script).unwrap();
    }

    #[test]
    fn boot_leaves_an_active_session_alone_and_retries_after_exit() {
        const CHILD_DIRECTORY: &str = "LADDER_TEST_BOOT_RECOVERY_DIRECTORY";
        if let Some(directory) = std::env::var_os(CHILD_DIRECTORY) {
            let directory = Path::new(&directory);
            let journal = directory.join("dns-recovery.json");
            let log = directory.join("app.log");
            fs::write(&journal, b"do not read an active session's journal").unwrap();
            fs::write(&log, b"active session\n").unwrap();
            let desktop = session_file(directory).unwrap();
            desktop.lock().unwrap();
            let lease = DnsLease::for_tests(directory);

            restore_on_boot_with(&lease).unwrap();
            assert_eq!(
                fs::read(&journal).unwrap(),
                b"do not read an active session's journal"
            );
            assert_eq!(fs::read(&log).unwrap(), b"active session\n");

            drop(desktop);
            fs::write(&journal, b"[]").unwrap();
            lease.apply().unwrap();
            restore_on_boot_with(&lease).unwrap();
            assert!(!journal.exists());
            let log = fs::read_to_string(log).unwrap();
            assert!(log.contains("开始开机 DNS 恢复"));
            assert!(log.contains("开机 DNS 恢复完成"));
            assert!(!log.contains("active session"));
            return;
        }

        // 开机恢复会初始化全局日志, 在独立进程验证以免干扰其他测试。
        let dir = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "scheduler::tests::boot_leaves_an_active_session_alone_and_retries_after_exit",
                "--nocapture",
            ])
            .env(CHILD_DIRECTORY, dir.path())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn portable_lease_keeps_its_session_locked_until_drop() {
        let dir = tempfile::tempdir().unwrap();
        let lease = portable_dns(dir.path()).unwrap();
        assert_eq!(lease.recovery_paths(), (dir.path(), None));
        let other = session_file(dir.path()).unwrap();
        assert!(matches!(other.try_lock(), Err(TryLockError::WouldBlock)));
        drop(lease);
        other.try_lock().unwrap();
    }

    #[test]
    fn panic_recovery_defers_to_a_new_session_and_preserves_logs() {
        let dir = tempfile::tempdir().unwrap();
        let journal = dir.path().join("dns-recovery.json");
        let log = dir.path().join("app.log");
        fs::write(&journal, b"do not read an active session's journal").unwrap();
        fs::write(&log, b"panic location\n").unwrap();
        let desktop = session_file(dir.path()).unwrap();
        desktop.try_lock().unwrap();
        restore_after_panic(dir.path(), None).unwrap();
        assert_eq!(
            fs::read(&journal).unwrap(),
            b"do not read an active session's journal"
        );
        drop(desktop);

        // 空记录只需删除, 不调用真实 DNS 接口或刷新缓存。
        fs::write(&journal, b"[]").unwrap();
        restore_after_panic(dir.path(), None).unwrap();
        assert!(!journal.exists());
        assert_eq!(fs::read(log).unwrap(), b"panic location\n");
    }

    #[test]
    fn panic_recovery_cleans_empty_legacy_records_without_reinitializing_logging() {
        let machine = tempfile::tempdir().unwrap();
        let portable = tempfile::tempdir().unwrap();
        let old = portable.path().join("dns-recovery.json");
        fs::write(&old, b"[]").unwrap();
        restore_after_panic(machine.path(), Some(portable.path())).unwrap();
        assert!(!old.exists());
        assert!(!machine.path().join("dns-recovery.json").exists());
        assert!(!machine.path().join("app.log").exists());
    }

    #[test]
    fn desktop_waits_for_boot_recovery_to_finish() {
        let dir = tempfile::tempdir().unwrap();
        let boot = session_file(dir.path()).unwrap();
        boot.lock().unwrap();
        let desktop = session_file(dir.path()).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(boot);
        });
        lock_desktop_session(&desktop).unwrap();
        release.join().unwrap();
        let other = session_file(dir.path()).unwrap();
        assert!(matches!(other.try_lock(), Err(TryLockError::WouldBlock)));
    }

    #[test]
    fn boot_task_runs_only_the_protected_helper_as_system() {
        let xml = task_xml(Path::new(r"C:\Rut & 程序\ladder-dns-recovery.exe"));
        assert!(xml.contains("<BootTrigger>"));
        assert!(xml.contains("<UserId>S-1-5-18</UserId>"));
        assert!(xml.contains(r"<Command>C:\Rut &amp; 程序\ladder-dns-recovery.exe</Command>"));
        assert!(xml.contains("<Arguments>--restore-dns-on-boot</Arguments>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false"));
    }
}
