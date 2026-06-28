//! 共享类型定义、全局应用状态与通用工具函数。
//!
//! 各模块通过此文件访问全局单例 `AppState` 和公共工具函数，
//! 避免模块间循环依赖。

use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Mutex, OnceLock};
use tracing::warn;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

use crate::error::AppError;
use crate::settings;

#[derive(Clone, Copy)]
pub enum ConfigKind {
    SingBox,
    Xray,
}

#[derive(Clone)]
pub struct ConfigAction {
    pub kind: ConfigKind,
    pub path: PathBuf,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ProcessState {
    NotInstalled,
    NotRunning,
    Running,
}

/// 全局应用状态。
pub struct AppState {
    /// 可执行文件所在目录，所有相对路径以此为基准。
    pub exe_dir: PathBuf,
    pub sing_box_version: Option<String>,
    pub xray_version: Option<String>,
    /// GDI 位图句柄：绿色（运行中）、黄色（未运行）、红色（未安装）。
    pub icon_green: isize,
    pub icon_yellow: isize,
    pub icon_red: isize,
    pub settings: settings::Settings,
    /// 子进程句柄，用于直接 kill。
    pub child_sing_box: Option<Child>,
    pub child_xray: Option<Child>,
}

/// 全局应用状态，通过 OnceLock + Mutex 实现线程安全的单例。
pub static APP: OnceLock<Mutex<AppState>> = OnceLock::new();

/// 获取只读应用状态。
pub fn app_state() -> Option<std::sync::MutexGuard<'static, AppState>> {
    APP.get()?.lock().ok()
}

/// 获取可变应用状态。语义与 `app_state()` 相同，
/// Mutex::lock 返回 `MutexGuard` 总是可变的。
pub fn app_state_mut() -> Option<std::sync::MutexGuard<'static, AppState>> {
    app_state()
}

/// 获取 exe 所在目录。
pub fn exe_dir() -> Result<PathBuf, AppError> {
    app_state()
        .map(|app| app.exe_dir.clone())
        .ok_or_else(|| AppError::Msg("应用状态不可用".into()))
}

/// 将字符串转为 null 结尾的 UTF-16 Vec。
pub fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

/// 检查路径存在性，不存在则返回错误。
pub fn ensure_exists(path: &Path) -> Result<(), AppError> {
    if path.exists() {
        Ok(())
    } else {
        Err(AppError::Msg(format!("文件不存在: {}", path.display())))
    }
}

/// 从多个目录中收集所有 .json 文件，按文件名排序去重。
pub fn find_json_configs(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                paths.push(path);
            }
        }
    }

    paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    paths.dedup();
    paths
}

/// 通过 Win32 ToolHelp API 枚举所有与 `exe_name` 匹配的进程，返回 PID 列表。
pub fn find_pids_by_name(exe_name: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            warn!("CreateToolhelp32Snapshot 失败，无法枚举进程");
            return pids;
        };

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let end = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name_bytes = &entry.szExeFile[..end];
                let name = String::from_utf16_lossy(name_bytes);
                if name.eq_ignore_ascii_case(exe_name) {
                    pids.push(entry.th32ProcessID);
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
    }
    pids
}

/// 检查指定名称的进程是否正在运行。
pub fn is_process_running(exe_name: &str) -> bool {
    !find_pids_by_name(exe_name).is_empty()
}

/// 查询 sing-box 进程运行状态。
pub fn sing_box_state(app: &AppState) -> ProcessState {
    if !app.exe_dir.join("core").join("sing-box.exe").exists() {
        return ProcessState::NotInstalled;
    }
    if is_process_running("sing-box.exe") {
        ProcessState::Running
    } else {
        ProcessState::NotRunning
    }
}

/// 查询 xray 进程运行状态。
pub fn xray_state(app: &AppState) -> ProcessState {
    if !app.exe_dir.join("core").join("xray.exe").exists() {
        return ProcessState::NotInstalled;
    }
    if is_process_running("xray.exe") {
        ProcessState::Running
    } else {
        ProcessState::NotRunning
    }
}

/// 检测本地 sing-box 和 xray 的版本。
/// 版本号为 "0.0.0" 表示可执行文件存在但无法获取版本，视为未安装。
pub fn detect_versions(exe_dir: &Path) -> (Option<String>, Option<String>) {
    let sing_exe = exe_dir.join("core").join("sing-box.exe");
    let xray_exe = exe_dir.join("core").join("xray.exe");
    let sing_ver = if sing_exe.exists() {
        let v = crate::update::get_local_version(&sing_exe, "version");
        if v != "0.0.0" { Some(v) } else { None }
    } else {
        None
    };
    let xray_ver = if xray_exe.exists() {
        let v = crate::update::get_local_version(&xray_exe, "version");
        if v != "0.0.0" { Some(v) } else { None }
    } else {
        None
    };
    (sing_ver, xray_ver)
}

/// 格式化托盘提示文本，显示两个核心的版本状态。
pub fn format_tooltip(sing_ver: Option<&str>, xray_ver: Option<&str>) -> String {
    let sing = match sing_ver {
        Some(v) => format!("sing-box v{}", v),
        None => "sing-box 未安装".to_string(),
    };
    let xray = match xray_ver {
        Some(v) => format!("xray v{}", v),
        None => "xray 未安装".to_string(),
    };
    format!("{}\n{}", sing, xray)
}
