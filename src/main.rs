//! 程序入口与核心逻辑。
//!
//! 负责应用生命周期管理（启动、运行、退出）、sing-box / xray 子进程的
//! 启停控制、系统托盘菜单分发、TUN 接口名随机化、DNS 缓存清理，
//! 以及孤立 WinTUN 设备节点的清理。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod error;
mod settings;
mod toast;
mod tray;
mod update;

use serde_json::Value;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::ptr::null;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;
use tracing::{info, warn, Event};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::prelude::*;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Console::{
    GetStdHandle, SetConsoleMode, GetConsoleMode, CONSOLE_MODE,
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_ERROR_HANDLE,
};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, OpenProcess, TerminateProcess, PROCESS_TERMINATE,
};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_Status, CM_Get_Device_ID_ListW, CM_Get_Device_ID_List_SizeW,
    CM_Locate_DevNodeW, CR_SUCCESS, DN_HAS_PROBLEM, DN_STARTED,
};
use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;

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

/// 全局应用状态。
struct AppState {
    /// 可执行文件所在目录，所有相对路径以此为基准。
    exe_dir: PathBuf,
    /// 菜单项 ID → 配置文件路径的映射，用于配置切换。
    config_actions: HashMap<u16, ConfigAction>,
    sing_box_version: Option<String>,
    xray_version: Option<String>,
    /// GDI 位图句柄：绿色（运行中）、黄色（未运行）、红色（未安装）。
    icon_green: isize,
    icon_yellow: isize,
    icon_red: isize,
    settings: settings::Settings,
}

/// 全局应用状态，通过 OnceLock + Mutex 实现线程安全的单例。
static APP: OnceLock<Mutex<AppState>> = OnceLock::new();

fn main() {
    if let Err(err) = run() {
        tray::show_error(0, "启动失败", &err);
    }
}

/// 自定义日志格式：`{时间戳}{毫秒}Z [{级别}] {消息}`，级别带 ANSI 颜色。
struct BracketedLevel;

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_BLUE: &str = "\x1b[34m";

fn level_color(level: &tracing::Level) -> &'static str {
    match *level {
        tracing::Level::INFO => ANSI_GREEN,
        tracing::Level::WARN => ANSI_YELLOW,
        tracing::Level::ERROR => ANSI_RED,
        tracing::Level::DEBUG => ANSI_BLUE,
        _ => "",
    }
}

impl<S, N> FormatEvent<S, N> for BracketedLevel
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let now = OffsetDateTime::now_utc();
        let level = event.metadata().level();
        let color = level_color(level);
        let reset = if writer.has_ansi_escapes() { ANSI_RESET } else { "" };
        let lc = if writer.has_ansi_escapes() { color } else { "" };
        write!(
            &mut writer,
            "{}{:03}Z [{lc}{level}{reset}] ",
            now.format(time::macros::format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second]"
            ))
            .map_err(|_| std::fmt::Error)?,
            now.millisecond(),
        )?;
        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// 初始化日志系统。
///
/// 1. 为 Windows 控制台启用 ANSI 转义码支持（彩色输出）
/// 2. 创建 console_layer（stderr）和 file_layer（app.log）
/// 3. 日志级别优先使用 RUST_LOG 环境变量，否则使用 settings.toml 中的配置
fn init_logging(exe_dir: &Path, log_level: &str) -> Result<(), String> {
    // 启用 Windows Terminal 的 ANSI 转义码处理，否则颜色代码会显示为乱码
    unsafe {
        if let Ok(handle) = GetStdHandle(STD_ERROR_HANDLE) {
            let mut mode = CONSOLE_MODE::default();
            if GetConsoleMode(handle, &mut mode).is_ok() {
                let _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
    }

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level));

    let console_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .compact()
        .event_format(BracketedLevel)
        .with_writer(std::io::stderr);

    let log_path = exe_dir.join("app.log");
    let file = std::fs::File::create(log_path)
        .map_err(|e| format!("创建日志文件失败: {e}"))?;
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(false)
        .compact()
        .event_format(BracketedLevel)
        .with_writer(file);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(console_layer)
        .with(filter)
        .try_init()
        .map_err(|e| format!("初始化日志系统失败: {e}"))?;

    Ok(())
}

/// 应用主入口：初始化 COM → 创建目录 → 加载配置 → 初始化日志和 Toast →
/// 检测版本 → 创建托盘图标 → 运行消息循环 → 退出时清理 GDI 资源。
fn run() -> Result<(), String> {
    // 初始化 COM（单线程单元模式），Toast 通知和 ShellLink 都依赖 COM
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| format!("初始化 COM 失败: {e}"))?;
    }

    let exe_path = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = exe_path.parent().ok_or("无法获取exe目录")?.to_path_buf();

    fs::create_dir_all(exe_dir.join("core")).map_err(|e| e.to_string())?;
    fs::create_dir_all(exe_dir.join("configs").join("sing-box")).map_err(|e| e.to_string())?;
    fs::create_dir_all(exe_dir.join("configs").join("xray")).map_err(|e| e.to_string())?;

    let app_settings = settings::Settings::load(&exe_dir);

    init_logging(&exe_dir, &app_settings.log.level)?;
    info!("程序启动, exe目录: {}", exe_dir.display());
    toast::setup(&exe_path).map_err(|e| format!("初始化 Toast 通知失败: {e}"))?;

    APP.set(Mutex::new(AppState {
        exe_dir: exe_dir.clone(),
        config_actions: HashMap::new(),
        sing_box_version: None,
        xray_version: None,
        icon_green: 0,
        icon_yellow: 0,
        icon_red: 0,
        settings: app_settings,
    }))
    .map_err(|_| "初始化状态失败".to_string())?;

    {
        let mut app = app_state_mut().ok_or("应用状态不可用")?;
        unsafe {
            app.icon_green = tray::load_icon_bitmap(&exe_dir, "green_circle.ico");
            app.icon_yellow = tray::load_icon_bitmap(&exe_dir, "yellow_circle.ico");
            app.icon_red = tray::load_icon_bitmap(&exe_dir, "red_circle.ico");
        }
    }

    let tooltip = {
        let mut app = app_state_mut().ok_or("应用状态不可用")?;
        let (sing_ver, xray_ver) = detect_versions(&app.exe_dir);
        app.sing_box_version = sing_ver;
        app.xray_version = xray_ver;
        format_tooltip(app.sing_box_version.as_deref(), app.xray_version.as_deref())
    };

    unsafe {
        let h_instance = GetModuleHandleW(None)
            .map_err(|e| format!("获取模块句柄失败: {e}"))?
            .0 as isize;
        let hwnd = tray::create_window(h_instance)?;
        tray::add_icon(hwnd, h_instance, &exe_dir)?;
        tray::set_tooltip(&tooltip);
        tray::run_message_loop();
    }

    if let Some(app) = app_state() {
        unsafe {
            use windows::Win32::Graphics::Gdi::DeleteObject;
            use windows::Win32::Graphics::Gdi::HGDIOBJ;
            if app.icon_green != 0 { let _ = DeleteObject(HGDIOBJ(app.icon_green as *mut std::ffi::c_void)); }
            if app.icon_yellow != 0 { let _ = DeleteObject(HGDIOBJ(app.icon_yellow as *mut std::ffi::c_void)); }
            if app.icon_red != 0 { let _ = DeleteObject(HGDIOBJ(app.icon_red as *mut std::ffi::c_void)); }
        }
    }

    Ok(())
}

/// 分发托盘菜单命令。
///
/// 重启/终止/更新操作在独立线程中执行（避免阻塞 UI 线程），每个线程
/// 独立初始化 COM。切换配置操作在当前线程同步执行。退出操作终止所有
/// 进程并销毁窗口。
fn execute_menu_command(hwnd: isize, id: u16) {
    let result = match id {
        tray::ID_RESTART_SING | tray::ID_RESTART_XRAY | tray::ID_RESTART_ALL |
        tray::ID_STOP_SING | tray::ID_STOP_XRAY | tray::ID_STOP_ALL => {
            let exe_dir = match exe_dir() {
                Ok(d) => d,
                Err(e) => {
                    tray::show_error(hwnd, "操作失败", &e);
                    return;
                }
            };
            // 子线程需要独立初始化 COM，否则 Toast 通知会失败
            std::thread::spawn(move || {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                }
                let label = match id {
                    tray::ID_RESTART_SING => "重启 sing-box",
                    tray::ID_RESTART_XRAY => "重启 xray",
                    tray::ID_RESTART_ALL => "重启所有服务",
                    tray::ID_STOP_SING => "终止 sing-box",
                    tray::ID_STOP_XRAY => "终止 xray",
                    tray::ID_STOP_ALL => "终止所有服务",
                    _ => "",
                };
                info!("{label}");
                let result = match id {
                    tray::ID_RESTART_SING => restart_sing_box_at(&exe_dir),
                    tray::ID_RESTART_XRAY => restart_xray_at(&exe_dir),
                    tray::ID_RESTART_ALL => restart_all_at(&exe_dir),
                    tray::ID_STOP_SING => stop_processes(&["sing-box.exe"]),
                    tray::ID_STOP_XRAY => stop_processes(&["xray.exe"]),
                    tray::ID_STOP_ALL => stop_all(),
                    _ => unreachable!(),
                };
                if let Err(err) = result {
                    toast::show_toast("操作失败", &err);
                }
                unsafe {
                    CoUninitialize();
                }
            });
            return;
        }
        tray::ID_UPDATE_ALL | tray::ID_UPDATE_SING | tray::ID_UPDATE_XRAY => {
            let exe_dir = match exe_dir() {
                Ok(d) => d,
                Err(e) => {
                    tray::show_error(hwnd, "操作失败", &e);
                    return;
                }
            };
            let (sing_ver, xray_ver) = {
                let app = app_state().expect("应用状态不可用");
                (app.sing_box_version.clone(), app.xray_version.clone())
            };
            let (gh_enabled, gh_url, max_retries, retry_delay) = {
                let app = app_state().expect("应用状态不可用");
                let s = &app.settings;
                (s.gh_proxy.enabled, s.gh_proxy.url.clone(), s.download.max_retries, s.download.retry_delay_secs)
            };
            let _ = stop_all();
            std::thread::spawn(move || {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                }
                let label = match id {
                    tray::ID_UPDATE_ALL => "更新所有核心",
                    tray::ID_UPDATE_SING => "更新 sing-box",
                    tray::ID_UPDATE_XRAY => "更新 xray",
                    _ => "",
                };
                info!("{label}");
                let result = match id {
                    tray::ID_UPDATE_ALL => update::update_cores(&exe_dir, sing_ver.as_deref(), xray_ver.as_deref(), gh_enabled, &gh_url, max_retries, retry_delay),
                    tray::ID_UPDATE_SING => update::update_sing_box(&exe_dir, sing_ver.as_deref(), gh_enabled, &gh_url, max_retries, retry_delay),
                    tray::ID_UPDATE_XRAY => update::update_xray(&exe_dir, xray_ver.as_deref(), gh_enabled, &gh_url, max_retries, retry_delay),
                    _ => unreachable!(),
                };
                refresh_versions_and_tooltip(&exe_dir);
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
            info!("退出程序");
            let _ = stop_all();
            unsafe { let _ = DestroyWindow(HWND(hwnd as *mut std::ffi::c_void)); }
            Ok(())
        }
        _ => run_config_action(id),
    };

    if let Err(err) = result {
        tray::show_error(hwnd, "操作失败", &err);
    }
}

fn restart_all_at(exe_dir: &Path) -> Result<(), String> {
    stop_all()?;
    start_sing_box_at(exe_dir)?;
    start_xray_at(exe_dir)
}

fn restart_sing_box_at(exe_dir: &Path) -> Result<(), String> {
    stop_processes(&["sing-box.exe"])?;
    start_sing_box_at(exe_dir)
}

fn restart_xray_at(exe_dir: &Path) -> Result<(), String> {
    stop_processes(&["xray.exe"])?;
    start_xray_at(exe_dir)
}

/// 终止所有已知子进程并刷新 DNS。
fn stop_all() -> Result<(), String> {
    stop_processes(&["sing-box.exe", "xray.exe"])
}

fn stop_processes(processes: &[&str]) -> Result<(), String> {
    for process in processes {
        info!("终止进程: {process}");
        kill_processes_by_name(process);
    }
    flush_dns();
    Ok(())
}

/// 通过 Win32 ToolHelp API 枚举所有进程，终止与 `exe_name` 匹配的进程。
fn kill_processes_by_name(exe_name: &str) {
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return;
        };

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let end = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                let name_bytes = &entry.szExeFile[..end];
                let name = String::from_utf16_lossy(name_bytes);
                if name.eq_ignore_ascii_case(exe_name) {
                    if let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, entry.th32ProcessID) {
                        let _ = TerminateProcess(handle, 1);
                        let _ = CloseHandle(handle);
                    }
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
    }
}

/// 启动 sing-box 子进程。启动前清理孤立 WinTUN 设备并随机化 TUN 接口名。
fn start_sing_box_at(exe_dir: &Path) -> Result<(), String> {
    cleanup_orphaned_wintun();

    let exe = exe_dir.join("core").join("sing-box.exe");
    let config = exe_dir.join("configs").join("sing-box.json");

    ensure_exists(&exe)?;
    ensure_exists(&config)?;
    randomize_tun_name(&config)?;

    info!("启动 sing-box");
    let mut child = hidden_command(exe)
        .args(["run", "-D"])
        .arg(exe_dir)
        .arg("-c")
        .arg(config)
        .current_dir(exe_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 sing-box 失败: {e}"))?;
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                warn!("[sing-box] {line}");
            }
        });
    }

    Ok(())
}

/// 启动 xray 子进程，stderr 输出重定向到日志。
fn start_xray_at(exe_dir: &Path) -> Result<(), String> {
    let exe = exe_dir.join("core").join("xray.exe");
    let config = exe_dir.join("configs").join("xray.json");

    ensure_exists(&exe)?;
    ensure_exists(&config)?;

    info!("启动 xray");
    let mut child = hidden_command(exe)
        .args(["run", "-c"])
        .arg(config)
        .current_dir(exe_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 xray 失败: {e}"))?;
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                warn!("[xray] {line}");
            }
        });
    }

    Ok(())
}

/// 执行配置切换操作：将选中的配置文件复制到活跃配置路径并重启对应服务。
fn run_config_action(id: u16) -> Result<(), String> {
    let action = {
        let app = app_state().ok_or("应用状态不可用")?;
        app.config_actions.get(&id).cloned()
    };

    let Some(action) = action else {
        return Ok(());
    };

    let exe_dir = exe_dir()?;
    info!("切换配置: {}", action.path.display());
    match action.kind {
        ConfigKind::SingBox => {
            fs::copy(&action.path, exe_dir.join("configs").join("sing-box.json"))
                .map_err(|e| format!("切换 sing-box 配置失败: {e}"))?;
            restart_sing_box_at(&exe_dir)
        }
        ConfigKind::Xray => {
            fs::copy(&action.path, exe_dir.join("configs").join("xray.json"))
                .map_err(|e| format!("切换 xray 配置失败: {e}"))?;
            restart_xray_at(&exe_dir)
        }
    }
}

/// 随机化 sing-box 配置中的 TUN 接口名。
///
/// sing-box TUN 适配器在 Windows 上以固定名称注册，重启时如果旧适配器
/// 未完全释放会导致冲突。通过每次启动时生成随机 6 位十六进制名称来避免。
///
/// 使用字符串替换而非 JSON 序列化来保留原始配置的格式和注释。
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

    let old_pattern = format!("\"interface_name\": \"{}\"", old_name);
    let new_pattern = format!("\"interface_name\": \"{}\"", new_name);
    let new_text = text.replacen(&old_pattern, &new_pattern, 1);
    fs::write(config_path, new_text).map_err(|e| format!("写入 sing-box 配置失败: {e}"))
}

/// 生成 6 位随机十六进制字符串，用作 TUN 接口名。
/// 取纳秒时间戳的低 24 位（6 个十六进制字符），同一纳秒内重复调用可能冲突。
fn random_hex_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{:06x}", nanos & 0xFF_FFFF)
}

/// 创建不显示控制台窗口的子进程 Command。
fn run_hidden_program(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW.0);
    command
}

/// `run_hidden_program` 的别名，语义更清晰的调用入口。
fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    run_hidden_program(program)
}

// dnsapi.dll 导入，用于刷新系统 DNS 缓存。
#[link(name = "dnsapi")]
extern "system" {
    fn DnsFlushResolverCache() -> i32;
}

/// 刷新 DNS 缓存。代理进程终止后，残留的 DNS 缓存可能导致解析失败。
fn flush_dns() {
    unsafe { DnsFlushResolverCache(); }
}

#[derive(Clone, Copy, PartialEq)]
enum ProcessState {
    NotInstalled,
    NotRunning,
    Running,
}

fn sing_box_state(app: &AppState) -> ProcessState {
    if !app.exe_dir.join("core").join("sing-box.exe").exists() {
        return ProcessState::NotInstalled;
    }
    if is_process_running("sing-box.exe") {
        ProcessState::Running
    } else {
        ProcessState::NotRunning
    }
}

fn xray_state(app: &AppState) -> ProcessState {
    if !app.exe_dir.join("core").join("xray.exe").exists() {
        return ProcessState::NotInstalled;
    }
    if is_process_running("xray.exe") {
        ProcessState::Running
    } else {
        ProcessState::NotRunning
    }
}

/// 检查指定名称的进程是否正在运行。
fn is_process_running(exe_name: &str) -> bool {
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return false;
        };

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let end = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                let name_bytes = &entry.szExeFile[..end];
                let name = String::from_utf16_lossy(name_bytes);
                if name.eq_ignore_ascii_case(exe_name) {
                    let _ = CloseHandle(snapshot);
                    return true;
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
        false
    }
}

/// 清理孤立或异常的 WinTUN 设备节点。
///
/// sing-box TUN 模式依赖 WinTUN 驱动，异常退出后可能残留无效设备节点，
/// 导致下次启动时 TUN 接口创建失败。此函数：
///
/// 1. 通过 `CM_Get_Device_ID_ListW` 枚举所有设备实例 ID
/// 2. 解析双 null 结尾的多字符串缓冲区
/// 3. 筛选包含 "WINTUN" 的设备
/// 4. 检查设备状态：设备节点不存在（CR_NO_SUCH_DEVNODE）或未启动 / 有异常
/// 5. 通过 `pnputil /remove-device` 移除问题设备
/// 6. 最后执行 `pnputil /scan-devices` 重新扫描硬件
fn cleanup_orphaned_wintun() {
    const CR_NO_SUCH_DEVNODE: u32 = 0x0D; // 设备节点不存在

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
                    // 设备存在，检查是否未启动或有异常问题
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

/// 从多个目录中收集所有 .json 文件，按文件名排序去重。
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

fn exe_dir() -> Result<PathBuf, String> {
    app_state()
        .map(|app| app.exe_dir.clone())
        .ok_or_else(|| "应用状态不可用".to_string())
}

fn app_state() -> Option<std::sync::MutexGuard<'static, AppState>> {
    APP.get()?.lock().ok()
}

/// 获取可变应用状态。与 `app_state()` 相同（Mutex::lock 返回可变守卫），
/// 语义上区分只读和可变访问意图。
fn app_state_mut() -> Option<std::sync::MutexGuard<'static, AppState>> {
    app_state()
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_hex_name() {
        let name1 = random_hex_name();
        let name2 = random_hex_name();
        assert_eq!(name1.len(), 6);
        assert!(name1.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(name1, name2);
    }
}

/// 检测本地 sing-box 和 xray 的版本。
/// 版本号为 "0.0.0" 表示可执行文件存在但无法获取版本，视为未安装。
fn detect_versions(exe_dir: &Path) -> (Option<String>, Option<String>) {
    let sing_exe = exe_dir.join("core").join("sing-box.exe");
    let xray_exe = exe_dir.join("core").join("xray.exe");
    let sing_ver = if sing_exe.exists() {
        let v = update::get_local_version(&sing_exe, "version");
        if v != "0.0.0" { Some(v) } else { None }
    } else { None };
    let xray_ver = if xray_exe.exists() {
        let v = update::get_local_version(&xray_exe, "version");
        if v != "0.0.0" { Some(v) } else { None }
    } else { None };
    (sing_ver, xray_ver)
}

/// 格式化托盘提示文本，显示两个核心的版本状态。
fn format_tooltip(sing_ver: Option<&str>, xray_ver: Option<&str>) -> String {
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

/// 重新检测版本并更新托盘提示文本。
fn refresh_versions_and_tooltip(exe_dir: &Path) {
    if let Some(mut app) = app_state_mut() {
        let (sing_ver, xray_ver) = detect_versions(exe_dir);
        app.sing_box_version = sing_ver;
        app.xray_version = xray_ver;
        let tooltip = format_tooltip(app.sing_box_version.as_deref(), app.xray_version.as_deref());
        drop(app);
        tray::set_tooltip(&tooltip);
    }
}


