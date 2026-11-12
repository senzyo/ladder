use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;

use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::UI::Shell::{SE_ERR_ASSOCINCOMPLETE, SE_ERR_NOASSOC, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetForegroundWindow, SW_SHOWNORMAL, WM_NCACTIVATE,
};
use windows_sys::w;

pub(crate) fn set_titlebar_dark(window: &Window, dark: bool) {
    if let Ok(handle) = HasWindowHandle::window_handle(window)
        && let RawWindowHandle::Win32(raw) = handle.as_raw()
    {
        let hwnd = raw.hwnd.get() as HWND;
        let enabled = i32::from(dark);
        // HWND 由当前窗口持有，DWM 在这次调用中读取 BOOL。
        unsafe {
            let result = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                ptr::from_ref(&enabled).cast(),
                size_of_val(&enabled) as u32,
            );
            if result >= 0 {
                // Windows 10 需要切换非客户区的激活外观才会刷新标题栏颜色。
                let active = GetForegroundWindow() == hwnd;
                DefWindowProcW(hwnd, WM_NCACTIVATE, usize::from(!active), 0);
                DefWindowProcW(hwnd, WM_NCACTIVATE, usize::from(active), 0);
            }
        }
    }
}

pub(crate) fn open_folder(path: &Path) -> Result<(), String> {
    let metadata = path
        .metadata()
        .map_err(|error| format!("无法访问目录: {error}"))?;
    if !metadata.is_dir() {
        return Err("所选路径不是目录".into());
    }
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // 路径以 NUL 结尾, 缓冲区在同步调用期间有效。
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            w!("open"),
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result > 32 {
        Ok(())
    } else {
        Err(format!("无法打开目录 (Windows 错误码 {result})"))
    }
}

pub(crate) fn edit_config(path: &Path) -> Result<(), String> {
    let metadata = path
        .metadata()
        .map_err(|error| format!("无法访问配置文件: {error}"))?;
    if !metadata.is_file() {
        return Err("所选路径不是配置文件".into());
    }
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        return Err("只能编辑 JSON 配置文件".into());
    }
    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // 只请求编辑动词, 避免默认打开关联将配置交给解释器执行。
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            w!("edit"),
            wide_path.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result > 32 {
        return Ok(());
    }
    if result != SE_ERR_NOASSOC as isize && result != SE_ERR_ASSOCINCOMPLETE as isize {
        return Err(format!("无法打开配置编辑器 (Windows 错误码 {result})"));
    }

    let mut directory = [0u16; 260];
    // 系统只写入给定容量内, 返回的字符数不包含末尾 NUL。
    let length = unsafe { GetSystemDirectoryW(directory.as_mut_ptr(), directory.len() as u32) };
    if length == 0 {
        return Err(format!(
            "无法定位系统记事本: {}",
            std::io::Error::last_os_error()
        ));
    }
    if length as usize >= directory.len() {
        return Err("无法定位系统记事本: 系统目录路径过长".into());
    }
    let notepad =
        PathBuf::from(OsString::from_wide(&directory[..length as usize])).join("notepad.exe");
    Command::new(notepad)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法启动系统记事本: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_paths_fail_before_launching_shell() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("不存在的 配置.json");
        assert!(open_folder(&missing).unwrap_err().contains("无法访问目录"));
        assert!(
            edit_config(&missing)
                .unwrap_err()
                .contains("无法访问配置文件")
        );
    }

    #[test]
    fn rejects_wrong_path_types_without_launching_shell() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("配置.txt");
        std::fs::write(&file, "{}").unwrap();
        assert_eq!(open_folder(&file).unwrap_err(), "所选路径不是目录");
        assert_eq!(
            edit_config(directory.path()).unwrap_err(),
            "所选路径不是配置文件"
        );
        assert_eq!(edit_config(&file).unwrap_err(), "只能编辑 JSON 配置文件");
    }
}
