mod core_output;
mod desktop;
mod dns;
mod managed_child;
mod runtime;
mod scheduler;
mod ui;

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

use runtime::Runtime;
use ui::MainWindow;

#[derive(Clone, Copy)]
enum TrayAction {
    Open,
    Hide,
    Exit,
}

fn install_tray(events: &Sender<TrayAction>) -> Result<TrayIcon> {
    let menu = Menu::new();
    let exit = MenuItem::with_id("exit", "退出并终止", true, None);
    menu.append(&exit)?;
    let tray = TrayIconBuilder::new()
        .with_icon(Icon::from_resource(1, Some((32, 32)))?)
        .with_tooltip("Ladder")
        .with_menu(Box::new(menu))
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
        if event.id == "exit" {
            let _ = commands.try_send(TrayAction::Exit);
        }
    }));
    Ok(tray)
}

fn open_main(
    cx: &mut App,
    runtime: &Runtime,
    events: &Sender<TrayAction>,
) -> Result<WindowHandle<MainWindow>> {
    let bounds = Bounds::centered(None, size(px(820.), px(510.)), cx);
    let close = events.clone();
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(660.), px(440.))),
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
            cx.new(|cx| MainWindow::new(runtime.clone(), window, cx))
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
    let tray = install_tray(&events)?;
    let mut window = None;
    let startup = runtime.snapshot();
    if !startup.settings.minimize_to_tray
        || startup.dns_recovery_error.is_some()
        || startup.dns_task_warning.is_some()
    {
        window = Some(open_main(cx, &runtime, &events)?);
    }
    cx.spawn(async move |cx| {
        let _tray = tray;
        while let Ok(action) = incoming.recv().await {
            match action {
                TrayAction::Exit => {
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
                                window = Some(open_main(cx, &runtime, &events)?);
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
    Ok(())
}

fn report(message: &str) {
    message_box(message, MB_ICONERROR);
}

fn report_dns_on_exit(message: &str) {
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
    if std::env::args_os().nth(1).as_deref() == Some(scheduler::BOOT_ARGUMENT.as_ref()) {
        return match scheduler::restore_on_boot() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
    if let Err(error) = run() {
        report(&format!("{error:#}"));
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
