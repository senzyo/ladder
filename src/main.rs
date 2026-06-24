#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod toast;
mod tray;
mod update;

use serde_json::Value;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_Status, CM_Get_Device_ID_ListW, CM_Get_Device_ID_List_SizeW,
    CM_Locate_DevNodeW, CR_SUCCESS, DN_HAS_PROBLEM, DN_STARTED,
};
use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;

#[derive(Clone)]
enum ConfigKind {
    SingBox,
    Xray,
}

#[derive(Clone)]
struct ConfigAction {
    kind: ConfigKind,
    path: PathBuf,
}

struct AppState {
    work_dir: PathBuf,
    config_actions: HashMap<u16, ConfigAction>,
}

static APP: OnceLock<Mutex<AppState>> = OnceLock::new();

fn main() {
    if let Err(err) = run() {
        tray::show_error(null_mut(), "启动失败", &err);
    }
}

fn run() -> Result<(), String> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| format!("初始化 COM 失败: {e}"))?;
    }

    let work_dir = detect_work_dir();
    let exe_path = std::env::current_exe()
        .unwrap_or_else(|_| work_dir.join("sing-box_with_xray.exe"));
    toast::setup(&exe_path).map_err(|e| format!("初始化 Toast 通知失败: {e}"))?;

    fs::create_dir_all(work_dir.join("configs").join("sing-box")).map_err(|e| e.to_string())?;
    fs::create_dir_all(work_dir.join("configs").join("xray")).map_err(|e| e.to_string())?;

    APP.set(Mutex::new(AppState {
        work_dir: work_dir.clone(),
        config_actions: HashMap::new(),
    }))
    .map_err(|_| "初始化状态失败".to_string())?;

    unsafe {
        let h_instance = GetModuleHandleW(null());
        let hwnd = tray::create_window(h_instance)?;
        tray::add_icon(hwnd, h_instance, &work_dir)?;
        tray::run_message_loop();
    }

    Ok(())
}

fn execute_menu_command(hwnd: HWND, id: u16) {
    let result = match id {
        tray::ID_RESTART_SING => restart_sing_box(),
        tray::ID_RESTART_XRAY => restart_xray(),
        tray::ID_RESTART_ALL => restart_all(),
        tray::ID_STOP_SING => stop_processes(&["sing-box.exe"]),
        tray::ID_STOP_XRAY => stop_processes(&["xray.exe"]),
        tray::ID_STOP_ALL => stop_all(),
        tray::ID_UPDATE_ALL | tray::ID_UPDATE_SING | tray::ID_UPDATE_XRAY => {
            let work_dir = match work_dir() {
                Ok(d) => d,
                Err(e) => {
                    tray::show_error(hwnd, "操作失败", &e);
                    return;
                }
            };
            let _ = stop_all();
            std::thread::spawn(move || {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                }
                let result = match id {
                    tray::ID_UPDATE_ALL => update::update_cores(&work_dir),
                    tray::ID_UPDATE_SING => update::update_sing_box(&work_dir),
                    tray::ID_UPDATE_XRAY => update::update_xray(&work_dir),
                    _ => unreachable!(),
                };
                tray::set_tooltip("sing-box-with-xray");
                if let Err(e) = result {
                    toast::show_toast("更新失败", &e);
                }
                unsafe {
                    CoUninitialize();
                }
            });
            return;
        }
        tray::ID_EXIT => {
            unsafe { DestroyWindow(hwnd) };
            Ok(())
        }
        _ => run_config_action(id),
    };

    if let Err(err) = result {
        tray::show_error(hwnd, "操作失败", &err);
    }
}

fn restart_all() -> Result<(), String> {
    stop_all()?;
    start_sing_box()?;
    start_xray()
}

fn restart_sing_box() -> Result<(), String> {
    stop_processes(&["sing-box.exe"])?;
    start_sing_box()
}

fn restart_xray() -> Result<(), String> {
    stop_processes(&["xray.exe"])?;
    start_xray()
}

fn stop_all() -> Result<(), String> {
    stop_processes(&["sing-box.exe", "xray.exe"])
}

fn stop_processes(processes: &[&str]) -> Result<(), String> {
    for process in processes {
        let _ = hidden_command("taskkill")
            .args(["/F", "/IM", process])
            .status();
    }
    flush_dns();
    Ok(())
}

fn start_sing_box() -> Result<(), String> {
    cleanup_orphaned_wintun();

    let work_dir = work_dir()?;
    let exe = work_dir.join("sing-box.exe");
    let config = work_dir.join("sing-box.json");

    ensure_exists(&exe)?;
    ensure_exists(&config)?;
    randomize_tun_name(&config)?;

    hidden_command(exe)
        .args(["run", "-D"])
        .arg(&work_dir)
        .arg("-c")
        .arg(config)
        .current_dir(work_dir)
        .spawn()
        .map_err(|e| format!("启动 sing-box 失败: {e}"))?;

    Ok(())
}

fn start_xray() -> Result<(), String> {
    let work_dir = work_dir()?;
    let exe = work_dir.join("xray.exe");
    let config = work_dir.join("xray.json");

    ensure_exists(&exe)?;
    ensure_exists(&config)?;

    hidden_command(exe)
        .args(["run", "-c"])
        .arg(config)
        .current_dir(work_dir)
        .spawn()
        .map_err(|e| format!("启动 xray 失败: {e}"))?;

    Ok(())
}

fn run_config_action(id: u16) -> Result<(), String> {
    let action = {
        let app = app_state().ok_or("应用状态不可用")?;
        app.config_actions.get(&id).cloned()
    };

    let Some(action) = action else {
        return Ok(());
    };

    let work_dir = work_dir()?;
    match action.kind {
        ConfigKind::SingBox => {
            fs::copy(&action.path, work_dir.join("sing-box.json"))
                .map_err(|e| format!("切换 sing-box 配置失败: {e}"))?;
            restart_sing_box()
        }
        ConfigKind::Xray => {
            fs::copy(&action.path, work_dir.join("xray.json"))
                .map_err(|e| format!("切换 xray 配置失败: {e}"))?;
            restart_xray()
        }
    }
}

fn randomize_tun_name(config_path: &Path) -> Result<(), String> {
    let text =
        fs::read_to_string(config_path).map_err(|e| format!("读取 sing-box 配置失败: {e}"))?;
    let json: Value =
        serde_json::from_str(&text).map_err(|e| format!("解析 sing-box 配置失败: {e}"))?;
    let new_name = random_hex_name();

    let old_name = json
        .get("inbounds")
        .and_then(Value::as_array)
        .and_then(|inbounds| {
            inbounds.iter().find_map(|inbound| {
                if inbound.get("type").and_then(Value::as_str) == Some("tun") {
                    inbound.get("interface_name").and_then(Value::as_str)
                } else {
                    None
                }
            })
        })
        .ok_or("未在 sing-box.json 中找到 type=tun 的 inbound".to_string())?;

    if old_name == new_name {
        return Ok(());
    }

    let new_text = text.replacen(old_name, &new_name, 1);
    fs::write(config_path, new_text).map_err(|e| format!("写入 sing-box 配置失败: {e}"))
}

fn random_hex_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{:06x}", nanos & 0xFF_FFFF)
}

fn run_hidden_program(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    run_hidden_program(program)
}

fn flush_dns() {
    let _ = hidden_command("ipconfig").arg("/flushdns").status();
}

#[derive(Clone, Copy, PartialEq)]
enum ProcessState {
    NotInstalled,
    NotRunning,
    Running,
}

fn sing_box_state(app: &mut AppState) -> ProcessState {
    if !app.work_dir.join("sing-box.exe").exists() {
        return ProcessState::NotInstalled;
    }
    if is_process_running("sing-box.exe") {
        ProcessState::Running
    } else {
        ProcessState::NotRunning
    }
}

fn xray_state(app: &mut AppState) -> ProcessState {
    if !app.work_dir.join("xray.exe").exists() {
        return ProcessState::NotInstalled;
    }
    if is_process_running("xray.exe") {
        ProcessState::Running
    } else {
        ProcessState::NotRunning
    }
}

fn is_process_running(exe_name: &str) -> bool {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let end = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                let name_bytes = &entry.szExeFile[..end];
                let name = String::from_utf16_lossy(name_bytes);
                if name.eq_ignore_ascii_case(exe_name) {
                    CloseHandle(snapshot);
                    return true;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }

        CloseHandle(snapshot);
        false
    }
}

fn cleanup_orphaned_wintun() {
    const CR_NO_SUCH_DEVNODE: u32 = 0x0D;

    let mut instance_ids: Vec<String> = Vec::new();

    unsafe {
        let mut size = 0u32;
        if CM_Get_Device_ID_List_SizeW(&mut size, null(), 0) != CR_SUCCESS {
            return;
        }
        if size == 0 {
            return;
        }

        let mut buffer: Vec<u16> = vec![0u16; size as usize];
        if CM_Get_Device_ID_ListW(null(), buffer.as_mut_ptr(), size, 0) != CR_SUCCESS {
            return;
        }

        let mut start = 0usize;
        while start < buffer.len() {
            let end = buffer[start..]
                .iter()
                .position(|&c| c == 0)
                .map(|p| start + p)
                .unwrap_or(buffer.len());
            if end == start {
                break;
            }
            let id = String::from_utf16_lossy(&buffer[start..end]);

            if id.to_uppercase().contains("WINTUN") {
                let mut dev_inst = 0u32;
                let wide_id = wide(&id);
                let locate_ret = CM_Locate_DevNodeW(&mut dev_inst, wide_id.as_ptr(), 0);

                if locate_ret == CR_NO_SUCH_DEVNODE {
                    instance_ids.push(id);
                } else if locate_ret == CR_SUCCESS {
                    let mut status = 0u32;
                    let mut problem = 0u32;
                    if CM_Get_DevNode_Status(&mut status, &mut problem, dev_inst, 0) == CR_SUCCESS
                        && ((status & DN_STARTED) == 0 || (status & DN_HAS_PROBLEM) != 0)
                    {
                        instance_ids.push(id);
                    }
                }
            }

            start = end + 1;
        }
    }

    for id in &instance_ids {
        let _ = hidden_command("pnputil")
            .args(["/remove-device", id.as_str()])
            .status();
    }

    if !instance_ids.is_empty() {
        let _ = hidden_command("pnputil").arg("/scan-devices").status();
    }
}

fn ensure_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        Ok(())
    } else {
        Err(format!("文件不存在: {}", path.display()))
    }
}

fn find_json_configs(dirs: &[PathBuf]) -> Vec<PathBuf> {
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

    paths.sort_by_key(|path| path.file_name().map(|name| name.to_os_string()));
    paths.dedup();
    paths
}

fn detect_work_dir() -> PathBuf {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some(dir) = find_work_dir_from(&current_dir) {
        return dir;
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Some(dir) = find_work_dir_from(dir) {
                return dir;
            }
        }
    }

    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        let app_dir = PathBuf::from(user_profile)
            .join("Apps")
            .join("sing-box-with-xray");
        if app_dir.exists() {
            return app_dir;
        }
    }

    current_dir
}

fn find_work_dir_from(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|dir| {
        if dir.join("sing-box.json").exists() || dir.join("Restart.ps1").exists() {
            Some(dir.to_path_buf())
        } else {
            None
        }
    })
}

fn work_dir() -> Result<PathBuf, String> {
    app_state()
        .map(|app| app.work_dir.clone())
        .ok_or_else(|| "应用状态不可用".to_string())
}

fn app_state() -> Option<std::sync::MutexGuard<'static, AppState>> {
    APP.get()?.lock().ok()
}

fn app_state_mut() -> Option<std::sync::MutexGuard<'static, AppState>> {
    app_state()
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn set_wstr_array<const N: usize>(target: &mut [u16; N], value: &str) {
    let wide = wide(value);
    let len = wide.len().saturating_sub(1).min(N - 1);
    target[..len].copy_from_slice(&wide[..len]);
    target[len] = 0;
}


