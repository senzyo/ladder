use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, GetDC, ReleaseDC,
    SelectObject, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
    DefWindowProcW, DestroyIcon, DestroyMenu, DispatchMessageW, DrawIconEx, GetCursorPos,
    GetMenuItemCount, GetMessageW,
    HICON, HMENU, IDI_APPLICATION, IMAGE_ICON, LoadIconW, LoadImageW,
    LR_DEFAULTSIZE, LR_LOADFROMFILE, MB_ICONERROR, MB_OK, MENUITEMINFOW, MIIM_BITMAP,
    MessageBoxW, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, SetMenuItemInfoW, TrackPopupMenu, TranslateMessage,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_DESTROY, WM_LBUTTONUP,
    WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED, DI_NORMAL,
};

use crate::{AppState, ConfigAction, ConfigKind, ProcessState};

pub const WM_TRAY_ICON: u32 = WM_APP + 1;
const TRAY_UID: u32 = 1;

static TRAY_HWND: OnceLock<isize> = OnceLock::new();

pub const ID_RESTART_SING: u16 = 101;
pub const ID_RESTART_XRAY: u16 = 102;
pub const ID_RESTART_ALL: u16 = 103;
pub const ID_STOP_SING: u16 = 201;
pub const ID_STOP_XRAY: u16 = 202;
pub const ID_STOP_ALL: u16 = 203;
pub const ID_UPDATE_ALL: u16 = 301;
pub const ID_UPDATE_SING: u16 = 302;
pub const ID_UPDATE_XRAY: u16 = 303;
pub const ID_EXIT: u16 = 999;
pub const ID_SING_CONFIG_BASE: u16 = 1000;
pub const ID_XRAY_CONFIG_BASE: u16 = 2000;

pub unsafe fn create_window(h_instance: HINSTANCE) -> Result<HWND, String> {
    let class_name = crate::wide("SingBoxWithXrayTrayWindow");

    let wnd_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: h_instance,
        lpszClassName: class_name.as_ptr(),
        ..zeroed()
    };

    if RegisterClassW(&wnd_class) == 0 {
        return Err("注册托盘窗口类失败".to_string());
    }

    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        crate::wide("sing-box-with-xray").as_ptr(),
        WS_OVERLAPPED,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        null_mut(),
        null_mut(),
        h_instance,
        null(),
    );

    if hwnd.is_null() {
        return Err("创建托盘窗口失败".to_string());
    }

    Ok(hwnd)
}

pub unsafe fn add_icon(hwnd: HWND, h_instance: HINSTANCE, work_dir: &Path) -> Result<(), String> {
    let icon = load_app_icon(h_instance, work_dir);
    let mut nid: NOTIFYICONDATAW = zeroed();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_UID;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY_ICON;
    nid.hIcon = icon;
    crate::set_wstr_array(&mut nid.szTip, "sing-box-with-xray");

    if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
        return Err("添加系统托盘图标失败".to_string());
    }

    let _ = TRAY_HWND.set(hwnd as isize);
    Ok(())
}

pub unsafe fn run_message_loop() {
    let mut msg: MSG = zeroed();
    while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

pub fn show_error(hwnd: HWND, title: &str, message: &str) {
    unsafe {
        let title = crate::wide(title);
        let message = crate::wide(message);
        MessageBoxW(hwnd, message.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

pub fn set_tooltip(text: &str) {
    if let Some(&hwnd_val) = TRAY_HWND.get() {
        let hwnd = hwnd_val as HWND;
        unsafe {
            let mut nid: NOTIFYICONDATAW = zeroed();
            nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = TRAY_UID;
            nid.uFlags = NIF_TIP;
            crate::set_wstr_array(&mut nid.szTip, text);
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAY_ICON => {
            let event = lparam as u32;
            if event == WM_LBUTTONUP || event == WM_RBUTTONUP {
                let selected = show_tray_menu(hwnd);
                if selected != 0 {
                    crate::execute_menu_command(hwnd, selected);
                }
            }
            0
        }
        WM_DESTROY => {
            remove_tray_icon(hwnd);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn load_app_icon(h_instance: HINSTANCE, work_dir: &Path) -> HICON {
    let icon_path = work_dir.join("icon").join("ladder.ico");
    if icon_path.exists() {
        let icon_path = crate::wide_path(&icon_path);
        let icon = LoadImageW(
            h_instance,
            icon_path.as_ptr(),
            IMAGE_ICON,
            0,
            0,
            LR_LOADFROMFILE | LR_DEFAULTSIZE,
        );
        if !icon.is_null() {
            return icon as HICON;
        }
    }

    LoadIconW(null_mut(), IDI_APPLICATION)
}

pub(crate) unsafe fn load_icon_bitmap(work_dir: &Path, icon_name: &str) -> isize {
    let icon_path = work_dir.join("icon").join(icon_name);
    if !icon_path.exists() {
        return 0;
    }
    let icon_path = crate::wide_path(&icon_path);
    let hicon = LoadImageW(
        null_mut(),
        icon_path.as_ptr(),
        IMAGE_ICON,
        16,
        16,
        LR_LOADFROMFILE,
    );
    if hicon.is_null() {
        return 0;
    }

    let screen_dc = GetDC(null_mut());
    if screen_dc.is_null() {
        DestroyIcon(hicon as HICON);
        return 0;
    }
    let mem_dc = CreateCompatibleDC(screen_dc);
    if mem_dc.is_null() {
        ReleaseDC(null_mut(), screen_dc);
        DestroyIcon(hicon as HICON);
        return 0;
    }

    let mut bmi: BITMAPINFO = zeroed();
    bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = 16;
    bmi.bmiHeader.biHeight = 16;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;

    let mut pv_bits: *mut c_void = null_mut();
    let bmp = CreateDIBSection(
        mem_dc,
        &bmi,
        DIB_RGB_COLORS,
        &mut pv_bits,
        0 as _,
        0,
    );

    ReleaseDC(null_mut(), screen_dc);

    if bmp.is_null() {
        DeleteDC(mem_dc);
        DestroyIcon(hicon as HICON);
        return 0;
    }

    let old_bmp = SelectObject(mem_dc, bmp);
    DrawIconEx(mem_dc, 0, 0, hicon as HICON, 16, 16, 0, null_mut(), DI_NORMAL);
    SelectObject(mem_dc, old_bmp);
    DeleteDC(mem_dc);
    DestroyIcon(hicon as HICON);

    bmp as isize
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = zeroed();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_UID;
    Shell_NotifyIconW(NIM_DELETE, &nid);
}

unsafe fn show_tray_menu(hwnd: HWND) -> u16 {
    let mut app = match crate::app_state_mut() {
        Some(app) => app,
        None => return 0,
    };

    let menu = CreatePopupMenu();

    let sing_state = crate::sing_box_state(&mut app);
    let xray_state = crate::xray_state(&mut app);
    let work_dir = app.work_dir.clone();

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
    AppendMenuW(menu, MF_SEPARATOR, 0, null());

    let restart_menu = CreatePopupMenu();
    let stop_menu = CreatePopupMenu();
    let update_menu = CreatePopupMenu();
    let sing_menu = CreatePopupMenu();
    let xray_menu = CreatePopupMenu();

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
        &[work_dir.join("configs").join("sing-box")],
    );
    append_config_items(
        &mut app,
        xray_menu,
        ConfigKind::Xray,
        ID_XRAY_CONFIG_BASE,
        &[work_dir.join("configs").join("xray")],
    );

    append_submenu(menu, restart_menu, "重新启动");
    append_submenu(menu, stop_menu, "终止运行");
    append_submenu(menu, update_menu, "更新核心");
    append_submenu(menu, sing_menu, "切换 sing-box 配置");
    append_submenu(menu, xray_menu, "切换 xray 配置");
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    append_item(menu, ID_EXIT, "退出并终止");

    let mut point = POINT { x: 0, y: 0 };
    GetCursorPos(&mut point);
    SetForegroundWindow(hwnd);
    let selected = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
        point.x,
        point.y,
        0,
        hwnd,
        null(),
    );

    DestroyMenu(menu);

    selected as u16
}

unsafe fn append_item(menu: HMENU, id: u16, label: &str) {
    let label = crate::wide(label);
    AppendMenuW(menu, MF_STRING, id as usize, label.as_ptr());
}

unsafe fn append_disabled_item(menu: HMENU, label: &str) {
    let label = crate::wide(label);
    AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, label.as_ptr());
}

unsafe fn append_status_item(
    menu: HMENU,
    label: &str,
    hbmp: isize,
) {
    let label = crate::wide(label);
    let position = GetMenuItemCount(menu) as u32;
    AppendMenuW(menu, MF_STRING, 0, label.as_ptr());

    if hbmp == 0 {
        return;
    }

    let mut mii: MENUITEMINFOW = zeroed();
    mii.cbSize = size_of::<MENUITEMINFOW>() as u32;
    mii.fMask = MIIM_BITMAP;
    mii.hbmpItem = hbmp as _;
    SetMenuItemInfoW(menu, position, 1, &mii);
}

unsafe fn append_submenu(menu: HMENU, submenu: HMENU, label: &str) {
    let label = crate::wide(label);
    AppendMenuW(menu, MF_STRING | MF_POPUP, submenu as usize, label.as_ptr());
}

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
