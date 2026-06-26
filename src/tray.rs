//! Windows 系统托盘 UI。
//!
//! 负责创建隐藏窗口、注册托盘图标、构建右键弹出菜单（重启/终止/更新/切换配置/退出），
//! 处理鼠标消息并将菜单事件分发给 `main::execute_menu_command`。

use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, GetDC, ReleaseDC,
    SelectObject, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
    DefWindowProcW, DestroyIcon, DestroyMenu, DispatchMessageW, DrawIconEx, GetCursorPos,
    GetMenuItemCount, GetMessageW,
    HICON, HMENU, IDI_APPLICATION, IMAGE_ICON, LoadIconW, LoadImageW,
    LR_DEFAULTSIZE, LR_LOADFROMFILE, MB_ICONERROR, MB_OK, MENUITEMINFOW, MIIM_BITMAP,
    MessageBoxW, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, SetMenuItemInfoW, TrackPopupMenu, TranslateMessage,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_DESTROY, WM_LBUTTONUP, WM_NULL,
    WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED, DI_NORMAL,
};

use tracing::warn;
use crate::{AppState, ConfigAction, ConfigKind, ProcessState};

/// 托盘图标的自定义消息 ID，当托盘收到鼠标事件时通过此消息通知窗口。
pub const WM_TRAY_ICON: u32 = WM_APP + 1;
const TRAY_UID: u32 = 1;

static TRAY_HWND: OnceLock<isize> = OnceLock::new();

// 菜单 ID 分段规划：101-199 重启，201-299 终止，301-399 更新，999 退出，
// 1000-1999 sing-box 配置，2000-2999 xray 配置。
pub const ID_RESTART_SING: u16 = 101;
pub const ID_RESTART_XRAY: u16 = 102;
pub const ID_RESTART_ALL: u16 = 103;
pub const ID_STOP_SING: u16 = 201;
pub const ID_STOP_XRAY: u16 = 202;
pub const ID_STOP_ALL: u16 = 203;
pub const ID_UPDATE_ALL: u16 = 301;
pub const ID_UPDATE_SING: u16 = 302;
pub const ID_UPDATE_XRAY: u16 = 303;
pub const ID_OPEN_DIR: u16 = 998;
pub const ID_EXIT: u16 = 999;
pub const ID_SING_CONFIG_BASE: u16 = 1000;
pub const ID_XRAY_CONFIG_BASE: u16 = 2000;

/// 创建托盘图标所依附的隐藏窗口。返回窗口句柄。
pub unsafe fn create_window(h_instance: isize) -> Result<isize, String> {
    let h_instance = HINSTANCE(h_instance as _);
    let class_name = to_wide("SingBoxWithXrayTrayWindow");

    let wnd_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: h_instance,
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };

    if RegisterClassW(&wnd_class) == 0 {
        return Err("注册托盘窗口类失败".to_string());
    }

    let title = to_wide("sing-box_with_xray");
    let hwnd = CreateWindowExW(
        Default::default(),
        PCWSTR(class_name.as_ptr()),
        PCWSTR(title.as_ptr()),
        WS_OVERLAPPED,
        CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT,
        None, None, Some(h_instance), None,
    );

    let Ok(hwnd) = hwnd else {
        return Err("创建托盘窗口失败".to_string());
    };

    Ok(hwnd.0 as isize)
}

/// 向系统托盘添加图标并注册鼠标事件回调。
pub unsafe fn add_icon(hwnd: isize, h_instance: isize, exe_dir: &Path) -> Result<(), String> {
    let hwnd = HWND(hwnd as _);
    let h_instance = HINSTANCE(h_instance as _);
    let icon = load_app_icon(Some(h_instance), exe_dir);
    let nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_UID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY_ICON,
        hIcon: icon,
        szTip: to_wide_padded("sing-box_with_xray"),
        ..Default::default()
    };

    if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
        return Err("添加系统托盘图标失败".to_string());
    }

    let _ = TRAY_HWND.set(hwnd.0 as isize);
    Ok(())
}

/// 运行 Windows 消息循环，阻塞直到窗口被销毁。
pub unsafe fn run_message_loop() {
    let mut msg: MSG = Default::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

/// 显示错误消息对话框。
pub fn show_error(hwnd: isize, title: &str, message: &str) {
    let hwnd = if hwnd == 0 { None } else { Some(HWND(hwnd as _)) };
    unsafe {
        let _ = MessageBoxW(hwnd, &HSTRING::from(message), &HSTRING::from(title), MB_OK | MB_ICONERROR);
    }
}

/// 更新托盘图标的悬停提示文本。
pub fn set_tooltip(text: &str) {
    if let Some(&hwnd_val) = TRAY_HWND.get() {
        let hwnd = HWND(hwnd_val as _);
        unsafe {
            let nid = NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: TRAY_UID,
                uFlags: NIF_TIP,
                szTip: to_wide_padded(text),
                ..Default::default()
            };
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }
}

/// 从 ICO 文件加载图标并转换为 32 位 DIB 位图句柄。
///
/// 流程：LoadImageW 加载 ICO → 创建兼容 DC → 创建 DIB Section →
/// DrawIconEx 绘制到位图 → 清理中间资源 → 返回位图句柄。
/// 菜单项需要位图句柄（而非图标句柄）来显示状态图标。
pub(crate) unsafe fn load_icon_bitmap(exe_dir: &Path, icon_name: &str) -> isize {
    let icon_path = exe_dir.join("icons").join(icon_name);
    if !icon_path.exists() {
        warn!("图标文件不存在: {}", icon_path.display());
        return 0;
    }
    let icon_path_w: Vec<u16> = icon_path.to_string_lossy().encode_utf16().chain(Some(0)).collect();
    let hicon = LoadImageW(
        None,
        PCWSTR(icon_path_w.as_ptr()),
        IMAGE_ICON,
        16, 16,
        LR_LOADFROMFILE | LR_DEFAULTSIZE,
    );
    let Ok(hicon) = hicon else {
        warn!("加载图标失败: {}", icon_path.display());
        return 0;
    };
    let hicon = HICON(hicon.0);

    let screen_dc = GetDC(None);
    if screen_dc.is_invalid() {
        let _ = DestroyIcon(hicon);
        return 0;
    }
    let mem_dc = CreateCompatibleDC(Some(screen_dc));
    if mem_dc.is_invalid() {
        let _ = ReleaseDC(None, screen_dc);
        let _ = DestroyIcon(hicon);
        return 0;
    }

    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: 16,
            biHeight: 16,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut pv_bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let Ok(bmp) = CreateDIBSection(
        Some(screen_dc),
        &bmi,
        DIB_RGB_COLORS,
        &mut pv_bits,
        None,
        0,
    ) else {
        let _ = DeleteDC(mem_dc);
        let _ = DestroyIcon(hicon);
        return 0;
    };

    let old_bmp = SelectObject(mem_dc, HGDIOBJ(bmp.0));
    let _ = DrawIconEx(
        mem_dc, 0, 0,
        hicon,
        16, 16, 0,
        None,
        DI_NORMAL,
    );
    SelectObject(mem_dc, old_bmp);
    let _ = DeleteDC(mem_dc);
    let _ = DestroyIcon(hicon);

    bmp.0 as isize
}

/// 窗口过程：处理托盘图标鼠标事件和窗口销毁。
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAY_ICON => {
            let event = lparam.0 as u32;
            if event == WM_LBUTTONUP || event == WM_RBUTTONUP {
                let selected = show_tray_menu(hwnd);
                if selected != 0 {
                    crate::execute_menu_command(hwnd.0 as isize, selected);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            remove_tray_icon(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn load_app_icon(h_instance: Option<HINSTANCE>, exe_dir: &Path) -> HICON {
    let icon_path = exe_dir.join("icons").join("ladder.ico");
    if icon_path.exists() {
        let icon_path_w: Vec<u16> = icon_path.to_string_lossy().encode_utf16().chain(Some(0)).collect();
        let icon = LoadImageW(
            h_instance,
            PCWSTR(icon_path_w.as_ptr()),
            IMAGE_ICON,
            0, 0,
            LR_LOADFROMFILE | LR_DEFAULTSIZE,
        );
        if let Ok(icon) = icon {
            return HICON(icon.0);
        }
        warn!("加载托盘图标失败，回退到默认图标: {}", icon_path.display());
    } else {
        warn!("托盘图标不存在，回退到默认图标: {}", icon_path.display());
    }
    LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_UID,
        ..Default::default()
    };
    let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
}

/// 构建并显示右键弹出菜单，返回用户选择的菜单项 ID（0 表示取消）。
unsafe fn show_tray_menu(hwnd: HWND) -> u16 {
    let mut app = match crate::app_state_mut() {
        Some(app) => app,
        None => return 0,
    };

    let Ok(menu) = CreatePopupMenu() else { return 0; };

    let sing_state = crate::sing_box_state(&app);
    let xray_state = crate::xray_state(&app);
    let exe_dir = app.exe_dir.clone();

    let status_hbmp = |s: ProcessState| match s {
        ProcessState::Running => app.icon_green,
        ProcessState::NotRunning => app.icon_yellow,
        ProcessState::NotInstalled => app.icon_red,
    };
    let status_label = |s: ProcessState, name: &str| match s {
        ProcessState::Running => format!("{name} 正在运行"),
        ProcessState::NotRunning => format!("{name} 未在运行"),
        ProcessState::NotInstalled => format!("{name} 未安装"),
    };

    append_status_item(menu, &status_label(sing_state, "sing-box"), status_hbmp(sing_state));
    append_status_item(menu, &status_label(xray_state, "xray"), status_hbmp(xray_state));
    append_separator(menu);

    let restart_menu = new_submenu();
    let stop_menu = new_submenu();
    let update_menu = new_submenu();
    let sing_menu = new_submenu();
    let xray_menu = new_submenu();

    append_item(restart_menu, ID_RESTART_ALL, "重启 sing-box 和 xray");
    append_item(restart_menu, ID_RESTART_SING, "重启 sing-box");
    append_item(restart_menu, ID_RESTART_XRAY, "重启 xray");

    append_item(stop_menu, ID_STOP_ALL, "终止 sing-box 和 xray");
    append_item(stop_menu, ID_STOP_SING, "终止 sing-box");
    append_item(stop_menu, ID_STOP_XRAY, "终止 xray");

    append_item(update_menu, ID_UPDATE_ALL, "更新 sing-box 和 xray");
    append_item(update_menu, ID_UPDATE_SING, "更新 sing-box");
    append_item(update_menu, ID_UPDATE_XRAY, "更新 xray");

    app.config_actions.clear();
    append_config_items(
        &mut app,
        sing_menu,
        ConfigKind::SingBox,
        ID_SING_CONFIG_BASE,
        &[exe_dir.join("configs").join("sing-box")],
    );
    append_config_items(
        &mut app,
        xray_menu,
        ConfigKind::Xray,
        ID_XRAY_CONFIG_BASE,
        &[exe_dir.join("configs").join("xray")],
    );

    append_submenu(menu, restart_menu, "重新启动");
    append_submenu(menu, stop_menu, "终止运行");
    append_submenu(menu, update_menu, "更新核心");
    append_submenu(menu, sing_menu, "切换 sing-box 配置");
    append_submenu(menu, xray_menu, "切换 xray 配置");
    append_separator(menu);
    append_item(menu, ID_OPEN_DIR, "打开程序目录");
    append_item(menu, ID_EXIT, "退出并终止");

    let mut point = POINT::default();
    let _ = GetCursorPos(&mut point);
    let _ = SetForegroundWindow(hwnd);
    let selected = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
        point.x, point.y,
        Some(0),
        hwnd,
        None,
    );
    // 必须发送 WM_NULL 以正确结束菜单交互，否则点击外部时菜单可能卡住
    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));

    let _ = DestroyMenu(menu);

    selected.0 as u16
}

fn new_submenu() -> HMENU {
    unsafe { CreatePopupMenu().unwrap_or_default() }
}

unsafe fn append_separator(menu: HMENU) {
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
}

unsafe fn append_item(menu: HMENU, id: u16, label: &str) {
    let w = to_wide(label);
    let _ = AppendMenuW(menu, MF_STRING, id as usize, PCWSTR(w.as_ptr()));
}

unsafe fn append_disabled_item(menu: HMENU, label: &str) {
    let w = to_wide(label);
    let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(w.as_ptr()));
}

/// 添加带状态位图的菜单项（仅显示，不可点击）。
/// 先 AppendMenuW 创建文本项，再 SetMenuItemInfoW 设置位图——
/// 这是 Win32 菜单项附加位图的标准做法。
unsafe fn append_status_item(menu: HMENU, label: &str, hbmp: isize) {
    let w = to_wide(label);
    let position = GetMenuItemCount(Some(menu)) as u32;
    let _ = AppendMenuW(menu, MF_STRING, 0, PCWSTR(w.as_ptr()));

    if hbmp == 0 {
        return;
    }

    let mii = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_BITMAP,
        hbmpItem: HBITMAP(hbmp as _),
        ..Default::default()
    };
    let _ = SetMenuItemInfoW(menu, position, true, &mii);
}

unsafe fn append_submenu(menu: HMENU, submenu: HMENU, label: &str) {
    let w = to_wide(label);
    let _ = AppendMenuW(menu, MF_STRING | MF_POPUP, submenu.0 as usize, PCWSTR(w.as_ptr()));
}

/// 扫描配置目录并将每个 .json 文件添加为菜单项，最多 900 项。
unsafe fn append_config_items(
    app: &mut AppState,
    menu: HMENU,
    kind: ConfigKind,
    base_id: u16,
    dirs: &[PathBuf],
) {
    let mut added = 0;

    for (id, path) in (base_id..).zip(crate::find_json_configs(dirs)) {
        if id >= base_id + 900 {
            break;
        }

        let label = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("未命名配置")
            .to_string();
        append_item(menu, id, &label);
        app.config_actions.insert(
            id,
            ConfigAction {
                kind: kind.clone(),
                path,
            },
        );
        added += 1;
    }

    if added == 0 {
        append_disabled_item(menu, "未找到 .json 配置");
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// 将字符串转为 UTF-16 并填充到固定长度缓冲区。
/// `N` 是缓冲区长度（含 null 终止符），超出部分被截断。
/// NOTIFYICONDATAW.szTip 要求 128 个 u16 的固定长度数组。
fn to_wide_padded<const N: usize>(s: &str) -> [u16; N] {
    let mut buf = [0u16; N];
    let wide = to_wide(s);
    let len = wide.len().saturating_sub(1).min(N - 1);
    buf[..len].copy_from_slice(&wide[..len]);
    buf
}
