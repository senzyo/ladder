//! 程序入口与核心编排。
//!
//! 负责应用生命周期管理（启动、运行、退出）、日志初始化、
//! 托盘菜单命令分发，以及状态刷新。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dns;
mod error;
mod process;
mod scheduler;
mod settings;
mod state;
mod toast;
mod tray;
mod update;

use error::AppError;

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use tracing::{Event, debug, error, info, warn};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::prelude::*;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Console::{
    CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_ERROR_HANDLE, SetConsoleMode,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;

use crate::state::ConfigAction;

/// COM 初始化守卫，Drop 时自动调用 CoUninitialize。
struct ComGuard;

impl ComGuard {
    fn new() -> Result<Self, AppError> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| AppError::Msg(format!("初始化 COM 失败: {e}")))?;
        }
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

fn main() {
    if let Err(err) = run() {
        tray::show_error(0, "启动失败", &err.to_string());
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
    fn format_event(&self, ctx: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> std::fmt::Result {
        let level = event.metadata().level();
        let color = level_color(level);
        let reset = if writer.has_ansi_escapes() { ANSI_RESET } else { "" };
        let lc = if writer.has_ansi_escapes() { color } else { "" };
        let ts = format_utc_timestamp();
        write!(&mut writer, "{ts} [{lc}{level}{reset}] ")?;
        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// 将当前 UTC 时间格式化为 `YYYY-MM-DDTHH:MM:SS.mmmZ`。
fn format_utc_timestamp() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let millis = dur.subsec_millis();

    // 秒 → UTC 时间组件（简化算法，适用于 1970-2100）
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hour = time_of_day / 3600;
    let minute = (time_of_day % 3600) / 60;
    let second = time_of_day % 60;

    // 天数 → 年/月/日
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// 将自 UNIX 纪元以来的天数转换为年、月、日。
fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // 先估算年份
    let mut year = 1970 + days / 365;
    // 微调：计算该年之前的总天数
    loop {
        let prev_days = days_before_year(year);
        if prev_days > days {
            year -= 1;
        } else {
            days -= prev_days;
            break;
        }
    }

    // 月份天数（非闰年）
    let month_days = [31u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let leap = is_leap(year);

    let mut month = 1u64;
    for (i, &md) in month_days.iter().enumerate() {
        let dim = if i == 1 && leap { 29 } else { md };
        if days < dim {
            month = i as u64 + 1;
            break;
        }
        days -= dim;
    }

    (year, month, days + 1)
}

/// 1970-01-01 到指定年份 01-01 的天数。
fn days_before_year(year: u64) -> u64 {
    let y = year - 1970;
    y * 365 + y / 4 - y / 100 + y / 400
}

fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// 初始化日志系统。
///
/// 1. 为 Windows 控制台启用 ANSI 转义码支持（彩色输出）
/// 2. 创建 console_layer（stderr）和 file_layer（app.log）
/// 3. 日志级别优先使用 RUST_LOG 环境变量，否则使用 settings.toml 中的配置
fn init_logging(exe_dir: &Path, log_level: &str) -> Result<(), AppError> {
    unsafe {
        if let Ok(handle) = GetStdHandle(STD_ERROR_HANDLE) {
            let mut mode = CONSOLE_MODE::default();
            if GetConsoleMode(handle, &mut mode).is_ok() {
                let _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
    }

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    let console_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .compact()
        .event_format(BracketedLevel)
        .with_writer(std::io::stderr);

    let log_path = exe_dir.join("app.log");
    let file = std::fs::File::create(log_path).map_err(|e| AppError::Msg(format!("创建日志文件失败: {e}")))?;
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
        .map_err(|e| AppError::Msg(format!("初始化日志系统失败: {e}")))?;

    Ok(())
}

/// 应用主入口：初始化 COM → 创建目录 → 加载配置 → 初始化日志和 Toast →
/// 检测版本 → 创建托盘图标 → 运行消息循环 → 退出时清理 GDI 资源。
fn run() -> Result<(), AppError> {
    let _com = ComGuard::new()?;

    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path
        .parent()
        .ok_or(AppError::Msg("无法获取exe目录".into()))?
        .to_path_buf();

    fs::create_dir_all(exe_dir.join("sing-box_core"))?;
    fs::create_dir_all(exe_dir.join("xray_core"))?;
    fs::create_dir_all(exe_dir.join("configs").join("sing-box"))?;
    fs::create_dir_all(exe_dir.join("configs").join("xray"))?;

    let app_settings = settings::Settings::load(&exe_dir);

    init_logging(&exe_dir, &app_settings.log.level)?;
    for w in settings::Settings::take_warnings() {
        warn!("{w}");
    }
    info!("程序启动, exe目录: {}", exe_dir.display());
    debug!(
        "生效配置: proxy={}({}), log_level={}, max_retries={}, retry_delay={}s",
        if app_settings.gh_proxy.enabled {
            "enabled"
        } else {
            "disabled"
        },
        app_settings.gh_proxy.url,
        app_settings.log.level,
        app_settings.download.max_retries,
        app_settings.download.retry_delay_secs,
    );
    process::cleanup_network_registry();
    dns::restore_dns_to_dhcp();
    scheduler::ensure_boot_dns_reset_task(&exe_dir);
    toast::setup(&exe_path).map_err(|e| AppError::Msg(format!("初始化 Toast 通知失败: {e}")))?;

    let icon_green = unsafe { tray::load_icon_bitmap(&exe_dir, "green_circle.ico") };
    let icon_yellow = unsafe { tray::load_icon_bitmap(&exe_dir, "yellow_circle.ico") };
    let icon_red = unsafe { tray::load_icon_bitmap(&exe_dir, "red_circle.ico") };

    state::APP
        .set(Mutex::new(state::AppState {
            exe_dir: exe_dir.clone(),
            icon_green,
            icon_yellow,
            icon_red,
            settings: app_settings,
            child_sing_box: None,
            child_xray: None,
        }))
        .map_err(|_| AppError::Msg("初始化状态失败".into()))?;

    unsafe {
        let h_instance = GetModuleHandleW(None)
            .map_err(|e| AppError::Msg(format!("获取模块句柄失败: {e}")))?
            .0 as isize;
        let hwnd = tray::create_window(h_instance)?;
        tray::add_icon(hwnd, h_instance, &exe_dir)?;
        tray::set_tooltip("sing-box with xray");
        tray::run_message_loop();
    }

    if let Some(app) = state::app_state() {
        unsafe {
            if app.icon_green != 0 {
                let _ = DeleteObject(HGDIOBJ(app.icon_green as *mut std::ffi::c_void));
            }
            if app.icon_yellow != 0 {
                let _ = DeleteObject(HGDIOBJ(app.icon_yellow as *mut std::ffi::c_void));
            }
            if app.icon_red != 0 {
                let _ = DeleteObject(HGDIOBJ(app.icon_red as *mut std::ffi::c_void));
            }
        }
    }

    Ok(())
}

/// 分发托盘菜单命令。
///
/// 重启/终止/更新操作在独立线程中执行（避免阻塞 UI 线程），每个线程
/// 独立初始化 COM。切换配置操作在当前线程同步执行。退出操作终止所有
/// 进程并销毁窗口。
fn execute_menu_command(hwnd: isize, id: u16, config_actions: &HashMap<u16, ConfigAction>) {
    let result = match id {
        tray::ID_RESTART_SING
        | tray::ID_RESTART_XRAY
        | tray::ID_RESTART_ALL
        | tray::ID_STOP_SING
        | tray::ID_STOP_XRAY
        | tray::ID_STOP_ALL => {
            let exe_dir = match state::exe_dir() {
                Ok(d) => d,
                Err(e) => {
                    error!("获取 exe 目录失败: {e}");
                    tray::show_error(hwnd, "操作失败", &e.to_string());
                    return;
                }
            };
            std::thread::spawn(move || {
                let _com = ComGuard::new();
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
                    tray::ID_RESTART_SING => process::restart_sing_box_at(&exe_dir),
                    tray::ID_RESTART_XRAY => process::restart_xray_at(&exe_dir),
                    tray::ID_RESTART_ALL => process::restart_all_at(&exe_dir),
                    tray::ID_STOP_SING => process::stop_processes(&["sing-box.exe"]),
                    tray::ID_STOP_XRAY => process::stop_processes(&["xray.exe"]),
                    tray::ID_STOP_ALL => process::stop_all(),
                    _ => unreachable!(),
                };
                if let Err(err) = result {
                    error!("操作失败: {err}");
                    toast::show_toast("操作失败", &err.to_string());
                }
            });
            return;
        }
        tray::ID_UPDATE_ALL | tray::ID_UPDATE_SING | tray::ID_UPDATE_XRAY => {
            let exe_dir = match state::exe_dir() {
                Ok(d) => d,
                Err(e) => {
                    error!("获取 exe 目录失败: {e}");
                    tray::show_error(hwnd, "操作失败", &e.to_string());
                    return;
                }
            };
            let (gh_enabled, gh_url, max_retries, retry_delay) = {
                let app = match state::app_state() {
                    Some(a) => a,
                    None => {
                        error!("应用状态不可用");
                        tray::show_error(hwnd, "操作失败", "应用状态不可用");
                        return;
                    }
                };
                let s = &app.settings;
                (
                    s.gh_proxy.enabled,
                    s.gh_proxy.url.clone(),
                    s.download.max_retries,
                    s.download.retry_delay_secs,
                )
            };
            if let Err(e) = process::stop_all() {
                warn!("更新前终止进程失败: {e}");
            }
            std::thread::spawn(move || {
                let _com = ComGuard::new();
                let label = match id {
                    tray::ID_UPDATE_ALL => "更新所有核心",
                    tray::ID_UPDATE_SING => "更新 sing-box",
                    tray::ID_UPDATE_XRAY => "更新 xray",
                    _ => "",
                };
                info!("{label}");
                let result = match id {
                    tray::ID_UPDATE_ALL => {
                        update::update_cores(&exe_dir, gh_enabled, &gh_url, max_retries, retry_delay)
                    }
                    tray::ID_UPDATE_SING => {
                        update::update_sing_box(&exe_dir, gh_enabled, &gh_url, max_retries, retry_delay)
                    }
                    tray::ID_UPDATE_XRAY => {
                        update::update_xray(&exe_dir, gh_enabled, &gh_url, max_retries, retry_delay)
                    }
                    _ => unreachable!(),
                };
                if let Err(e) = result {
                    error!("更新失败: {e}");
                    toast::show_toast("更新失败", &e.to_string());
                }
            });
            return;
        }
        tray::ID_OPEN_DIR => {
            let exe_dir = match state::exe_dir() {
                Ok(d) => d,
                Err(e) => {
                    error!("获取 exe 目录失败: {e}");
                    return;
                }
            };
            debug!("打开程序目录: {}", exe_dir.display());
            let _ = Command::new("explorer").arg(&exe_dir).spawn();
            return;
        }
        tray::ID_EXIT => {
            info!("退出程序");
            if let Err(e) = process::stop_all() {
                warn!("退出时终止进程失败: {e}");
            }
            unsafe {
                let _ = DestroyWindow(HWND(hwnd as *mut std::ffi::c_void));
            }
            Ok(())
        }
        _ => run_config_action(id, config_actions),
    };

    if let Err(err) = result {
        error!("菜单命令执行失败: {err}");
        toast::show_toast("操作失败", &err.to_string());
    }
}

/// 执行配置切换操作：将选中的配置文件复制到活跃配置路径并重启对应服务。
fn run_config_action(id: u16, config_actions: &HashMap<u16, ConfigAction>) -> Result<(), AppError> {
    let Some(action) = config_actions.get(&id).cloned() else {
        return Ok(());
    };

    let exe_dir = state::exe_dir()?;
    info!("切换配置: {}", action.path.display());
    match action.kind {
        state::ConfigKind::SingBox => {
            let dest = exe_dir.join("configs").join("sing-box.json");
            debug!("复制配置: {} -> {}", action.path.display(), dest.display());
            fs::copy(&action.path, dest).map_err(|e| AppError::Msg(format!("切换 sing-box 配置失败: {e}")))?;
            process::restart_sing_box_at(&exe_dir)
        }
        state::ConfigKind::Xray => {
            let dest = exe_dir.join("configs").join("xray.json");
            debug!("复制配置: {} -> {}", action.path.display(), dest.display());
            fs::copy(&action.path, dest).map_err(|e| AppError::Msg(format!("切换 xray 配置失败: {e}")))?;
            process::restart_xray_at(&exe_dir)
        }
    }
}
