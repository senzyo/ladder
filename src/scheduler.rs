//! 机器级 DNS 恢复。SYSTEM 只访问 Program Files 下的程序和快照。

use std::ffi::OsString;
use std::fs::{self, File, TryLockError};
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, MapViewOfFile, PAGE_READONLY, UnmapViewOfFile,
};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
use windows_sys::Win32::UI::Shell::{CSIDL_PROGRAM_FILES, CSIDL_SYSTEM, SHGetFolderPathW};

use crate::dns::DnsLease;

pub(crate) const BOOT_ARGUMENT: &str = "--restore-dns-on-boot";
const TASK_NAME: &str = "Reset_DNS_On_Boot";

fn system_folder(id: u32) -> Result<PathBuf> {
    let mut buffer = [0u16; 260];
    // SHGetFolderPathW 要求 MAX_PATH 个字符，成功时返回以 NUL 结尾的绝对路径。
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
        bail!("无法读取 Windows 系统目录（HRESULT {status:#x}）");
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
        .context("无法验证机器级 DNS 恢复目录")?;
    let reason = match status.code() {
        Some(0) => return Ok(()),
        Some(2) => "目录是重解析点",
        Some(3) => "目录权限不符合 SYSTEM 任务要求",
        Some(4) => "恢复目录包含非预期文件、子目录或重解析点",
        Some(5) => "恢复文件权限不符合 SYSTEM 任务要求",
        Some(code) => bail!(
            "无法创建或检查 {}（Windows 错误 {code:#010x}）",
            directory.display()
        ),
        None => bail!("无法检查 {}（{status}）", directory.display()),
    };
    bail!("{}：{reason}", directory.display())
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
            Err(error) => return Err(anyhow::anyhow!("无法取得 DNS 恢复锁：{error}")),
        }
    }
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

// 目录别名也可能指向同一份快照。无法确认便携目录独立时，等待用户修复机器目录。
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
        (
            DesktopDns::Unavailable(format!(
                "无法安全访问机器级 DNS 恢复记录。请检查恢复目录，或等待占用结束后重试：{error:#}"
            )),
            None,
        )
    };
    let directory = match recovery_directory() {
        Ok(path) => path,
        Err(error) => return unavailable(error),
    };
    if let Err(error) = prepare_directory(&directory) {
        return if can_use_portable(&directory, portable) {
            (
                DesktopDns::Portable(DnsLease::new(portable)),
                Some(format!(
                    "本次使用便携 DNS 恢复记录，只能由 Ladder 恢复。若旧版开机任务仍在，它可能会将 DNS 重置为自动获取。{error:#}"
                )),
            )
        } else {
            unavailable(error)
        };
    }
    let session = match session_file(&directory).context("无法打开 DNS 恢复锁") {
        Ok(file) => file,
        Err(error) => return unavailable(error),
    };
    if let Err(error) = lock_desktop_session(&session) {
        return unavailable(error);
    }
    let warning = std::env::current_exe()
        .context("无法定位 Ladder 程序")
        .and_then(|exe| ensure_boot_dns_reset_task(&exe, &directory))
        .err()
        .map(|error| format!(
            "更新开机 DNS 恢复任务 Reset_DNS_On_Boot 失败。若旧版任务仍在，它可能在开机时把 DNS 重置为自动获取。{error:#}"
        ));
    (
        DesktopDns::Machine {
            lease: DnsLease::with_legacy(&directory, portable),
            _session: session,
        },
        warning,
    )
}

pub(crate) fn restore_on_boot() -> Result<()> {
    let directory = recovery_directory()?;
    let session = session_file(&directory)?;
    match session.try_lock() {
        Ok(()) => DnsLease::new(&directory)
            .restore_if_pending()
            .map_err(anyhow::Error::msg),
        // 已经打开的 Ladder 会在启动时恢复，之后由它独占正在运行的核心快照。
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

fn running_image(exe: &Path) -> Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    let source = File::options()
        .read(true)
        .share_mode(1 /* FILE_SHARE_READ */)
        .open(exe)?;
    // 文件名所在的便携目录可以被移动。确认同一个文件对象后，从此句柄读取 SYSTEM 副本。
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtAreMappedFilesTheSame(
            image: *const std::ffi::c_void,
            data: *const std::ffi::c_void,
        ) -> i32;
    }
    // 仅映射文件开头用于查询内核对象身份，不执行或解引用其中的内容。
    let same = unsafe {
        let mapping = CreateFileMappingW(
            source.as_raw_handle(),
            std::ptr::null(),
            PAGE_READONLY,
            0,
            0,
            std::ptr::null(),
        );
        if mapping.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 1);
        CloseHandle(mapping);
        if view.Value.is_null() {
            bail!("无法映射开机 DNS 恢复程序");
        }
        let status = NtAreMappedFilesTheSame(GetModuleHandleW(std::ptr::null()), view.Value);
        UnmapViewOfFile(view);
        status == 0
    };
    if !same {
        bail!("Ladder 程序路径已变化，无法核实开机 DNS 恢复程序");
    }
    Ok(source)
}

fn ensure_boot_dns_reset_task(exe: &Path, directory: &Path) -> Result<()> {
    let helper = directory.join("ladder-dns-recovery.exe");
    // 从运行中的映像复制内容，新文件继承受保护目录的权限。完整写入后才替换旧版本。
    let staged = helper.with_extension("tmp");
    {
        let mut source = running_image(exe)?;
        let mut dest = File::create(&staged)?;
        io::copy(&mut source, &mut dest).context("复制开机 DNS 恢复程序失败")?;
        dest.sync_all()?;
    }
    fs::rename(&staged, &helper).context("更新开机 DNS 恢复程序失败")?;

    let xml = directory.join("dns-task.xml");
    let contents: Vec<_> = std::iter::once(0xfeff)
        .chain(task_xml(&helper).encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect();
    fs::write(&xml, contents)?;
    let system = system_folder(CSIDL_SYSTEM)?;
    // /F 原地替换旧版的同名任务，确保旧版重置为自动获取的动作不再生效。
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
    let status = status.context("无法启动 Windows 任务计划工具")?;
    if !status.success() {
        bail!("任务计划工具返回 {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_source_must_be_the_running_image() {
        let exe = std::env::current_exe().unwrap();
        running_image(&exe).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let copy = temp.path().join("other.exe");
        fs::write(&copy, b"test fixture").unwrap();
        assert!(running_image(&copy).is_err());
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
        // 用测试账户替代该组，保留 Windows 对继承项及 OWNER RIGHTS 的实际处理。
        let script = include_str!("recovery_directory.ps1")
            .replace("O:BAD:P", "D:P")
            .replace(
                ";;;BA)",
                ";;;' + [Security.Principal.WindowsIdentity]::GetCurrent().User.Value + ')",
            );
        check_directory(&machine, &script).unwrap();
        fs::write(machine.join("dns-recovery.json"), b"[]").unwrap();
        check_directory(&machine, &script).unwrap();
    }

    #[test]
    fn boot_leaves_an_active_session_alone_and_retries_after_exit() {
        let dir = tempfile::tempdir().unwrap();
        let desktop = session_file(dir.path()).unwrap();
        desktop.lock().unwrap();
        let boot = session_file(dir.path()).unwrap();
        assert!(matches!(boot.try_lock(), Err(TryLockError::WouldBlock)));
        drop(desktop);
        boot.try_lock().unwrap();
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
