//! Windows Toast 通知。
//!
//! 通过 Windows COM ToastNotification API 弹出系统通知。
//! 首次运行时会在开始菜单创建快捷方式并设置 AUMID (AppUserModelID) ,
//! 这是 Windows 识别应用身份以显示 Toast 通知的必要条件。

use std::ffi::{OsStr, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing::error;
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
use windows::Win32::UI::Shell::{IShellLinkW, SetCurrentProcessExplicitAppUserModelID, ShellLink};
use windows::core::HSTRING;
use windows_sys::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows_sys::core::GUID;

use crate::app_error::AppError;
use crate::windows_strings;

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
pub fn setup(exe_path: &Path) -> Result<(), AppError> {
    // SAFETY: HSTRING 提供以空字符结尾的 UTF-16, 并存活到同步调用结束。
    unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID)) }
        .map_err(|e| AppError::Msg(format!("设置 AUMID 失败: {e}")))?;
    let icon_path = exe_path
        .parent()
        .unwrap_or(exe_path)
        .join("icons")
        .join("ladder.ico");
    let _ = ICON_URI.set(format!(
        "file:///{}",
        icon_path.to_string_lossy().replace('\\', "/")
    ));
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

#[must_use]
fn icon_element() -> String {
    let uri = ICON_URI.get().map(|s| s.as_str()).unwrap_or("");
    if uri.is_empty() {
        return String::new();
    }
    format!("<image placement=\"appLogoOverride\" src=\"{}\"/>", uri)
}

#[must_use]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn show_toast_xml(xml_str: &str, tag: Option<&str>) -> Result<(), AppError> {
    let xml =
        XmlDocument::new().map_err(|e| AppError::Msg(format!("创建 XmlDocument 失败: {e}")))?;
    xml.LoadXml(&HSTRING::from(xml_str))
        .map_err(|e| AppError::Msg(format!("加载 toast XML 失败: {e}")))?;
    let toast = ToastNotification::CreateToastNotification(&xml)
        .map_err(|e| AppError::Msg(format!("创建 toast 失败: {e}")))?;
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
    let title_wide = windows_strings::wide(title);
    let msg_wide = windows_strings::wide(&msg);
    // SAFETY: 无所有者窗口是允许的; 两个 UTF-16 缓冲区以空字符结尾,
    // 在同步 MessageBoxW 返回前保持有效, Win32 不取得缓冲区所有权。
    unsafe {
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
fn ensure_shortcut(exe_path: &Path, icon_path: &Path) -> Result<(), AppError> {
    let programs_dir = get_programs_dir()?;
    let lnk_path = programs_dir.join(format!("{}.lnk", SHORTCUT_NAME));

    let old_lnk = programs_dir.join(format!("{}.lnk", AUMID));
    if old_lnk.exists() && old_lnk != lnk_path {
        let _ = std::fs::remove_file(&old_lnk);
    }

    std::fs::create_dir_all(&programs_dir)
        .map_err(|e| AppError::Msg(format!("创建 Programs 目录失败: {e}")))?;
    create_shortcut(exe_path, icon_path, &lnk_path)
}

fn create_shortcut(exe_path: &Path, icon_path: &Path, lnk_path: &Path) -> Result<(), AppError> {
    use windows::core::Interface;

    {
        // SAFETY: 创建 Windows ShellLink 并请求其 IShellLinkW 接口, 不聚合外部对象。
        // 接口仅在当前线程使用; COM 未初始化等失败沿原 Result 路径返回。
        let shell_link: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| AppError::Msg(format!("创建 ShellLink 失败: {e}")))?;
        // SAFETY: shell_link 持有有效接口; HSTRING 的空字符结尾缓冲区覆盖同步调用。
        unsafe { shell_link.SetPath(&HSTRING::from(exe_path.to_string_lossy().as_ref())) }
            .map_err(|e| AppError::Msg(format!("设置链接路径失败: {e}")))?;
        if let Some(parent) = exe_path.parent() {
            // SAFETY: 同一线程的有效接口, 临时 HSTRING 保持到调用结束。
            unsafe {
                shell_link.SetWorkingDirectory(&HSTRING::from(parent.to_string_lossy().as_ref()))
            }
            .map_err(|e| AppError::Msg(format!("设置工作目录失败: {e}")))?;
        }
        // SAFETY: 同一线程的有效接口; 路径缓冲区覆盖调用, 图标索引为既有的 0。
        unsafe {
            shell_link.SetIconLocation(&HSTRING::from(icon_path.to_string_lossy().as_ref()), 0)
        }
        .map_err(|e| AppError::Msg(format!("设置图标路径失败: {e}")))?;
        let persist: IPersistFile = shell_link
            .cast()
            .map_err(|e| AppError::Msg(format!("获取 IPersistFile 失败: {e}")))?;
        // SAFETY: persist 来自同一对象的 QueryInterface; 路径缓冲区覆盖同步保存调用。
        unsafe { persist.Save(&HSTRING::from(lnk_path.to_string_lossy().as_ref()), true) }
            .map_err(|e| AppError::Msg(format!("保存快捷方式失败: {e}")))?;
    }

    set_shortcut_aumid(lnk_path)
}

/// 在快捷方式文件上设置 AppUserModelID 属性。
///
/// Windows Toast 通知系统通过 AUMID 识别应用身份。不仅进程需要调用
/// `SetCurrentProcessExplicitAppUserModelID`, 开始菜单中的快捷方式
/// 也需要写入相同的 AUMID, 否则通知无法关联到应用图标。
fn set_shortcut_aumid(lnk_path: &Path) -> Result<(), AppError> {
    use std::os::windows::ffi::OsStrExt;

    let lnk_wide: Vec<u16> = OsStr::new(&lnk_path.to_string_lossy().as_ref())
        .encode_wide()
        .chain(Some(0))
        .collect();

    let mut pps: *mut c_void = std::ptr::null_mut();
    // SAFETY: 路径以空字符结尾且覆盖调用; IID 为 IPropertyStore, 输出槽可写。
    // 不传入绑定上下文, 成功后的接口在本线程使用并按既有路径 Release。
    let hr = unsafe {
        windows_sys::Win32::UI::Shell::PropertiesSystem::SHGetPropertyStoreFromParsingName(
            lnk_wide.as_ptr(),
            std::ptr::null_mut(),
            2, // GPS_READWRITE
            &IID_IPROPERTY_STORE,
            &mut pps,
        )
    };
    if hr != 0 {
        return Err(AppError::Msg(format!(
            "SHGetPropertyStoreFromParsingName 失败: 0x{hr:08X}"
        )));
    }
    if pps.is_null() {
        return Err(AppError::Msg("未获取到 IPropertyStore".into()));
    }

    // SAFETY: S_OK 且非空的输出是所请求的 IPropertyStore; repr(C) 接口首字段
    // 为下方按 COM ABI 定义的 vtable 指针, 此引用最后一次使用不晚于 Release。
    let prop_store = unsafe { &*(pps as *const IPropertyStoreW) };

    let aumid_wide: Vec<u16> = OsStr::new(AUMID).encode_wide().chain(Some(0)).collect();

    // windows-sys 的 Default 与原先 zeroed 相同, 且此绑定无析构清理。
    let mut pv = PROPVARIANT::default();
    let pv_ptr = &mut pv as *mut PROPVARIANT as *mut u8;
    // SAFETY: 当前绑定与 Windows ABI 在 amd64/ARM64 上均为偏移 0 的 u16 标签,
    // 以及偏移 8 的指针联合体; 两次写入均在 pv 内且满足对齐。
    // VT_LPWSTR 借用仍存活的 Vec, 不调用 PropVariantClear, 不转交 Vec 的所有权。
    unsafe {
        std::ptr::write(pv_ptr as *mut u16, VT_LPWSTR as u16);
        std::ptr::write(pv_ptr.add(8) as *mut *const u16, aumid_wide.as_ptr());
    }

    let pkey = PROPERTYKEY_S {
        fmtid: GUID_PKEY_AUMID,
        pid: 5,
    };
    // SAFETY: 接口引用尚未释放, vtable 与 IID 对应; pkey/pv 及 UTF-16 Vec 均
    // 保持到 Release 完成, REFPROPVARIANT 仅借用输入, 不转交 Vec 的所有权。
    let hr = unsafe { ((*prop_store.0).set_value)(pps, &pkey, &pv) };
    if hr != 0 {
        // SAFETY: 归还 SHGetPropertyStoreFromParsingName 返回的这次引用, 之后立即返回。
        unsafe { ((*prop_store.0).release)(pps) };
        return Err(AppError::Msg(format!(
            "IPropertyStore::SetValue 失败: 0x{hr:08X}"
        )));
    }
    // SAFETY: 同一线程上的接口引用仍有效, Commit 不接收额外裸指针。
    let hr = unsafe { ((*prop_store.0).commit)(pps) };
    // SAFETY: 无论 Commit 成败均归还所取得的引用, 后续不再访问接口或 vtable。
    unsafe { ((*prop_store.0).release)(pps) };
    if hr != 0 {
        return Err(AppError::Msg(format!(
            "IPropertyStore::Commit 失败: 0x{hr:08X}"
        )));
    }
    Ok(())
}

fn get_programs_dir() -> Result<PathBuf, AppError> {
    let appdata =
        std::env::var("APPDATA").map_err(|_| AppError::Msg("无法获取 APPDATA 路径".into()))?;
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
const GUID_PKEY_AUMID: GUID = GUID {
    data1: 0x9f4c2855,
    data2: 0x9f79,
    data3: 0x4b39,
    data4: [0xa8, 0xd0, 0xe1, 0xd4, 0x2d, 0xe1, 0xd5, 0xf3],
};

/// IPropertyStore 接口的 IID。
const IID_IPROPERTY_STORE: GUID = GUID {
    data1: 0x886d8eeb,
    data2: 0x8cf2,
    data3: 0x4446,
    data4: [0x8d, 0x02, 0xcd, 0xba, 0x1d, 0xbd, 0xcf, 0x99],
};

#[repr(C)]
struct PROPERTYKEY_S {
    fmtid: GUID,
    pid: u32,
}

// 手动定义 IPropertyStore COM vtable, 顺序对应 IUnknown 的三项方法,
// 再依次为 GetCount/GetAt/GetValue/SetValue/Commit; 占位槽保持指针宽度。
// typed COM 迁移已延期: windows 0.62.2 将非负 HRESULT 统一视为成功,
// 而此处须保留原 hr != 0 判定。属性布局、借用缓冲区和手动 Release 保持不变。
#[repr(C)]
struct IPropertyStoreVtbl {
    query_interface: unsafe extern "system" fn(
        this: *mut c_void,
        riid: *const GUID,
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
        propvar: *const PROPVARIANT,
    ) -> i32,
    commit: unsafe extern "system" fn(this: *mut c_void) -> i32,
}

#[repr(C)]
struct IPropertyStoreW(*mut IPropertyStoreVtbl);
