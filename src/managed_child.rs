use std::cmp::Ordering;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;
use std::ptr;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Globalization::{CSTR_EQUAL, CSTR_LESS_THAN, CompareStringOrdinal};
use windows_sys::Win32::System::Diagnostics::Debug::{
    GetThreadErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX,
    SetThreadErrorMode,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, UpdateProcThreadAttribute,
    WaitForSingleObject,
};

use crate::core::Core;
use crate::core_output::CoreOutput;
use crate::tun_config::PreparedConfig;

pub(crate) struct ManagedChild {
    process: JobProcess,
    pub(crate) output: CoreOutput,
    config: PreparedConfig,
}

pub(crate) struct JobProcess {
    job: OwnedHandle,
    process: OwnedHandle,
}

fn checked(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn create_job() -> io::Result<OwnedHandle> {
    // 匿名且不可继承, 最后一个句柄始终由 Ladder 持有。
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    checked(unsafe {
        SetInformationJobObject(
            raw,
            JobObjectExtendedLimitInformation,
            ptr::from_ref(&limits).cast(),
            size_of_val(&limits) as u32,
        )
    })?;
    Ok(job)
}

// Windows 的环境块按变量名 (忽略大小写) 排序, 以两个 NUL 结束。
fn xray_environment(assets: &Path) -> Vec<u16> {
    let name: Vec<_> = "XRAY_LOCATION_ASSET".encode_utf16().collect();
    let mut entries: Vec<_> = std::env::vars_os()
        .map(|(key, value)| (key.encode_wide().collect::<Vec<_>>(), value))
        .collect();
    let compare = |left: &[u16], right: &[u16]| match unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left.len() as i32,
            right.as_ptr(),
            right.len() as i32,
            1,
        )
    } {
        CSTR_LESS_THAN => Ordering::Less,
        CSTR_EQUAL => Ordering::Equal,
        _ => Ordering::Greater,
    };
    entries.retain(|(key, _)| compare(key, &name) != Ordering::Equal);
    entries.push((name, assets.as_os_str().to_owned()));
    entries.sort_unstable_by(|(left, _), (right, _)| compare(left, right));
    let mut block = Vec::new();
    for (key, value) in entries {
        block.extend(key);
        block.push(b'=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

struct AttributeList(Vec<usize>);

fn config_command(
    core: Core,
    exe: &Path,
    assets: &Path,
    config: &Path,
    validate: bool,
) -> OsString {
    let mut command = OsString::from("\"");
    command.push(exe);
    command.push(match (core, validate) {
        (Core::SingBox, true) => "\" check -D \"",
        (Core::SingBox, false) => "\" run -D \"",
        (Core::Xray, true) => "\" run -test",
        (Core::Xray, false) => "\" run",
        (Core::Mihomo, true) => "\" -t -d \"",
        (Core::Mihomo, false) => "\" -d \"",
    });
    if core != Core::Xray {
        command.push(if core == Core::Mihomo {
            config.parent().unwrap()
        } else {
            assets
        });
        command.push("\"");
    }
    command.push(if core == Core::Mihomo {
        " -f \""
    } else {
        " -c \""
    });
    command.push(config);
    command.push("\"");
    command
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast()) };
    }
}

impl ManagedChild {
    pub(crate) fn spawn(
        core: Core,
        root: &Path,
        exe: &Path,
        assets: &Path,
        config: PreparedConfig,
    ) -> io::Result<Self> {
        let stdin = File::open("NUL")?;
        let (stdout, stdout_write) = io::pipe()?;
        let (stderr, stderr_write) = io::pipe()?;
        let stdio = [
            stdin.as_raw_handle(),
            stdout_write.as_raw_handle(),
            stderr_write.as_raw_handle(),
        ];
        let command = config_command(core, exe, assets, config.path(), false);
        let env = (core == Core::Xray).then(|| xray_environment(assets));
        let process = JobProcess::spawn(exe, root, command, env.as_deref(), stdio)?;
        drop((stdin, stdout_write, stderr_write));
        Ok(Self {
            process,
            output: CoreOutput::capture(core, stdout, stderr),
            config,
        })
    }

    pub(crate) fn tun_names(&self) -> &[String] {
        self.config.names()
    }

    pub(crate) fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        self.process.try_wait()
    }

    pub(crate) fn terminate(&self) -> io::Result<()> {
        self.process.terminate()
    }
}

impl JobProcess {
    pub(crate) fn spawn_version(core: Core, exe: &Path, stdout: &File) -> io::Result<Self> {
        let root = exe
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "无法定位核心目录"))?;
        let stdin = File::open("NUL")?;
        let stdout = stdout.try_clone()?;
        let stderr = File::options().write(true).open("NUL")?;
        let stdio = [
            stdin.as_raw_handle(),
            stdout.as_raw_handle(),
            stderr.as_raw_handle(),
        ];
        let mut command = OsString::from("\"");
        command.push(exe);
        command.push(if core == Core::Mihomo {
            "\" -v"
        } else {
            "\" version"
        });
        Self::spawn(exe, root, command, None, stdio)
    }

    pub(crate) fn spawn_validation(
        core: Core,
        exe: &Path,
        root: &Path,
        config: &Path,
        output: &File,
    ) -> io::Result<Self> {
        let assets = exe
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "无法定位核心目录"))?;
        let stdin = File::open("NUL")?;
        let output = output.try_clone()?;
        let stderr = output.try_clone()?;
        let stdio = [
            stdin.as_raw_handle(),
            output.as_raw_handle(),
            stderr.as_raw_handle(),
        ];
        let env = (core == Core::Xray).then(|| xray_environment(assets));
        Self::spawn(
            exe,
            root,
            config_command(core, exe, assets, config, true),
            env.as_deref(),
            stdio,
        )
    }

    fn spawn(
        exe: &Path,
        root: &Path,
        command: OsString,
        env: Option<&[u16]>,
        stdio: [HANDLE; 3],
    ) -> io::Result<Self> {
        let job = create_job()?;
        let jobs = [job.as_raw_handle()];
        let wide = |value: &OsStr| value.encode_wide().chain(Some(0)).collect::<Vec<_>>();
        let (exe, root, mut command) = (
            wide(exe.as_os_str()),
            wide(root.as_os_str()),
            wide(&command),
        );
        // 初始化缓冲区按指针对齐。属性值及句柄保留到 CreateProcessW 返回。
        let mut bytes = 0;
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 2, 0, &mut bytes) };
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let list = storage.as_mut_ptr().cast();
        checked(unsafe { InitializeProcThreadAttributeList(list, 2, 0, &mut bytes) })?;
        let _attributes = AttributeList(storage);
        for (attribute, handles) in [
            (PROC_THREAD_ATTRIBUTE_JOB_LIST, jobs.as_slice()),
            (PROC_THREAD_ATTRIBUTE_HANDLE_LIST, stdio.as_slice()),
        ] {
            checked(unsafe {
                UpdateProcThreadAttribute(
                    list,
                    0,
                    attribute as usize,
                    handles.as_ptr().cast(),
                    size_of_val(handles),
                    ptr::null_mut(),
                    ptr::null(),
                )
            })?;
        }
        let startup = STARTUPINFOEXW {
            StartupInfo: STARTUPINFOW {
                cb: size_of::<STARTUPINFOEXW>() as u32,
                dwFlags: STARTF_USESTDHANDLES,
                hStdInput: stdio[0],
                hStdOutput: stdio[1],
                hStdError: stdio[2],
                ..Default::default()
            },
            lpAttributeList: list,
        };
        let mut info = PROCESS_INFORMATION::default();
        for handle in stdio {
            checked(unsafe {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT)
            })?;
        }
        let error_mode = unsafe { GetThreadErrorMode() }
            | SEM_FAILCRITICALERRORS
            | SEM_NOGPFAULTERRORBOX
            | SEM_NOOPENFILEERRORBOX;
        let mut previous_error_mode = 0;
        checked(unsafe { SetThreadErrorMode(error_mode, &mut previous_error_mode) })?;
        // 系统在新进程运行前完成 Job 归属。启动器在创建期间被强杀也不会遗留暂停的核心。
        let created = checked(unsafe {
            CreateProcessW(
                exe.as_ptr(),
                command.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                env.map_or(ptr::null(), |block| block.as_ptr().cast()),
                root.as_ptr(),
                &startup.StartupInfo,
                &mut info,
            )
        });
        // 只抑制此线程本次创建的系统错误对话, 不改变 GUI 或其他线程的错误模式。
        unsafe { SetThreadErrorMode(previous_error_mode, ptr::null_mut()) };
        created?;
        let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
        drop(unsafe { OwnedHandle::from_raw_handle(info.hThread) });
        Ok(Self { job, process })
    }

    pub(crate) fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        let process = self.process.as_raw_handle();
        match unsafe { WaitForSingleObject(process, 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                checked(unsafe { GetExitCodeProcess(process, &mut code) })?;
                Ok(Some(ExitStatus::from_raw(code)))
            }
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub(crate) fn terminate(&self) -> io::Result<()> {
        let job: HANDLE = self.job.as_raw_handle();
        checked(unsafe { TerminateJobObject(job, 1) })?;
        // 包括已关闭输出管道的后代。确认全部退出后才能重启核心和恢复 DNS。
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut counts = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            checked(unsafe {
                QueryInformationJobObject(
                    job,
                    JobObjectBasicAccountingInformation,
                    ptr::from_mut(&mut counts).cast(),
                    size_of_val(&counts) as u32,
                    ptr::null_mut(),
                )
            })?;
            if counts.ActiveProcesses == 0 && self.try_wait()?.is_some() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "等待托管进程退出超时, 请重试停止",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;

    #[test]
    fn validation_commands_use_official_flags_and_quote_paths() {
        let exe = Path::new(r"C:\Program Files\core.exe");
        let assets = Path::new(r"C:\Program Files\assets");
        let config = Path::new(r"C:\配置\daily config.json");
        for (core, command) in [
            (
                Core::SingBox,
                r#""C:\Program Files\core.exe" check -D "C:\Program Files\assets" -c "C:\配置\daily config.json""#,
            ),
            (
                Core::Xray,
                r#""C:\Program Files\core.exe" run -test -c "C:\配置\daily config.json""#,
            ),
            (
                Core::Mihomo,
                r#""C:\Program Files\core.exe" -t -d "C:\配置" -f "C:\配置\daily config.json""#,
            ),
        ] {
            assert_eq!(
                config_command(core, exe, assets, config, true),
                OsStr::new(command)
            );
        }
        assert_eq!(
            config_command(Core::Mihomo, exe, assets, config, false),
            OsStr::new(
                r#""C:\Program Files\core.exe" -d "C:\配置" -f "C:\配置\daily config.json""#
            )
        );
    }
}
