mod app_update;
mod app_updater;
mod core;
mod core_output;
mod desktop;
mod dns;
mod emergency;
mod github;
mod i18n;
mod latency;
mod live_proxy;
mod logging;
mod managed_child;
mod panic_helper;
mod runtime;
mod scheduler;
mod tun_cleanup;
mod tun_config;
mod ui;
mod update;
mod wireguard;

use std::{cell::Cell, process::ExitCode, ptr, rc::Rc};

use anyhow::{Context as _, Result};
use async_channel::{Receiver, Sender};
use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowHandle,
    WindowOptions, px, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
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
use i18n::{Language, app as t};
use runtime::Runtime;
use ui::MainWindow;

#[derive(Clone)]
enum TrayAction {
    Open,
    Hide,
    Exit,
    OpenDirectory,
    InstallUpdate,
    LanguageChanged,
}

fn tray_menu() -> Result<Menu> {
    let menu = Menu::new();
    menu.append(&MenuItem::with_id("open", t::open(), true, None))?;
    menu.append(&MenuItem::with_id("directory", t::directory(), true, None))?;
    menu.append(&MenuItem::with_id("exit", t::exit(), true, None))?;
    Ok(menu)
}

fn tray_action(id: &str) -> Option<TrayAction> {
    match id {
        "open" => Some(TrayAction::Open),
        "exit" => Some(TrayAction::Exit),
        "directory" => Some(TrayAction::OpenDirectory),
        _ => None,
    }
}

fn install_tray(events: &Sender<TrayAction>) -> Result<TrayIcon> {
    let tray = TrayIconBuilder::new()
        .with_icon(Icon::from_resource(1, Some((32, 32)))?)
        .with_tooltip("Ladder")
        .with_menu(Box::new(tray_menu()?))
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
            .map_err(|e| anyhow::anyhow!(t::window_handle(e)))?
            .as_raw()
        {
            RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get() as HWND),
            _ => anyhow::bail!(t::missing_handle()),
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
    let tray = install_tray(&events)?;
    if !startup.settings.minimize_to_tray
        || startup.settings_error.is_some()
        || startup.dns_recovery_error.is_some()
        || startup.dns_task_warning.is_some()
    {
        window = Some(open_main(cx, &runtime, &updater, &events)?);
    }
    cx.spawn(async move |cx| {
        let tray = tray;
        while let Ok(action) = incoming.recv().await {
            match action {
                TrayAction::LanguageChanged => match tray_menu() {
                    Ok(menu) => tray.set_menu(Some(Box::new(menu))),
                    Err(error) => report(&t::tray_update(error)),
                },
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
                                report(&t::update_cancelled(error));
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
                TrayAction::OpenDirectory => {
                    let result =
                        std::env::current_exe()
                            .map_err(t::read_directory)
                            .and_then(|exe| {
                                desktop::open_folder(exe.parent().ok_or(t::missing_directory())?)
                            });
                    if let Err(error) = result {
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
                        report(&t::window_operation(error));
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
    let base = exe.parent().context(t::missing_directory())?.to_owned();
    i18n::set_current(runtime::saved_language(&base));
    // 日志失败不能阻止核心管理或 DNS 恢复；界面通过 warning 展示故障。
    let _ = logging::init(&base.join("app.log"));
    logging::info(
        "app",
        format_args!(
            "Ladder {} 启动, 架构 {}, 进程 {}",
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
            report(&t::start_ui(error));
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
    message_box(&t::exit_dns_warning(message), MB_ICONWARNING);
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
    i18n::set_current(Language::system());
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
