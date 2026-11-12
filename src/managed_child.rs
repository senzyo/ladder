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

use crate::core_output::CoreOutput;
use crate::runtime::Core;

pub(crate) struct ManagedChild {
    job: OwnedHandle,
    process: OwnedHandle,
    pub(crate) output: CoreOutput,
}

fn checked(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn create_job() -> io::Result<OwnedHandle> {
    // 匿名且不可继承，最后一个句柄始终由 Ladder 持有。
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

// Windows 的环境块按变量名（忽略大小写）排序，以两个 NUL 结束。
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
        config: &Path,
    ) -> io::Result<Self> {
        let job = create_job()?;
        let stdin = File::open("NUL")?;
        let (stdout, stdout_write) = io::pipe()?;
        let (stderr, stderr_write) = io::pipe()?;
        let stdio = [
            stdin.as_raw_handle(),
            stdout_write.as_raw_handle(),
            stderr_write.as_raw_handle(),
        ];
        let jobs = [job.as_raw_handle()];

        // 三个路径都以实际文件或目录名结尾。Windows 文件名不能包含双引号。
        let mut command = OsString::from("\"");
        command.push(exe);
        command.push("\" run");
        if core == Core::SingBox {
            command.push(" -D \"");
            command.push(assets);
            command.push("\"");
        }
        command.push(" -c \"");
        command.push(config);
        command.push("\"");
        let wide = |value: &OsStr| value.encode_wide().chain(Some(0)).collect::<Vec<_>>();
        let (exe, root, mut command) = (
            wide(exe.as_os_str()),
            wide(root.as_os_str()),
            wide(&command),
        );
        let env = (core == Core::Xray).then(|| xray_environment(assets));

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
        // 系统在新进程运行前完成 Job 归属。启动器在创建期间被强杀也不会遗留暂停的核心。
        checked(unsafe {
            CreateProcessW(
                exe.as_ptr(),
                command.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                env.as_ref()
                    .map_or(ptr::null(), |block| block.as_ptr().cast()),
                root.as_ptr(),
                &startup.StartupInfo,
                &mut info,
            )
        })?;
        let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
        drop(unsafe { OwnedHandle::from_raw_handle(info.hThread) });
        drop((stdin, stdout_write, stderr_write));
        Ok(Self {
            job,
            process,
            output: CoreOutput::capture(stdout, stderr),
        })
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
            if counts.ActiveProcesses == 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "等待托管进程退出超时，请重试停止",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
