//! panic 路径直接启动恢复助手, 避开 std::process::Command 的全局创建锁。

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE_FLAG_INHERIT, SetHandleInformation};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    GetCurrentProcessId, InitializeProcThreadAttributeList, OpenProcess,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW,
    UpdateProcThreadAttribute,
};

pub(crate) struct PanicHelper {
    // 先销毁属性表, 再释放它引用的句柄数组和句柄。
    attributes: AttributeList,
    // 属性表引用这块稳定的堆内存；句柄值与 HANDLE 数组具有相同布局。
    _inherited: Box<[usize; 2]>,
    executable: Vec<u16>,
    command: Vec<u16>,
    parent: OwnedHandle,
    output: File,
}

struct AttributeList(Vec<usize>);

impl Drop for AttributeList {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast()) };
    }
}

fn checked(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

// 始终加引号, 按 Windows argv 规则转义引号前和末尾的反斜杠。
fn append_argument(command: &mut Vec<u16>, argument: &OsStr) -> io::Result<()> {
    command.push(b'"' as u16);
    let mut slashes = 0;
    for unit in argument.encode_wide() {
        match unit {
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "恢复助手参数包含 NUL",
                ));
            }
            value if value == b'\\' as u16 => slashes += 1,
            value => {
                let count = if value == b'"' as u16 {
                    slashes * 2 + 1
                } else {
                    slashes
                };
                command.extend(std::iter::repeat_n(b'\\' as u16, count));
                command.push(value);
                slashes = 0;
            }
        }
    }
    command.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    command.push(b'"' as u16);
    Ok(())
}

impl PanicHelper {
    pub(crate) fn prepare(exe: &Path, arguments: &[OsString]) -> io::Result<Self> {
        let executable: Vec<_> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut command = Vec::new();
        append_argument(&mut command, exe.as_os_str())?;
        for argument in arguments {
            command.push(b' ' as u16);
            append_argument(&mut command, argument)?;
        }
        command.push(0);
        if command.len() > 32767 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "恢复助手命令行过长",
            ));
        }

        // 真实进程句柄不会因 PID 复用而等待另一个进程, 也不会赋予助手终止父进程的权限。
        let raw = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                GetCurrentProcessId(),
            )
        };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let parent = unsafe { OwnedHandle::from_raw_handle(raw) };
        let output = File::options().write(true).open("NUL")?;
        for handle in [parent.as_raw_handle(), output.as_raw_handle()] {
            checked(unsafe {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT)
            })?;
        }
        let inherited = Box::new([
            parent.as_raw_handle() as usize,
            output.as_raw_handle() as usize,
        ]);
        let mut bytes = 0;
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut bytes) };
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        checked(unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 1, 0, &mut bytes)
        })?;
        let mut attributes = AttributeList(storage);
        checked(unsafe {
            UpdateProcThreadAttribute(
                attributes.0.as_mut_ptr().cast(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                inherited.as_ptr().cast(),
                size_of_val(inherited.as_ref()),
                ptr::null_mut(),
                ptr::null(),
            )
        })?;
        Ok(Self {
            executable,
            command,
            parent,
            output,
            _inherited: inherited,
            attributes,
        })
    }

    pub(crate) fn spawn(&mut self) -> io::Result<()> {
        let startup = STARTUPINFOEXW {
            StartupInfo: STARTUPINFOW {
                cb: size_of::<STARTUPINFOEXW>() as u32,
                dwFlags: STARTF_USESTDHANDLES,
                hStdInput: self.parent.as_raw_handle(),
                hStdOutput: self.output.as_raw_handle(),
                hStdError: self.output.as_raw_handle(),
                ..Default::default()
            },
            lpAttributeList: self.attributes.0.as_mut_ptr().cast(),
        };
        let mut info = PROCESS_INFORMATION::default();
        // 不加入核心的 Job: 助手必须在 Ladder 终止后继续运行。
        checked(unsafe {
            CreateProcessW(
                self.executable.as_ptr(),
                self.command.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
                ptr::null(),
                ptr::null(),
                &startup.StartupInfo,
                &mut info,
            )
        })?;
        unsafe {
            CloseHandle(info.hThread);
            CloseHandle(info.hProcess);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_unicode_spaces_quotes_and_trailing_slashes() {
        for (input, expected) in [
            ("", "\"\""),
            (r"C:\目录 with spaces\", r#""C:\目录 with spaces\\""#),
            (r#"a\"b"#, r#""a\\\"b""#),
            (r"C:\", r#""C:\\""#),
        ] {
            let mut encoded = Vec::new();
            append_argument(&mut encoded, OsStr::new(input)).unwrap();
            assert_eq!(String::from_utf16(&encoded).unwrap(), expected);
        }
        assert!(append_argument(&mut Vec::new(), OsStr::new("invalid\0argument")).is_err());
    }

    #[test]
    fn missing_helper_is_a_spawn_error() {
        let folder = tempfile::tempdir().unwrap();
        let mut helper = PanicHelper::prepare(&folder.path().join("missing.exe"), &[]).unwrap();
        assert!(helper.spawn().is_err());
    }
}
