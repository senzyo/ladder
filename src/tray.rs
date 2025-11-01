//! Windows 系统托盘 UI。
//!
//! 负责创建隐藏窗口、注册托盘图标、构建右键弹出菜单 (重启/终止/更新/切换配置/退出) ,
//! 处理鼠标消息并将菜单事件分发给 `commands::execute_menu_command`。

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::OnceLock;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDC, HBITMAP, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW,
    DI_NORMAL, DefWindowProcW, DestroyIcon, DestroyMenu, DispatchMessageW, DrawIconEx,
    GetCursorPos, GetMenuItemCount, GetMessageW, HICON, HMENU, IDI_APPLICATION, IMAGE_ICON,
    LR_DEFAULTSIZE, LR_LOADFROMFILE, LoadIconW, LoadImageW, MB_ICONERROR, MB_ICONWARNING, MB_OK,
    MENUITEMINFOW, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MIIM_BITMAP, MSG, MessageBoxW,
    PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow, SetMenuItemInfoW,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP,
    WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR};

use crate::app_error::AppError;
use crate::app_state;
use crate::commands;
use crate::config_selection::{self, ConfigSelection};
use crate::core::{Core, CoreStatus};
use crate::process;
use crate::settings::CoreMode;
use crate::windows_strings;
use tracing::warn;

/// 三个状态图标的 GDI 位图句柄, 0 表示加载失败。
#[must_use = "状态图标应保存到应用状态, 并在退出时显式释放"]
#[derive(Clone, Copy, Default)]
pub struct StatusIcons {
    pub running: isize,
    pub not_running: isize,
    pub not_installed: isize,
}

impl StatusIcons {
    /// 取进程状态对应的位图句柄。
    #[must_use]
    pub fn handle_for(self, state: CoreStatus) -> isize {
        match state {
            CoreStatus::Running => self.running,
            CoreStatus::NotRunning => self.not_running,
            CoreStatus::NotInstalled => self.not_installed,
        }
    }

    /// 释放三个位图句柄。
    ///
    /// 刻意做成显式方法而不是 `Drop`: `AppState` 放在 `OnceLock` 里, 静态变量
    /// 的 Drop 永远不会执行, 写成 Drop 就成了不会运行的死代码。
    ///
    /// # Safety
    ///
    /// 非零句柄必须互不相同, 仍为本组拥有的有效位图, 且未被其他副本释放。
    /// 调用前必须关闭使用这些位图的菜单, 此后不得再使用本组的任何副本。
    pub unsafe fn delete(self) {
        for handle in [self.running, self.not_running, self.not_installed] {
            if handle != 0 {
                // SAFETY: 调用方保证位图仍有效、已不再被菜单使用, 且只释放一次。
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(handle as *mut c_void));
                }
            }
        }
    }
}

/// 托盘图标的自定义消息 ID, 当托盘收到鼠标事件时通过此消息通知窗口。
pub const WM_TRAY_ICON: u32 = WM_APP + 1;
const TRAY_UID: u32 = 1;

static TRAY_HWND: OnceLock<isize> = OnceLock::new();

/// 显示菜单前取得的状态快照, 不持有应用状态锁。
struct MenuSnapshot {
    exe_dir: PathBuf,
    icons: StatusIcons,
    core_mode: CoreMode,
}

/// 菜单关闭后的选择及对应的候选配置, ID 为 0 表示没有选择命令。
#[must_use]
#[derive(Default)]
struct MenuSelection {
    command_id: u16,
    config_selections: HashMap<u16, ConfigSelection>,
}

/// 单核菜单使用的核心身份及命令 ID。
struct CoreMenuItems {
    core: Core,
    restart_id: u16,
    stop_id: u16,
    update_id: u16,
    config_base: u16,
}

enum MenuItemState {
    Enabled,
    Disabled,
}

// 菜单 ID 分段规划: 101-199 重启, 201-299 终止, 301-399 更新, 999 退出,
// 1000-1999 sing-box 配置, 2000-2999 xray 配置。
pub const ID_RESTART_SING: u16 = 101;
pub const ID_RESTART_XRAY: u16 = 102;
pub const ID_RESTART_ALL: u16 = 103;
pub const ID_STOP_SING: u16 = 201;
pub const ID_STOP_XRAY: u16 = 202;
pub const ID_STOP_ALL: u16 = 203;
pub const ID_UPDATE_ALL: u16 = 301;
pub const ID_UPDATE_SING: u16 = 302;
pub const ID_UPDATE_XRAY: u16 = 303;
pub const ID_SWITCH_CORE_XRAY: u16 = 401;
pub const ID_SWITCH_CORE_SING: u16 = 402;
pub const ID_SWITCH_CORE_BOTH: u16 = 403;
pub const ID_OPEN_DIR: u16 = 998;
pub const ID_EXIT: u16 = 999;
pub const ID_SING_CONFIG_BASE: u16 = 1000;
pub const ID_XRAY_CONFIG_BASE: u16 = 2000;

/// 创建托盘图标所依附的隐藏窗口。返回窗口句柄。
///
/// # Safety
///
/// `h_instance` 必须是当前进程仍加载的模块句柄, 在窗口类及其窗口使用期间保持有效。
/// 调用方须在当前线程运行消息循环, 并在此线程销毁窗口。
pub unsafe fn create_window(h_instance: isize) -> Result<isize, AppError> {
    let h_instance = HINSTANCE(h_instance as _);
    let class_name = windows_strings::wide("LadderTrayWindow");

    let wnd_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: h_instance,
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };

    // SAFETY: 模块有效性由调用方保证; 类名以 NUL 结尾且覆盖调用期,
    // 窗口过程具有系统 ABI 和静态代码地址, RegisterClassW 复制类信息。
    if unsafe { RegisterClassW(&wnd_class) } == 0 {
        return Err(AppError::Msg("注册托盘窗口类失败".into()));
    }

    let title = windows_strings::wide("ladder");
    // SAFETY: 类已注册, 模块仍有效, 两个 UTF-16 缓冲区覆盖同步创建调用,
    // 无父窗口、菜单或用户数据指针; 系统以约定 ABI 调用 wnd_proc。
    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            None,
            None,
            Some(h_instance),
            None,
        )
    };

    let Ok(hwnd) = hwnd else {
        return Err(AppError::Msg("创建托盘窗口失败".into()));
    };

    Ok(hwnd.0 as isize)
}

/// 向系统托盘添加图标并注册鼠标事件回调。
///
/// # Safety
///
/// `hwnd` 必须是 `create_window` 返回且尚未销毁的当前线程窗口;
/// `h_instance` 必须是创建该窗口时使用的仍有效模块句柄。
pub unsafe fn add_icon(hwnd: isize, h_instance: isize, exe_dir: &Path) -> Result<(), AppError> {
    let hwnd = HWND(hwnd as _);
    let h_instance = HINSTANCE(h_instance as _);
    // SAFETY: 调用方保证模块仍加载; 图标路径缓冲区由 load_app_icon 管理。
    let icon = unsafe { load_app_icon(Some(h_instance), exe_dir) };
    let nid = NOTIFYICONDATAW {
        cbSize: mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_UID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY_ICON,
        hIcon: icon,
        szTip: to_wide_padded("ladder"),
        ..Default::default()
    };

    // SAFETY: 窗口有效性由调用方保证, nid 大小和标志匹配其初始化字段;
    // 图标仍有效或为加载失败的空句柄, Shell 在调用内读取并复制图标数据。
    if !unsafe { Shell_NotifyIconW(NIM_ADD, &nid) }.as_bool() {
        return Err(AppError::Msg("添加系统托盘图标失败".into()));
    }

    let _ = TRAY_HWND.set(hwnd.0 as isize);
    Ok(())
}

/// 运行 Windows 消息循环, 阻塞直到窗口被销毁。
///
/// # Safety
///
/// 必须在创建托盘窗口的线程调用; 窗口过程及其使用的状态、位图在分发期间必须有效。
/// 消息中的指针/回调须遵循对应 Win32 消息约定, 不能向此线程投递伪造的指针消息。
pub unsafe fn run_message_loop() {
    let mut msg: MSG = Default::default();
    // SAFETY: msg 是初始化且可写的完整结构, 当前线程与消息来源遵守调用方契约。
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        // SAFETY: 正常取得消息时, msg 来自当前线程队列, 回调及载荷遵守调用方契约。
        // 保留原有 as_bool 判定; GetMessageW == -1 时不具备上述来源保证,
        // 该错误路径尚未验证, 属于另行处理的既有缺陷, 此处不宣称已证明安全。
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// 显示错误消息对话框。
pub fn show_error(hwnd: isize, title: &str, message: &str) {
    let hwnd = if hwnd == 0 {
        None
    } else {
        Some(HWND(hwnd as _))
    };
    let message = HSTRING::from(message);
    let title = HSTRING::from(title);
    // SAFETY: 两个字符串在同步调用期间有效; hwnd 仅为系统校验的可选所有者句柄,
    // 本函数不将其解释为 Rust 指针, 也不传递回调或用户数据。
    let _ = unsafe { MessageBoxW(hwnd, &message, &title, MB_OK | MB_ICONERROR) };
}

/// 显示警告消息对话框。
pub fn show_warn(hwnd: isize, title: &str, message: &str) {
    let hwnd = if hwnd == 0 {
        None
    } else {
        Some(HWND(hwnd as _))
    };
    let message = HSTRING::from(message);
    let title = HSTRING::from(title);
    // SAFETY: 字符串覆盖同步调用期, 可选窗口句柄仅由系统校验;
    // 不传递 Rust 回调或用户数据指针。
    let _ = unsafe { MessageBoxW(hwnd, &message, &title, MB_OK | MB_ICONWARNING) };
}

/// 更新托盘图标的悬停提示文本。
pub fn set_tooltip(text: &str) {
    if let Some(&hwnd_val) = TRAY_HWND.get() {
        let hwnd = HWND(hwnd_val as _);
        let nid = NOTIFYICONDATAW {
            cbSize: mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_UID,
            uFlags: NIF_TIP,
            szTip: to_wide_padded(text),
            ..Default::default()
        };
        // SAFETY: nid 大小正确, NIF_TIP 只读取内联且 NUL 结尾的文本;
        // 窗口 ID 仅用于查找托盘项, 退出竞态下无效句柄由 Shell 报错。
        let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
    }
}

/// 加载托盘菜单使用的三个状态图标。
pub fn load_status_icons(exe_dir: &Path) -> StatusIcons {
    StatusIcons {
        running: load_icon_bitmap(exe_dir, "green_circle.ico"),
        not_running: load_icon_bitmap(exe_dir, "yellow_circle.ico"),
        not_installed: load_icon_bitmap(exe_dir, "red_circle.ico"),
    }
}

/// 从 ICO 文件加载图标并转换为 32 位 DIB 位图句柄。
///
/// 流程: LoadImageW 加载 ICO → 创建兼容 DC → 创建 DIB Section →
/// DrawIconEx 绘制到位图 → 清理中间资源 → 返回位图句柄。
/// 菜单项需要位图句柄 (而非图标句柄) 来显示状态图标。
#[must_use = "加载成功的位图需要保存并显式释放"]
pub(crate) fn load_icon_bitmap(exe_dir: &Path, icon_name: &str) -> isize {
    let icon_path = exe_dir.join("icons").join(icon_name);
    if !icon_path.exists() {
        warn!("图标文件不存在: {}", icon_path.display());
        return 0;
    }
    let icon_path_w: Vec<u16> = icon_path
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: 路径是覆盖调用期的 NUL 结尾 UTF-16 缓冲区, LR_LOADFROMFILE
    // 让系统按文件加载; 未指定 LR_SHARED, 成功句柄由本函数独占释放。
    let hicon = unsafe {
        LoadImageW(
            None,
            PCWSTR(icon_path_w.as_ptr()),
            IMAGE_ICON,
            16,
            16,
            LR_LOADFROMFILE | LR_DEFAULTSIZE,
        )
    };
    let Ok(hicon) = hicon else {
        warn!("加载图标失败: {}", icon_path.display());
        return 0;
    };
    let hicon = HICON(hicon.0);

    // SAFETY: None 请求当前线程使用的屏幕 DC, 无输入指针。
    let screen_dc = unsafe { GetDC(None) };
    if screen_dc.is_invalid() {
        // SAFETY: hicon 是本函数成功加载且尚未释放的非共享图标。
        let _ = unsafe { DestroyIcon(hicon) };
        return 0;
    }
    // SAFETY: screen_dc 已检查有效且仍由当前线程持有。
    let mem_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
    if mem_dc.is_invalid() {
        // SAFETY: DC 和非共享图标分别由本函数取得, 未移交或释放。
        unsafe {
            let _ = ReleaseDC(None, screen_dc);
            let _ = DestroyIcon(hicon);
        }
        return 0;
    }

    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: 16,
            biHeight: 16,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut pv_bits: *mut c_void = ptr::null_mut();
    // SAFETY: DC 有效, bmi 是完整初始化的 32 位 BI_RGB 描述;
    // pv_bits 是可写指针槽, 未传文件映射, 返回的像素指针不会在 Rust 中解引用。
    let bitmap =
        unsafe { CreateDIBSection(Some(screen_dc), &bmi, DIB_RGB_COLORS, &mut pv_bits, None, 0) };
    let Ok(bmp) = bitmap else {
        // SAFETY: mem_dc 和 hicon 由本函数独占且仍有效。
        // 保留此失败分支遗漏 ReleaseDC 的既有缺陷, 不在安全边界整理中修复。
        unsafe {
            let _ = DeleteDC(mem_dc);
            let _ = DestroyIcon(hicon);
        }
        return 0;
    };

    // SAFETY: 位图、内存 DC 和图标均由本函数创建且仍有效; 只在本线程绘制,
    // 随后先恢复旧对象再释放 DC/图标, 返回的位图交由调用方在菜单关闭后释放。
    unsafe {
        let old_bmp = SelectObject(mem_dc, HGDIOBJ(bmp.0));
        let _ = DrawIconEx(mem_dc, 0, 0, hicon, 16, 16, 0, None, DI_NORMAL);
        SelectObject(mem_dc, old_bmp);
        let _ = ReleaseDC(None, screen_dc);
        let _ = DeleteDC(mem_dc);
        let _ = DestroyIcon(hicon);
    }
    bmp.0 as isize
}

/// 窗口过程: 处理托盘图标鼠标事件和窗口销毁。
///
/// # Safety
///
/// 仅作为注册给 Win32 的窗口过程调用; 参数须遵守窗口消息 ABI,
/// `hwnd` 属于当前线程, 消息指针载荷在对应处理期间有效。
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
                // SAFETY: hwnd 来自系统窗口回调且属于当前线程; AppState 中的位图
                // 直到消息循环退出才释放, 菜单关闭前不会由本回调销毁窗口。
                let selection = unsafe { show_tray_menu(hwnd) };
                if selection.command_id != 0 {
                    // SAFETY: 窗口属于当前回调线程, 菜单已关闭; 退出命令在此线程
                    // 完成唯一一次窗口销毁, 其他命令按原有后台调度执行。
                    unsafe {
                        commands::execute_menu_command(
                            hwnd.0 as isize,
                            selection.command_id,
                            &selection.config_selections,
                        );
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: hwnd 是系统正在销毁的本模块窗口, 仅请求删除既定托盘标识。
            unsafe { remove_tray_icon(hwnd) };
            // SAFETY: 仅向当前线程消息队列提交退出码, 无指针参数。
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: 原样转发系统交给本窗口过程的参数, 不改写消息的指针载荷。
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// # Safety
///
/// 若提供模块句柄, 它必须在加载调用期间保持有效。
#[must_use]
unsafe fn load_app_icon(h_instance: Option<HINSTANCE>, exe_dir: &Path) -> HICON {
    let icon_path = exe_dir.join("icons").join("ladder.ico");
    if icon_path.exists() {
        let icon_path_w: Vec<u16> = icon_path
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: 模块由调用方保证有效, 路径缓冲区以 NUL 结尾并覆盖调用期;
        // LR_LOADFROMFILE 指示按文件加载, 保留原有自有图标释放规则。
        let icon = unsafe {
            LoadImageW(
                h_instance,
                PCWSTR(icon_path_w.as_ptr()),
                IMAGE_ICON,
                0,
                0,
                LR_LOADFROMFILE | LR_DEFAULTSIZE,
            )
        };
        if let Ok(icon) = icon {
            return HICON(icon.0);
        }
        warn!("加载托盘图标失败, 回退到默认图标: {}", icon_path.display());
    } else {
        warn!("托盘图标不存在, 回退到默认图标: {}", icon_path.display());
    }
    // SAFETY: None 配合系统预定义资源 ID 请求共享图标, 不传入字符串指针。
    unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default()
}

/// # Safety
///
/// `hwnd` 必须是本模块创建的当前线程窗口, 可处于 WM_DESTROY 处理期间。
/// 不要求托盘项已成功添加, 不存在的标识由 Shell 返回失败。
unsafe fn remove_tray_icon(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_UID,
        ..Default::default()
    };
    // SAFETY: nid 完整初始化且大小正确, 窗口/托盘标识由调用方保证,
    // 删除操作仅在调用期间借用 nid。
    let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
}

/// 构建并显示右键弹出菜单, 返回用户选择的菜单项 ID 和配置项映射。
///
/// 菜单根据当前核心模式 (xray / sing-box / both) 动态构建。
/// Mutex 仅在读取状态时持有, TrackPopupMenu 在锁外执行,
/// 避免模态消息循环重入窗口过程时因 Mutex 不可重入导致死锁。
///
/// # Safety
///
/// `hwnd` 必须属于当前线程且在菜单关闭前有效; 应用状态中的非零位图句柄
/// 必须仍有效, 且只能在菜单及消息循环结束后释放。
unsafe fn show_tray_menu(hwnd: HWND) -> MenuSelection {
    let Some(MenuSnapshot {
        exe_dir,
        icons,
        core_mode,
    }) = app_state::with_app_state(|app| MenuSnapshot {
        exe_dir: app.exe_dir.clone(),
        icons: app.icons,
        core_mode: app.settings.core.mode,
    })
    else {
        return MenuSelection::default();
    };

    // 锁已释放才做状态查询: 进程快照和文件存在性检查都是几十毫秒级的
    // 系统调用, 放在锁内会阻塞需要写状态的后台线程。
    let states = process::core_states(&exe_dir);

    // SAFETY: 无输入指针, 成功菜单由本函数独占, 在显示结束后统一销毁。
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
        return MenuSelection::default();
    };

    let status_label = |s: CoreStatus, name: &str| match s {
        CoreStatus::Running => format!("{name} 正在运行"),
        CoreStatus::NotRunning => format!("{name} 未在运行"),
        CoreStatus::NotInstalled => format!("{name} 未安装"),
    };

    // ── 状态项: 仅显示已启用核心 ──
    if core_mode.runs_sing_box() {
        // SAFETY: 菜单有效, 状态位图由调用方保证在菜单销毁前保持有效。
        unsafe {
            append_status_item(
                menu,
                &status_label(states.sing_box, Core::SingBox.label()),
                icons.handle_for(states.sing_box),
            );
        }
    }
    if core_mode.runs_xray() {
        // SAFETY: 菜单有效, 状态位图由调用方保证在菜单销毁前保持有效。
        unsafe {
            append_status_item(
                menu,
                &status_label(states.xray, Core::Xray.label()),
                icons.handle_for(states.xray),
            );
        }
    }
    // SAFETY: 菜单由本函数创建且尚未销毁, 空子菜单仍交由 Win32 报错。
    unsafe {
        append_separator(menu);
    }

    // ── 操作项 ──
    let mut config_selections = HashMap::new();

    if core_mode == CoreMode::Both {
        // 双核模式: 重新启动/终止运行/更新核心 均为子菜单
        let restart_menu = new_submenu();
        let stop_menu = new_submenu();
        let update_menu = new_submenu();

        // SAFETY: 菜单由本函数创建且尚未销毁, 空子菜单仍交由 Win32 报错。
        unsafe {
            append_item(restart_menu, ID_RESTART_SING, "重启 sing-box");
            append_item(restart_menu, ID_RESTART_XRAY, "重启 xray");
            append_item(restart_menu, ID_RESTART_ALL, "重启 sing-box 和 xray");
            append_item(stop_menu, ID_STOP_SING, "终止 sing-box");
            append_item(stop_menu, ID_STOP_XRAY, "终止 xray");
            append_item(stop_menu, ID_STOP_ALL, "终止 sing-box 和 xray");
            append_item(update_menu, ID_UPDATE_SING, "更新 sing-box");
            append_item(update_menu, ID_UPDATE_XRAY, "更新 xray");
            append_item(update_menu, ID_UPDATE_ALL, "更新 sing-box 和 xray");
        }

        let sing_menu = new_submenu();
        let xray_menu = new_submenu();
        // SAFETY: 菜单由本函数创建且尚未销毁, 空子菜单仍交由 Win32 报错。
        unsafe {
            append_config_items(
                &mut config_selections,
                sing_menu,
                Core::SingBox,
                ID_SING_CONFIG_BASE,
                &[Core::SingBox.config_dir(&exe_dir)],
            );
            append_config_items(
                &mut config_selections,
                xray_menu,
                Core::Xray,
                ID_XRAY_CONFIG_BASE,
                &[Core::Xray.config_dir(&exe_dir)],
            );
        }

        // SAFETY: 根菜单及子菜单尚未销毁, 附加成功后随根菜单统一释放。
        unsafe {
            append_submenu(menu, restart_menu, "重新启动");
            append_submenu(menu, stop_menu, "终止运行");
            append_submenu(menu, update_menu, "更新核心");
            append_submenu(menu, sing_menu, "切换 sing-box 配置");
            append_submenu(menu, xray_menu, "切换 xray 配置");
        }
    } else {
        // 单核模式: 操作项为直接可点击的一级菜单
        let items = if core_mode.runs_xray() {
            CoreMenuItems {
                core: Core::Xray,
                restart_id: ID_RESTART_XRAY,
                stop_id: ID_STOP_XRAY,
                update_id: ID_UPDATE_XRAY,
                config_base: ID_XRAY_CONFIG_BASE,
            }
        } else {
            CoreMenuItems {
                core: Core::SingBox,
                restart_id: ID_RESTART_SING,
                stop_id: ID_STOP_SING,
                update_id: ID_UPDATE_SING,
                config_base: ID_SING_CONFIG_BASE,
            }
        };

        // SAFETY: 菜单由本函数创建且尚未销毁, 空子菜单仍交由 Win32 报错。
        unsafe {
            append_item(menu, items.restart_id, "重新启动");
            append_item(menu, items.stop_id, "终止运行");
            append_item(menu, items.update_id, "更新核心");
        }

        let config_menu = new_submenu();
        // SAFETY: 菜单由本函数创建且尚未销毁, 空子菜单仍交由 Win32 报错。
        unsafe {
            append_config_items(
                &mut config_selections,
                config_menu,
                items.core,
                items.config_base,
                &[items.core.config_dir(&exe_dir)],
            );
        }
        // SAFETY: 根菜单及子菜单尚未销毁, 附加成功后随根菜单统一释放。
        unsafe {
            append_submenu(menu, config_menu, "切换配置");
        }
    }

    // ── 切换核心子菜单 (始终显示)  ──
    // SAFETY: 菜单由本函数创建且尚未销毁, 空子菜单仍交由 Win32 报错。
    unsafe {
        append_separator(menu);
    }
    let switch_menu = new_submenu();
    // SAFETY: 菜单由本函数创建且尚未销毁, 空子菜单仍交由 Win32 报错。
    unsafe {
        append_item_or_disabled(
            switch_menu,
            ID_SWITCH_CORE_XRAY,
            "仅用 xray",
            if core_mode == CoreMode::Xray {
                MenuItemState::Disabled
            } else {
                MenuItemState::Enabled
            },
        );
        append_item_or_disabled(
            switch_menu,
            ID_SWITCH_CORE_SING,
            "仅用 sing-box",
            if core_mode == CoreMode::SingBox {
                MenuItemState::Disabled
            } else {
                MenuItemState::Enabled
            },
        );
        append_item_or_disabled(
            switch_menu,
            ID_SWITCH_CORE_BOTH,
            "全部启用",
            if core_mode == CoreMode::Both {
                MenuItemState::Disabled
            } else {
                MenuItemState::Enabled
            },
        );
    }
    // SAFETY: 根菜单及子菜单尚未销毁, 附加成功后随根菜单统一释放。
    unsafe {
        append_submenu(menu, switch_menu, "切换核心");
    }
    // SAFETY: 菜单由本函数创建且尚未销毁, 空子菜单仍交由 Win32 报错。
    unsafe {
        append_item(menu, ID_OPEN_DIR, "打开程序目录");
        append_separator(menu);
        append_item(menu, ID_EXIT, "退出并终止");
    }

    let mut point = POINT::default();
    // SAFETY: point 是初始化且可写的结构, 不向系统交出所有权。
    let _ = unsafe { GetCursorPos(&mut point) };
    // SAFETY: 窗口由当前线程持有并保持有效, 由调用方契约保证。
    let _ = unsafe { SetForegroundWindow(hwnd) };
    // SAFETY: menu 及位图在模态调用期间有效, hwnd 属于当前线程;
    // 无排除矩形指针, 应用状态锁已释放, 可允许窗口过程重入。
    let selected = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            Some(0),
            hwnd,
            None,
        )
    };
    // SAFETY: WM_NULL 没有指针载荷, hwnd 在当前函数返回前保持有效。
    let _ = unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) };

    // SAFETY: 模态菜单已关闭, 本函数独占根菜单, 连同已附加子菜单仅销毁一次。
    let _ = unsafe { DestroyMenu(menu) };

    MenuSelection {
        command_id: selected.0 as u16,
        config_selections,
    }
}

#[must_use]
fn new_submenu() -> HMENU {
    // SAFETY: 无输入参数; 返回新建的菜单句柄, 创建失败沿用空句柄降级。
    unsafe { CreatePopupMenu().unwrap_or_default() }
}

/// # Safety
///
/// `menu` 必须是当前线程构建且未销毁的菜单, 或创建失败的空句柄。
unsafe fn append_separator(menu: HMENU) {
    // SAFETY: 菜单借用由调用方保证; MF_SEPARATOR 不读取文本或用户数据。
    let _ = unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, None) };
}

/// # Safety
///
/// `menu` 必须是当前线程构建且未销毁的菜单, 或创建失败的空句柄。
unsafe fn append_item(menu: HMENU, id: u16, label: &str) {
    let w = windows_strings::wide(label);
    // SAFETY: 菜单借用由调用方保证; NUL 结尾的 w 在同步复制文本期间有效,
    // MF_STRING 将 id 解释为命令 ID, 不作为指针解引用。
    let _ = unsafe { AppendMenuW(menu, MF_STRING, id as usize, PCWSTR(w.as_ptr())) };
}

/// # Safety
///
/// `menu` 必须是当前线程构建且未销毁的菜单, 或创建失败的空句柄。
unsafe fn append_disabled_item(menu: HMENU, label: &str) {
    let w = windows_strings::wide(label);
    // SAFETY: 菜单借用由调用方保证; w 以 NUL 结尾, 系统在调用中复制文本。
    let _ = unsafe { AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(w.as_ptr())) };
}

/// 添加菜单项, 禁用时灰显且不分配命令 ID。
///
/// # Safety
///
/// `menu` 必须是当前线程构建且未销毁的菜单, 或创建失败的空句柄。
unsafe fn append_item_or_disabled(menu: HMENU, id: u16, label: &str, state: MenuItemState) {
    let w = windows_strings::wide(label);
    let flags = match state {
        MenuItemState::Enabled => MF_STRING,
        MenuItemState::Disabled => MF_STRING | MF_GRAYED,
    };
    let item_id = match state {
        MenuItemState::Enabled => id as usize,
        MenuItemState::Disabled => 0,
    };
    // SAFETY: 菜单借用由调用方保证; flags 始终指定字符串, w 是覆盖同步复制期
    // 的 NUL 结尾缓冲区, item_id 仅作为数值命令 ID。
    let _ = unsafe { AppendMenuW(menu, flags, item_id, PCWSTR(w.as_ptr())) };
}

/// 添加带状态位图的菜单项 (仅显示, 不可点击) 。
/// 先 AppendMenuW 创建文本项, 再 SetMenuItemInfoW 设置位图——
/// 这是 Win32 菜单项附加位图的标准做法。
///
/// # Safety
///
/// `menu` 必须是当前线程正在构建的有效菜单; 非零 `hbmp` 必须是有效位图,
/// 在菜单销毁前不得释放或改为其他用途。
unsafe fn append_status_item(menu: HMENU, label: &str, hbmp: isize) {
    let w = windows_strings::wide(label);
    // SAFETY: 调用方保证菜单有效, 未在其他线程并发销毁。
    let position = unsafe { GetMenuItemCount(Some(menu)) } as u32;
    // SAFETY: 菜单有效, w 在同步复制文本期间保持 NUL 结尾和有效存储。
    let _ = unsafe { AppendMenuW(menu, MF_STRING, 0, PCWSTR(w.as_ptr())) };

    if hbmp == 0 {
        return;
    }

    let mii = MENUITEMINFOW {
        cbSize: mem::size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_BITMAP,
        hbmpItem: HBITMAP(hbmp as _),
        ..Default::default()
    };
    // SAFETY: mii 大小与 MIIM_BITMAP 字段一致, 位图在菜单使用期有效由调用方保证;
    // 系统仅在调用内借用结构, 保留的位图句柄不会指向 Rust 栈存储。
    let _ = unsafe { SetMenuItemInfoW(menu, position, true, &mii) };
}

/// # Safety
///
/// 两个菜单必须由当前线程创建且尚未销毁, 或为创建失败的空句柄。
/// 非空子菜单尚未附加到其他菜单; 附加成功后不得独立销毁, 由根菜单统一释放。
unsafe fn append_submenu(menu: HMENU, submenu: HMENU, label: &str) {
    let w = windows_strings::wide(label);
    // SAFETY: 调用方保证菜单的所有权关系; MF_POPUP 使第三参数作为子菜单句柄,
    // w 仅在系统同步复制菜单文本期间借用。
    let _ = unsafe {
        AppendMenuW(
            menu,
            MF_STRING | MF_POPUP,
            submenu.0 as usize,
            PCWSTR(w.as_ptr()),
        )
    };
}

/// 扫描配置目录并将每个 .json 文件添加为菜单项, 最多 900 项。
///
/// # Safety
///
/// `menu` 必须是当前线程构建且未销毁的菜单, 或创建失败的空句柄。
unsafe fn append_config_items(
    map: &mut HashMap<u16, ConfigSelection>,
    menu: HMENU,
    core: Core,
    base_id: u16,
    dirs: &[PathBuf],
) {
    let mut added = 0;

    let config_items = (base_id..).zip(config_selection::find_json_configs(dirs));
    for (id, path) in config_items {
        if id >= base_id + 900 {
            break;
        }

        let label = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("未命名配置")
            .to_string();
        // SAFETY: 扫描不改变 menu 的所有权, 调用方保证其在本函数期间可用。
        unsafe { append_item(menu, id, &label) };
        map.insert(id, ConfigSelection { core, path });
        added += 1;
    }

    if added == 0 {
        // SAFETY: 同一菜单仍在调用方保证的构建/借用期内。
        unsafe { append_disabled_item(menu, "未找到 .json 配置") };
    }
}

/// 将字符串转为 UTF-16 并填充到固定长度缓冲区。
/// `N` 是缓冲区长度 (含 null 终止符) , 超出部分被截断。
/// NOTIFYICONDATAW.szTip 要求 128 个 u16 的固定长度数组。
/// 调用方须保证 `N > 0`, 为 null 终止符保留空间。
#[must_use]
fn to_wide_padded<const N: usize>(s: &str) -> [u16; N] {
    let mut buf = [0u16; N];
    let wide: Vec<u16> = s.encode_utf16().collect();
    let len = wide.len().min(N - 1);
    buf[..len].copy_from_slice(&wide[..len]);
    buf
}
