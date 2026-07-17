//! Windows Toast 通知。
//!
//! 通过 Windows COM ToastNotification API 弹出系统通知。
//! 首次运行时会在开始菜单创建快捷方式并设置 AUMID (AppUserModelID) ,
//! 这是 Windows 识别应用身份以显示 Toast 通知的必要条件。

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::OnceLock;
use tracing::error;
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
use windows::Win32::UI::Shell::{IShellLinkW, SetCurrentProcessExplicitAppUserModelID, ShellLink};
use windows::core::{HSTRING, Interface};

use crate::error::AppError;

const AUMID: &str = "Ladder";
const SHORTCUT_NAME: &str = "ladder";

static ICON_URI: OnceLock<String> = OnceLock::new();

// ═══════════════════════════════════════════════
// Public API
// ═══════════════════════════════════════════════

/// 初始化 Toast 通知系统。
///
/// 设置当前进程的 AUMID, 并在开始菜单 Programs 目录创建快捷方式。
/// Windows 要求应用必须有带 AUMID 的快捷方式才能显示 Toast 通知。
pub fn setup(exe_path: &std::path::Path) -> Result<(), AppError> {
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID))
            .map_err(|e| AppError::Msg(format!("设置 AUMID 失败: {e}")))?;
    }
    let icon_path = exe_path.parent().unwrap_or(exe_path).join("icons").join("ladder.ico");
    let _ = ICON_URI.set(format!("file:///{}", icon_path.to_string_lossy().replace('\\', "/")));
    ensure_shortcut(exe_path, &icon_path)?;
    Ok(())
}

/// 显示 Toast 通知, 失败时回退到 MessageBox。
pub fn show_toast(title: &str, message: &str) {
    show_toast_tagged(title, message, "");
}

/// 显示带 tag 的 Toast 通知, 相同 tag 会替换之前的通知。
/// tag 为空字符串时不设置 tag。
pub fn show_toast_tagged(title: &str, message: &str, tag: &str) {
    let xml = format!(
        "<toast><visual><binding template=\"ToastGeneric\">{}<text>{}</text><text>{}</text></binding></visual><audio silent=\"true\"/></toast>",
        icon_element(),
        xml_escape(title),
        xml_escape(message),
    );
    if let Err(e) = show_toast_xml(&xml, if tag.is_empty() { None } else { Some(tag) }) {
        error!("Toast 通知失败: {e}");
        fallback_msgbox(title, &e.to_string());
    }
}

/// 显示带进度条的 Toast 通知 (不确定进度) , 用于下载过程。
pub fn show_progress_toast(title: &str, tag: &str) {
    let xml = format!(
        "<toast><visual><binding template=\"ToastGeneric\">{}<text>{}</text><progress title=\"下载中\" value=\"indeterminate\" valueStringOverride=\"准备下载...\" status=\"\"/></binding></visual><audio silent=\"true\"/></toast>",
        icon_element(),
        xml_escape(title),
    );
    if let Err(e) = show_toast_xml(&xml, Some(tag)) {
        error!("进度 Toast 通知失败 (tag={tag}): {e}");
        fallback_msgbox(title, &e.to_string());
    }
}

// ═══════════════════════════════════════════════
// Internal helpers
// ═══════════════════════════════════════════════

fn icon_element() -> String {
    let uri = ICON_URI.get().map(|s| s.as_str()).unwrap_or("");
    if uri.is_empty() {
        return String::new();
    }
    format!("<image placement=\"appLogoOverride\" src=\"{}\"/>", uri)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn show_toast_xml(xml_str: &str, tag: Option<&str>) -> Result<(), AppError> {
    let xml = XmlDocument::new().map_err(|e| AppError::Msg(format!("创建 XmlDocument 失败: {e}")))?;
    xml.LoadXml(&HSTRING::from(xml_str))
        .map_err(|e| AppError::Msg(format!("加载 toast XML 失败: {e}")))?;
    let toast =
        ToastNotification::CreateToastNotification(&xml).map_err(|e| AppError::Msg(format!("创建 toast 失败: {e}")))?;
    if let Some(t) = tag {
        toast
            .SetTag(&HSTRING::from(t))
            .map_err(|e| AppError::Msg(format!("设置 toast tag 失败: {e}")))?;
    }
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
        .map_err(|e| AppError::Msg(format!("创建 ToastNotifier 失败: {e}")))?;
    notifier
        .Show(&toast)
        .map_err(|e| AppError::Msg(format!("显示 toast 失败: {e}")))?;
    Ok(())
}

fn fallback_msgbox(title: &str, err: &str) {
    let msg = format!("Toast 通知失败: {err}");
    unsafe {
        let title_wide = crate::state::wide(title);
        let msg_wide = crate::state::wide(&msg);
        windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
            std::ptr::null_mut(),
            msg_wide.as_ptr(),
            title_wide.as_ptr(),
            windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
        );
    }
}

// ═══════════════════════════════════════════════
// Shortcut (windows crate IShellLinkW + windows-sys property store)
// ═══════════════════════════════════════════════

/// 确保开始菜单快捷方式存在。如果旧版以 AUMID 命名的快捷方式存在则删除 (迁移) 。
fn ensure_shortcut(exe_path: &std::path::Path, icon_path: &std::path::Path) -> Result<(), AppError> {
    let programs_dir = get_programs_dir()?;
    let lnk_path = programs_dir.join(format!("{}.lnk", SHORTCUT_NAME));

    let old_lnk = programs_dir.join(format!("{}.lnk", AUMID));
    if old_lnk.exists() && old_lnk != lnk_path {
        let _ = std::fs::remove_file(&old_lnk);
    }

    std::fs::create_dir_all(&programs_dir).map_err(|e| AppError::Msg(format!("创建 Programs 目录失败: {e}")))?;
    create_shortcut(exe_path, icon_path, &lnk_path)
}

fn create_shortcut(
    exe_path: &std::path::Path,
    icon_path: &std::path::Path,
    lnk_path: &std::path::Path,
) -> Result<(), AppError> {
    unsafe {
        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| AppError::Msg(format!("创建 ShellLink 失败: {e}")))?;
        shell_link
            .SetPath(&HSTRING::from(exe_path.to_string_lossy().as_ref()))
            .map_err(|e| AppError::Msg(format!("设置链接路径失败: {e}")))?;
        if let Some(parent) = exe_path.parent() {
            shell_link
                .SetWorkingDirectory(&HSTRING::from(parent.to_string_lossy().as_ref()))
                .map_err(|e| AppError::Msg(format!("设置工作目录失败: {e}")))?;
        }
        shell_link
            .SetIconLocation(&HSTRING::from(icon_path.to_string_lossy().as_ref()), 0)
            .map_err(|e| AppError::Msg(format!("设置图标路径失败: {e}")))?;
        let persist: IPersistFile = shell_link
            .cast()
            .map_err(|e| AppError::Msg(format!("获取 IPersistFile 失败: {e}")))?;
        persist
            .Save(&HSTRING::from(lnk_path.to_string_lossy().as_ref()), true)
            .map_err(|e| AppError::Msg(format!("保存快捷方式失败: {e}")))?;
    }

    set_shortcut_aumid(lnk_path)
}

/// 在快捷方式文件上设置 AppUserModelID 属性。
///
/// Windows Toast 通知系统通过 AUMID 识别应用身份。不仅进程需要调用
/// `SetCurrentProcessExplicitAppUserModelID`, 开始菜单中的快捷方式
/// 也需要写入相同的 AUMID, 否则通知无法关联到应用图标。
fn set_shortcut_aumid(lnk_path: &std::path::Path) -> Result<(), AppError> {
    unsafe {
        use windows_sys::Win32::System::Com::StructuredStorage::PROPVARIANT;

        let lnk_wide: Vec<u16> = std::ffi::OsStr::new(&lnk_path.to_string_lossy().as_ref())
            .encode_wide()
            .chain(Some(0))
            .collect();

        let mut pps: *mut c_void = std::ptr::null_mut();
        let hr = windows_sys::Win32::UI::Shell::PropertiesSystem::SHGetPropertyStoreFromParsingName(
            lnk_wide.as_ptr(),
            std::ptr::null_mut(),
            2, // GPS_READWRITE
            &IID_IPROPERTY_STORE,
            &mut pps,
        );
        if hr != 0 {
            return Err(AppError::Msg(format!(
                "SHGetPropertyStoreFromParsingName 失败: 0x{hr:08X}"
            )));
        }
        if pps.is_null() {
            return Err(AppError::Msg("未获取到 IPropertyStore".into()));
        }

        let prop_store = &*(pps as *const IPropertyStoreW);

        let aumid_wide: Vec<u16> = std::ffi::OsStr::new(AUMID).encode_wide().chain(Some(0)).collect();

        // 手动构建 PROPVARIANT 结构体:
        // 偏移 0: VT_LPWSTR 类型标签 (u16)
        // 偏移 8: 指向 null 结尾 Unicode 字符串的指针
        let mut pv: PROPVARIANT = std::mem::zeroed();
        let pv_ptr = &mut pv as *mut PROPVARIANT as *mut u8;
        std::ptr::write(pv_ptr as *mut u16, VT_LPWSTR as u16);
        std::ptr::write(pv_ptr.add(8) as *mut *const u16, aumid_wide.as_ptr());

        let pkey = PROPERTYKEY_S {
            fmtid: GUID_PKEY_AUMID,
            pid: 5,
        };
        let hr = ((*prop_store.0).set_value)(pps, &pkey, &pv);
        if hr != 0 {
            ((*prop_store.0).release)(pps);
            return Err(AppError::Msg(format!("IPropertyStore::SetValue 失败: 0x{hr:08X}")));
        }
        let hr = ((*prop_store.0).commit)(pps);
        ((*prop_store.0).release)(pps);
        if hr != 0 {
            return Err(AppError::Msg(format!("IPropertyStore::Commit 失败: 0x{hr:08X}")));
        }
    }
    Ok(())
}

fn get_programs_dir() -> Result<PathBuf, AppError> {
    let appdata = std::env::var("APPDATA").map_err(|_| AppError::Msg("无法获取 APPDATA 路径".into()))?;
    Ok(PathBuf::from(appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs"))
}

// ═══════════════════════════════════════════════
// COM vtables & types for windows-sys IPropertyStore
// ═══════════════════════════════════════════════

/// COM VARIANT 类型: null 结尾 Unicode 字符串 (LPWSTR) 。
const VT_LPWSTR: i32 = 31;

/// PKEY_AppUserModel_ID 属性的 GUID (来自 propkey.h) 。
const GUID_PKEY_AUMID: windows_sys::core::GUID = windows_sys::core::GUID {
    data1: 0x9F4C2855,
    data2: 0x9F79,
    data3: 0x4B39,
    data4: [0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3],
};

/// IPropertyStore 接口的 IID。
const IID_IPROPERTY_STORE: windows_sys::core::GUID = windows_sys::core::GUID {
    data1: 0x886D8EEB,
    data2: 0x8CF2,
    data3: 0x4446,
    data4: [0x8D, 0x02, 0xCD, 0xBA, 0x1D, 0xBD, 0xCF, 0x99],
};

#[repr(C)]
struct PROPERTYKEY_S {
    fmtid: windows_sys::core::GUID,
    pid: u32,
}

// 手动定义 IPropertyStore COM vtable。
// windows crate 的 IPropertyStore 与 windows-sys 的 PROPVARIANT 存在跨 crate 兼容性问题,
// 因此直接用 windows-sys 构建最小可用的 vtable 来调用 SetValue / Commit。
#[repr(C)]
struct IPropertyStoreVtbl {
    query_interface: unsafe extern "system" fn(
        this: *mut c_void,
        riid: *const windows_sys::core::GUID,
        ppv: *mut *mut c_void,
    ) -> i32,
    add_ref: unsafe extern "system" fn(this: *mut c_void) -> u32,
    release: unsafe extern "system" fn(this: *mut c_void) -> u32,
    _get_count: *const c_void,
    _get_at: *const c_void,
    _get_value: *const c_void,
    set_value: unsafe extern "system" fn(
        this: *mut c_void,
        key: *const PROPERTYKEY_S,
        propvar: *const windows_sys::Win32::System::Com::StructuredStorage::PROPVARIANT,
    ) -> i32,
    commit: unsafe extern "system" fn(this: *mut c_void) -> i32,
}

#[repr(C)]
struct IPropertyStoreW(*mut IPropertyStoreVtbl);
