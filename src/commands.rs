//! 托盘菜单命令映射、执行与后台任务调度。

use std::collections::HashMap;
use std::ffi::c_void;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread::Builder;
use tracing::{debug, error, info, warn};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;

use crate::app_error::AppError;
use crate::app_state::{self, FlagGuard};
use crate::com_guard::ComGuard;
use crate::config_selection::ConfigSelection;
use crate::core::Core;
use crate::settings::CoreMode;
use crate::{process, toast, tray, update};

/// 把操作放到后台线程执行, 避免阻塞托盘的消息循环。
///
/// `guard` 被 move 进线程, 闭包结束时随之释放忙标志。线程创建失败时闭包连同
/// `guard` 一起在此处 drop, 同样不会泄漏忙标志。
///
/// 线程内独立初始化 COM: Toast 通知需要 COM, 而 COM 的初始化是线程局部的。
///
/// 闭包内的 panic 不在这里兜底 —— release 构建是 abort, 展开根本不会发生,
/// 清理由 `emergency::install_panic_hook` 负责。`name` 会成为线程名, panic 日志据此
/// 指出是哪个后台操作出的问题。
fn spawn_background<F: FnOnce() + Send + 'static>(name: &'static str, guard: FlagGuard, f: F) {
    let spawned = Builder::new().name(name.to_owned()).spawn(move || {
        let _busy = guard;
        let _com = match ComGuard::try_new() {
            Ok(c) => Some(c),
            Err(e) => {
                warn!("COM 初始化失败: {e}");
                None
            }
        };
        f();
    });

    if let Err(e) = spawned {
        error!("创建后台线程 [{name}] 失败: {e}");
        toast::show_toast("操作失败", "无法创建后台线程, 请稍后重试");
    }
}

/// 以 exe 目录为唯一参数的操作。
type ExeDirFn = fn(&Path) -> Result<(), AppError>;

/// 重启 / 终止类菜单命令。
struct ServiceCommand {
    label: &'static str,
    run: ExeDirFn,
}

/// 核心更新类菜单命令。
struct UpdateCommand {
    label: &'static str,
    run: fn(&Path, &str, u32, u64) -> Result<(), AppError>,
}

/// 把菜单 ID 映射为重启 / 终止命令, 未命中返回 `None`。
///
/// 标签和执行体绑定在同一张表里, 新增菜单项时不会出现"外层 match 加了 ID
/// 但内层忘了写实现"的错配, 因此不再需要 `unreachable!` 兜底。
fn service_command(id: u16) -> Option<ServiceCommand> {
    let cmd = match id {
        tray::ID_RESTART_SING => ServiceCommand {
            label: "重启 sing-box",
            run: process::restart_sing_box_at,
        },
        tray::ID_RESTART_XRAY => ServiceCommand {
            label: "重启 xray",
            run: process::restart_xray_at,
        },
        tray::ID_RESTART_ALL => ServiceCommand {
            label: "重启所有服务",
            run: process::restart_all_at,
        },
        tray::ID_STOP_SING => ServiceCommand {
            label: "终止 sing-box",
            run: |_| process::stop_processes(&[Core::SingBox]),
        },
        tray::ID_STOP_XRAY => ServiceCommand {
            label: "终止 xray",
            run: |_| process::stop_processes(&[Core::Xray]),
        },
        tray::ID_STOP_ALL => ServiceCommand {
            label: "终止所有服务",
            run: |_| process::stop_all(),
        },
        _ => return None,
    };
    Some(cmd)
}

/// 把菜单 ID 映射为核心更新命令, 未命中返回 `None`。
fn update_command(id: u16) -> Option<UpdateCommand> {
    let cmd = match id {
        tray::ID_UPDATE_ALL => UpdateCommand {
            label: "更新所有核心",
            run: update::update_cores,
        },
        tray::ID_UPDATE_SING => UpdateCommand {
            label: "更新 sing-box",
            run: update::update_sing_box,
        },
        tray::ID_UPDATE_XRAY => UpdateCommand {
            label: "更新 xray",
            run: update::update_xray,
        },
        _ => return None,
    };
    Some(cmd)
}

/// 把菜单 ID 映射为目标核心模式, 未命中返回 `None`。
fn switch_core_mode(id: u16) -> Option<CoreMode> {
    match id {
        tray::ID_SWITCH_CORE_XRAY => Some(CoreMode::Xray),
        tray::ID_SWITCH_CORE_SING => Some(CoreMode::SingBox),
        tray::ID_SWITCH_CORE_BOTH => Some(CoreMode::Both),
        _ => None,
    }
}

/// 分发托盘菜单命令。
///
/// 耗时操作 (重启 / 终止 / 更新 / 切换配置) 都交给后台线程, 避免阻塞消息
/// 循环。忙标志由 `FlagGuard` 管理: 交给后台线程的分支在线程结束时释放,
/// 其余分支在本函数返回时释放, 提前返回不会漏。
pub(crate) fn execute_menu_command(
    hwnd: isize,
    id: u16,
    config_selections: &HashMap<u16, ConfigSelection>,
) {
    // 打开目录和退出不占用忙标志: 前者只是拉起 explorer, 后者必须随时可用。
    match id {
        tray::ID_OPEN_DIR => return open_exe_dir(),
        tray::ID_EXIT => return exit_app(hwnd),
        _ => {}
    }

    let Some(guard) = app_state::acquire_busy() else {
        toast::show_toast("操作进行中", "请等待当前操作完成");
        return;
    };

    if let Some(cmd) = service_command(id) {
        run_service_command(hwnd, guard, cmd);
    } else if let Some(cmd) = update_command(id) {
        run_update_command(hwnd, guard, cmd);
    } else if let Some(mode) = switch_core_mode(id) {
        run_switch_core(guard, mode);
    } else if let Some(action) = config_selections.get(&id).cloned() {
        run_config_switch(guard, action);
    } else {
        debug!("忽略未知菜单项: {id}");
    }
}

fn open_exe_dir() {
    let exe_dir = match app_state::exe_dir() {
        Ok(d) => d,
        Err(e) => {
            error!("获取 exe 目录失败: {e}");
            return;
        }
    };
    info!("打开程序目录: {}", exe_dir.display());
    let _ = Command::new("explorer").arg(&exe_dir).spawn();
}

fn exit_app(hwnd: isize) {
    info!("退出程序");
    // 这里刻意同步执行: 一旦 DestroyWindow 触发消息循环退出, 进程很快就会
    // 结束, 放到后台线程会来不及恢复物理网卡 DNS。
    if let Err(e) = process::stop_all() {
        warn!("退出时终止进程失败: {e}");
    }
    unsafe {
        let _ = DestroyWindow(HWND(hwnd as *mut c_void));
    }
}

fn run_service_command(hwnd: isize, guard: FlagGuard, cmd: ServiceCommand) {
    let exe_dir = match app_state::exe_dir() {
        Ok(d) => d,
        Err(e) => {
            error!("获取 exe 目录失败: {e}");
            tray::show_error(hwnd, "操作失败", &e.to_string());
            return;
        }
    };
    spawn_background("bg-restart-stop", guard, move || {
        info!("{}", cmd.label);
        if let Err(err) = (cmd.run)(&exe_dir) {
            error!("操作失败: {err}");
            toast::show_toast("操作失败", &err.to_string());
        }
    });
}

fn run_update_command(hwnd: isize, guard: FlagGuard, cmd: UpdateCommand) {
    // 规则集更新也在往核心目录写文件, 而且同样是几十 MB 的下载。两件事不并行:
    // 规则集是切换配置的副作用、已经在下载中, 中断它等于白下一遍; 让用户等十几
    // 秒再点更新核心的代价小得多。
    //
    // 反方向不需要检查: 规则集更新只由 restart_all_at / restart_xray_at 触发,
    // 而这两个只在占着忙标志的后台线程里跑, 本次更新期间用户点不到它们。
    if process::ruleset_updating() {
        info!("规则集更新进行中, 本次核心更新取消");
        toast::show_toast("请稍后再试", "规则集正在更新, 完成后再更新核心");
        return;
    }

    let exe_dir = match app_state::exe_dir() {
        Ok(d) => d,
        Err(e) => {
            error!("获取 exe 目录失败: {e}");
            tray::show_error(hwnd, "操作失败", &e.to_string());
            return;
        }
    };
    let (gh_proxy, max_retries, delay_secs) = {
        let app = match app_state::app_state() {
            Some(a) => a,
            None => {
                error!("应用状态不可用");
                tray::show_error(hwnd, "操作失败", "应用状态不可用");
                return;
            }
        };
        let s = &app.settings;
        (
            s.download.core.gh_proxy.clone(),
            s.download.retry.max_retries,
            s.download.retry.delay_secs,
        )
    };
    spawn_background("bg-update", guard, move || {
        info!("{}", cmd.label);
        // 放在后台线程里终止进程: stop_all 会枚举进程、逐块物理网卡读写注册表
        // 恢复 DNS 并刷新 DNS 缓存, 在 UI 线程执行会卡住托盘菜单。
        if let Err(e) = process::stop_all() {
            warn!("更新前终止进程失败: {e}");
        }
        if let Err(e) = (cmd.run)(&exe_dir, &gh_proxy, max_retries, delay_secs) {
            error!("更新失败: {e}");
            toast::show_toast("更新失败", &e.to_string());
        }
    });
}

fn run_switch_core(guard: FlagGuard, new_mode: CoreMode) {
    spawn_background("bg-switch-core", guard, move || {
        // 与更新核心同理: stop_all 涉及进程枚举和注册表读写, 不能在 UI 线程做。
        if let Err(e) = process::stop_all() {
            warn!("切换核心前终止进程失败: {e}");
        }
        {
            let mut app = match app_state::app_state_mut() {
                Some(a) => a,
                None => {
                    error!("应用状态不可用");
                    toast::show_toast("操作失败", "应用状态不可用");
                    return;
                }
            };
            app.settings.core.mode = new_mode;
            if let Err(e) = app.settings.save(&app.exe_dir) {
                error!("保存核心模式失败: {e}");
                toast::show_toast("操作失败", &e.to_string());
                return;
            }
        }
        info!("核心模式已切换为: {new_mode:?}");
    });
}

fn run_config_switch(guard: FlagGuard, action: ConfigSelection) {
    spawn_background("bg-config-switch", guard, move || {
        let exe_dir = match app_state::exe_dir() {
            Ok(d) => d,
            Err(e) => {
                error!("获取 exe 目录失败: {e}");
                return;
            }
        };
        let restart: ExeDirFn = match action.core {
            Core::SingBox => process::restart_sing_box_at,
            Core::Xray => process::restart_xray_at,
        };
        let dest = action.core.active_config(&exe_dir);

        info!("切换配置: {}", action.path.display());
        debug!("复制配置: {} -> {}", action.path.display(), dest.display());
        if let Err(e) = fs::copy(&action.path, &dest) {
            error!("复制配置失败: {e}");
            toast::show_toast("操作失败", &e.to_string());
            return;
        }
        if let Err(err) = restart(&exe_dir) {
            error!("操作失败: {err}");
            toast::show_toast("操作失败", &err.to_string());
        }
    });
}
