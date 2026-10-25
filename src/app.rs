//! 应用启动、消息循环与正常退出资源清理。

use std::fs;
use tracing::{info, warn};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;

use crate::app_error::AppError;
use crate::app_state::{self, AppState};
use crate::com_guard::ComGuard;
use crate::core::Core;
use crate::settings::Settings;
use crate::single_instance_guard::SingleInstanceGuard;
use crate::{dns, emergency, logging, process, scheduler, toast, tray};

pub(crate) fn run() {
    let _guard = match SingleInstanceGuard::acquire() {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => {
            tray::show_warn(
                0,
                "请勿重复启动",
                "软件已在运行, 请勿重复启动, 你可以在右下角的托盘中找到它",
            );
            return;
        }
        Err(err) => {
            eprintln!("创建单实例互斥体失败: {err}");
            None
        }
    };

    if let Err(err) = run_application() {
        tray::show_error(0, "启动失败", &err.to_string());
    }
}

/// 应用主入口: 初始化 COM → 创建目录 → 加载配置 → 初始化日志和 Toast →
/// 检测版本 → 创建托盘图标 → 运行消息循环 → 退出时清理 GDI 资源。
fn run_application() -> Result<(), AppError> {
    let _com = ComGuard::try_new()?;

    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path
        .parent()
        .ok_or(AppError::Msg("无法获取 exe 所在目录".into()))?
        .to_path_buf();

    for core in Core::ALL {
        fs::create_dir_all(core.core_dir(&exe_dir))?;
        fs::create_dir_all(core.config_dir(&exe_dir))?;
    }

    let app_settings = Settings::load(&exe_dir);

    logging::init_logging(&exe_dir, &app_settings.log)?;
    // 尽早安装: 之后的任何 panic 都要走清理路径恢复 DNS
    emergency::install_panic_hook();
    for w in Settings::take_warnings() {
        warn!("{w}");
    }
    info!("程序启动, exe 所在目录: {}", exe_dir.display());
    process::cleanup_network_registry();
    dns::restore_dns_to_dhcp();
    scheduler::ensure_boot_dns_reset_task(&exe_dir);
    toast::setup(&exe_path).map_err(|e| AppError::Msg(format!("初始化 Toast 通知失败: {e}")))?;

    let icons = tray::load_status_icons(&exe_dir);

    app_state::initialize(AppState {
        exe_dir: exe_dir.clone(),
        icons,
        settings: app_settings,
        child_sing_box: None,
        child_xray: None,
    })?;

    // SAFETY: None 请求当前可执行模块, 它在整个进程运行期保持加载; 不手动释放。
    let h_instance = unsafe { GetModuleHandleW(None) }
        .map_err(|e| AppError::Msg(format!("获取模块句柄失败: {e}")))?
        .0 as isize;
    // SAFETY: 模块句柄有效, 创建和后续消息分发均在当前主线程, 回调存活至进程结束。
    let hwnd = unsafe { tray::create_window(h_instance) }?;
    // SAFETY: hwnd 刚由本线程创建且未销毁; 模块和路径在同步注册期间有效。
    unsafe { tray::add_icon(hwnd, h_instance, &exe_dir) }?;
    tray::set_tooltip("ladder");
    // SAFETY: 当前主线程拥有托盘窗口, 循环中保持 COM 守卫和应用资源存活。
    unsafe { tray::run_message_loop() };

    let _ = app_state::with_app_state(|app| {
        // SAFETY: 主线程消息循环已结束, 弹出菜单已关闭; 位图由本次启动创建,
        // 仅在此释放一次, 后续不再使用这些句柄。
        unsafe { app.icons.delete() };
    });

    Ok(())
}
