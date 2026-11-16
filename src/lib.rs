mod app_update;
mod app_updater;
mod core_output;
mod desktop;
mod dns;
mod emergency;
mod github;
mod logging;
mod managed_child;
mod panic_helper;
mod runtime;
mod scheduler;
mod tun_cleanup;
mod tun_config;
mod ui;
mod update;

use std::{cell::Cell, process::ExitCode, ptr, rc::Rc, time::Duration};

use anyhow::{Context as _, Result};
use async_channel::{Receiver, Sender};
use gpui::{
    App, AppContext, Application, Bounds, Timer, TitlebarOptions, WindowBounds, WindowHandle,
    WindowOptions, px, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{
        IsIconic, MB_ICONERROR, MB_ICONWARNING, MB_OK, MessageBoxW, SW_HIDE, SW_RESTORE, SW_SHOW,
        SetForegroundWindow, ShowWindow,
    },
};
use windows_sys::w;

use app_updater::AppUpdater;
use runtime::{Action, Core, Runtime, Snapshot};
use ui::MainWindow;

#[derive(Clone)]
enum TrayAction {
    Open,
    Hide,
    Exit,
    Refresh,
    OpenDirectory,
    Core(Action),
    InstallUpdate,
}

fn tray_menu(snapshot: &Snapshot) -> Result<Menu> {
    let menu = Menu::new();
    menu.append(&MenuItem::with_id("open", "打开 Ladder", true, None))?;
    menu.append(&PredefinedMenuItem::separator())?;
    for (index, core) in Core::ALL.into_iter().enumerate() {
        let state = snapshot.core(core);
        let submenu = Submenu::new(core.label(), true);
        let idle = state.operation.is_none();
        submenu.append(&MenuItem::with_id(
            format!("restart:{index}"),
            "重启核心",
            idle && state.installed && state.current.is_some(),
            None,
        ))?;
        submenu.append(&MenuItem::with_id(
            format!("stop:{index}"),
            "停止核心",
            idle && state.running,
            None,
        ))?;
        let configs = Submenu::new("切换配置", idle && !state.configs.is_empty());
        for name in &state.configs {
            let current = state.current.as_ref() == Some(name);
            // 原生菜单把 & 解释为快捷键标记；显示原始文件名时须转义。
            let label = name.replace('&', "&&");
            configs.append(&MenuItem::with_id(
                format!("config:{index}:{name}"),
                if current {
                    format!("{label}（当前使用）")
                } else {
                    label
                },
                idle && !current,
                None,
            ))?;
        }
        submenu.append(&configs)?;
        menu.append(&submenu)?;
    }
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&MenuItem::with_id("directory", "打开程序目录", true, None))?;
    menu.append(&MenuItem::with_id("exit", "退出并终止", true, None))?;
    Ok(menu)
}

fn tray_action(id: &str) -> Option<TrayAction> {
    match id {
        "open" => Some(TrayAction::Open),
        "exit" => Some(TrayAction::Exit),
        "directory" => Some(TrayAction::OpenDirectory),
        _ => {
            let mut parts = id.splitn(3, ':');
            let command = parts.next()?;
            let core = *Core::ALL.get(parts.next()?.parse::<usize>().ok()?)?;
            let action = match command {
                "restart" => Action::Restart(core),
                "stop" => Action::Stop(core),
                "config" => Action::SwitchConfig(core, parts.next()?.to_owned()),
                _ => return None,
            };
            Some(TrayAction::Core(action))
        }
    }
}

fn tray_changed(previous: &Snapshot, current: &Snapshot) -> bool {
    Core::ALL.into_iter().any(|core| {
        let previous = previous.core(core);
        let current = current.core(core);
        previous.configs != current.configs
            || previous.current != current.current
            || previous.installed != current.installed
            || previous.running != current.running
            || previous.operation != current.operation
    })
}

fn install_tray(events: &Sender<TrayAction>, snapshot: &Snapshot) -> Result<TrayIcon> {
    let tray = TrayIconBuilder::new()
        .with_icon(Icon::from_resource(1, Some((32, 32)))?)
        .with_tooltip("Ladder")
        .with_menu(Box::new(tray_menu(snapshot)?))
        .with_menu_on_left_click(false)
        .build()?;
    let clicks = events.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            let _ = clicks.try_send(TrayAction::Open);
        }
    }));
    let commands = events.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(action) = tray_action(event.id.as_ref()) {
            let _ = commands.try_send(action);
        }
    }));
    Ok(tray)
}

fn open_main(
    cx: &mut App,
    runtime: &Runtime,
    updater: &AppUpdater,
    events: &Sender<TrayAction>,
) -> Result<WindowHandle<MainWindow>> {
    let window_size = size(px(900.), px(600.));
    let bounds = Bounds::centered(None, window_size, cx);
    let close = events.clone();
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(window_size),
            titlebar: Some(TitlebarOptions {
                title: Some("Ladder".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| {
            window.on_window_should_close(cx, move |_, _| {
                let _ = close.try_send(TrayAction::Hide);
                false
            });
            cx.new(|cx| {
                MainWindow::new(runtime.clone(), updater.clone(), events.clone(), window, cx)
            })
        },
    )
}

fn show_main(cx: &mut App, handle: WindowHandle<MainWindow>, show: bool) -> Result<()> {
    let hwnd = handle.update(cx, |_, window, _| {
        match window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("读取主窗口句柄失败: {e}"))?
            .as_raw()
        {
            RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get() as HWND),
            _ => anyhow::bail!("无法获取主窗口的 Windows 句柄"),
        }
    })??;
    // GPUI owns this HWND, and this function runs on its Windows message thread.
    unsafe {
        if show {
            ShowWindow(
                hwnd,
                if IsIconic(hwnd) != 0 {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            );
            SetForegroundWindow(hwnd);
        } else {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
    Ok(())
}

fn start_desktop(cx: &mut App, runtime: Runtime, cleaned: Rc<Cell<bool>>) -> Result<()> {
    let (events, incoming): (_, Receiver<TrayAction>) = async_channel::unbounded();
    let mut window = None;
    let startup = runtime.snapshot();
    let updater = AppUpdater::default();
    let tray = install_tray(&events, &startup)?;
    if !startup.settings.minimize_to_tray
        || startup.settings_error.is_some()
        || startup.dns_recovery_error.is_some()
        || startup.dns_task_warning.is_some()
    {
        window = Some(open_main(cx, &runtime, &updater, &events)?);
    }
    let refresh = events.clone();
    cx.spawn(async move |_| {
        loop {
            Timer::after(Duration::from_millis(500)).await;
            if refresh.try_send(TrayAction::Refresh).is_err() {
                break;
            }
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        let mut tray_snapshot = startup;
        while let Ok(action) = incoming.recv().await {
            match action {
                TrayAction::InstallUpdate => {
                    let prepared = match updater.take_prepared() {
                        Ok(prepared) => prepared,
                        Err(error) => {
                            updater.fail(error);
                            continue;
                        }
                    };
                    let worker = runtime.clone();
                    let result = cx
                        .background_executor()
                        .spawn(async move { worker.prepare_app_update() })
                        .await;
                    if let Err(error) = result {
                        updater.fail(error);
                        continue;
                    }
                    let helper = match cx
                        .background_executor()
                        .spawn(async move { prepared.launch() })
                        .await
                    {
                        Ok(helper) => helper,
                        Err(error) => {
                            runtime.cancel_app_update();
                            updater.fail(error);
                            continue;
                        }
                    };
                    let worker = runtime.clone();
                    let result = cx
                        .background_executor()
                        .spawn(async move { worker.shutdown() })
                        .await;
                    match result {
                        Ok(warning) => {
                            cleaned.set(true);
                            if let Some(message) = warning {
                                drop(helper);
                                report_dns_on_exit(&message);
                            } else if let Err(error) = helper.commit() {
                                report(&format!("{error}。更新已取消，请重新打开 Ladder 后重试。"));
                            }
                            let _ = cx.update(|cx| cx.quit());
                            break;
                        }
                        Err(error) => {
                            drop(helper);
                            runtime.cancel_app_update();
                            updater.fail(error);
                        }
                    }
                }
                TrayAction::Refresh => {
                    let snapshot = runtime.snapshot();
                    if tray_changed(&tray_snapshot, &snapshot) {
                        match tray_menu(&snapshot) {
                            Ok(menu) => {
                                tray.set_menu(Some(Box::new(menu)));
                                tray_snapshot = snapshot;
                            }
                            Err(error) => {
                                logging::error("tray", format_args!("更新托盘菜单失败：{error}"))
                            }
                        }
                    }
                }
                TrayAction::OpenDirectory => {
                    let result = std::env::current_exe()
                        .map_err(|error| format!("读取程序目录失败：{error}"))
                        .and_then(|exe| {
                            desktop::open_folder(exe.parent().ok_or("无法定位程序目录")?)
                        });
                    if let Err(error) = result {
                        report(&error);
                    }
                }
                TrayAction::Core(action) => {
                    if let Err(error) = runtime.send(action) {
                        report(&error);
                    }
                }
                TrayAction::Exit => {
                    logging::info("app", "正在退出 Ladder");
                    let worker = runtime.clone();
                    let result = cx
                        .background_executor()
                        .spawn(async move { worker.shutdown() })
                        .await;
                    match result {
                        Ok(warning) => {
                            cleaned.set(true);
                            if let Some(message) = warning {
                                report_dns_on_exit(&message);
                            }
                            let _ = cx.update(|cx| cx.quit());
                            break;
                        }
                        Err(message) => report(&message),
                    }
                }
                TrayAction::Open | TrayAction::Hide => {
                    let result = cx.update(|cx| -> Result<()> {
                        match window {
                            Some(handle) => {
                                show_main(cx, handle, matches!(action, TrayAction::Open))
                            }
                            None if matches!(action, TrayAction::Open) => {
                                window = Some(open_main(cx, &runtime, &updater, &events)?);
                                Ok(())
                            }
                            None => Ok(()),
                        }
                    });
                    if let Err(error) = result.and_then(|r| r) {
                        report(&format!("窗口操作失败: {error}"));
                    }
                }
            }
        }
    })
    .detach();
    Ok(())
}

struct Instance(HANDLE);

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn run() -> Result<()> {
    // GUI 单实例与后台开机恢复使用各自的锁。
    let handle = unsafe { CreateMutexW(ptr::null(), 0, w!("Global\\LadderDesktop.Singleton")) };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let _instance = Instance(handle);
    if existed {
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let base = exe.parent().context("无法定位 Ladder 程序目录")?.to_owned();
    // 日志失败不能阻止核心管理或 DNS 恢复；界面通过 warning 展示故障。
    let _ = logging::init(&base.join("app.log"));
    logging::info(
        "app",
        format_args!(
            "Ladder {} 启动，架构 {}，进程 {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::ARCH,
            std::process::id()
        ),
    );
    let (dns, warning) = scheduler::desktop_dns(&base);
    let runtime = Runtime::open_with_dns(base, dns, warning).map_err(anyhow::Error::msg)?;
    let cleaned = Rc::new(Cell::new(false));
    let clean_on_quit = cleaned.clone();
    let desktop_runtime = runtime.clone();
    Application::new().run(move |cx| {
        if let Err(error) = start_desktop(cx, desktop_runtime, clean_on_quit) {
            report(&format!("无法启动 Ladder 界面: {error}"));
            cx.quit();
        }
    });
    if !cleaned.get()
        && let Some(message) = runtime.shutdown().map_err(anyhow::Error::msg)?
    {
        report_dns_on_exit(&message);
    }
    logging::info("app", "Ladder 已退出");
    Ok(())
}

fn report(message: &str) {
    logging::error("app", message);
    message_box(message, MB_ICONERROR);
}

fn report_dns_on_exit(message: &str) {
    logging::warn("dns", message);
    message_box(
        &format!("核心已停止，DNS 尚未恢复。下次打开 Ladder 时会再次尝试恢复。\n\n{message}"),
        MB_ICONWARNING,
    );
}

fn message_box(message: &str, icon: u32) {
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            message.as_ptr(),
            w!("Ladder"),
            MB_OK | icon,
        );
    }
}

pub fn launch() -> ExitCode {
    logging::install_panic_hook();
    if std::env::args_os().nth(1).as_deref() == Some(emergency::ARGUMENT.as_ref()) {
        return match emergency::restore_after_panic() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
    if std::env::args_os().nth(1).as_deref() == Some(scheduler::BOOT_ARGUMENT.as_ref()) {
        return match scheduler::restore_on_boot() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
    emergency::install_panic_hook();
    if let Err(error) = run() {
        report(&format!("{error:#}"));
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
