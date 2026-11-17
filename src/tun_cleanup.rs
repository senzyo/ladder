//! 清理异常 WinTUN 设备和 tun- 网络记录, 保护健康设备及托管核心正在使用的名称。

use crate::tun_config::TUN_PREFIX;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CR_NO_SUCH_DEVNODE, CR_SUCCESS, DN_HAS_PROBLEM, DN_STARTED,
};

fn unprotected_name(name: &str, protected: &[String]) -> bool {
    name.starts_with(TUN_PREFIX) && !is_protected(name, protected)
}

fn is_protected(name: &str, protected: &[String]) -> bool {
    protected
        .iter()
        .any(|active| active.eq_ignore_ascii_case(name))
}

fn orphaned_status(result: u32, status: u32) -> bool {
    result == CR_NO_SUCH_DEVNODE
        || (result == CR_SUCCESS && (status & DN_STARTED == 0 || status & DN_HAS_PROBLEM != 0))
}

pub(crate) fn cleanup(protected_names: &[String]) {
    // Runtime 的进程测试不得碰宿主网卡或注册表。
    #[cfg(test)]
    let _ = protected_names;
    #[cfg(not(test))]
    system::cleanup(protected_names);
}

fn removable_device(name: Option<&str>, protected: &[String], result: u32, status: u32) -> bool {
    !name.is_some_and(|name| is_protected(name, protected)) && orphaned_status(result, status)
}

#[cfg(not(test))]
mod system {
    use super::{removable_device, unprotected_name};
    use crate::logging;
    use std::ffi::OsString;
    use std::io;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::ptr;
    use std::time::{Duration, Instant};
    use windows_registry::LOCAL_MACHINE;
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_Get_DevNode_PropertyW, CM_Get_DevNode_Status, CM_Get_Device_ID_List_SizeW,
        CM_Get_Device_ID_ListW, CM_LOCATE_DEVNODE_PHANTOM, CM_Locate_DevNodeW, CR_BUFFER_SMALL,
        CR_NO_SUCH_DEVNODE, CR_SUCCESS,
    };
    use windows_sys::Win32::Devices::Properties::DEVPROP_TYPE_STRING;
    use windows_sys::Win32::Foundation::{DEVPROPKEY, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
    use windows_sys::core::GUID;

    // Wintun 在创建适配器和改名时写入此属性；FriendlyName 并非接口名称。
    // https://git.zx2c4.com/wintun/tree/api/adapter.c (DEVPKEY_Wintun_Name)
    const WINTUN_NAME: DEVPROPKEY = DEVPROPKEY {
        fmtid: GUID::from_u128(0x3361c968_2f2e_4660_b47e_699cdc4c32b9),
        pid: 3,
    };

    pub(super) fn cleanup(protected_names: &[String]) {
        let mut protected = protected_names.to_vec();
        cleanup_devices(protected_names, &mut protected);
        match interface_names() {
            Ok(names) => protected.extend(names),
            Err(error) => {
                logging::warn(
                    "tun",
                    format_args!("读取现存接口失败, 跳过 TUN 网络记录清理: {error}"),
                );
                return;
            }
        };
        for path in [
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\NetworkList\Profiles",
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\NetworkList\Signatures\Unmanaged",
        ] {
            cleanup_registry(path, &protected);
        }
    }

    fn interface_names() -> io::Result<Vec<String>> {
        let mut table: *mut MIB_IF_TABLE2 = ptr::null_mut();
        // 系统分配表及柔性数组, 所有借用在 FreeMibTable 前结束。
        let result = unsafe { GetIfTable2(&mut table) };
        if result != NO_ERROR {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        if table.is_null() {
            return Err(io::Error::other("系统未返回网络接口表"));
        }
        let rows = unsafe {
            std::slice::from_raw_parts(
                ptr::addr_of!((*table).Table).cast::<MIB_IF_ROW2>(),
                (*table).NumEntries as usize,
            )
        };
        let names = rows
            .iter()
            .map(|row| {
                let end = row
                    .Alias
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(row.Alias.len());
                String::from_utf16_lossy(&row.Alias[..end])
            })
            .collect();
        unsafe { FreeMibTable(table.cast()) };
        Ok(names)
    }

    fn device_ids() -> Result<Vec<u16>, u32> {
        // 设备可能在两次调用间变化, 缓冲区不足时重新枚举。
        for _ in 0..3 {
            let mut size = 0;
            let result = unsafe { CM_Get_Device_ID_List_SizeW(&mut size, ptr::null(), 0) };
            if result != CR_SUCCESS {
                return Err(result);
            }
            if size == 0 {
                return Ok(Vec::new());
            }
            let mut buffer = vec![0; size as usize];
            let result =
                unsafe { CM_Get_Device_ID_ListW(ptr::null(), buffer.as_mut_ptr(), size, 0) };
            if result == CR_SUCCESS {
                return Ok(buffer);
            }
            if result != CR_BUFFER_SMALL {
                return Err(result);
            }
        }
        Err(CR_BUFFER_SMALL)
    }

    fn device_name(node: u32) -> Option<String> {
        // Wintun 的适配器名称上限为 MAX_ADAPTER_NAME (128 个 WCHAR)。
        let mut name = [0u16; 256];
        let mut bytes = size_of_val(&name) as u32;
        let mut kind = 0;
        let result = unsafe {
            CM_Get_DevNode_PropertyW(
                node,
                &WINTUN_NAME,
                &mut kind,
                name.as_mut_ptr().cast(),
                &mut bytes,
                0,
            )
        };
        if result != CR_SUCCESS || kind != DEVPROP_TYPE_STRING {
            return None;
        }
        let end = name.iter().position(|&c| c == 0)?;
        String::from_utf16(&name[..end]).ok()
    }

    fn cleanup_devices(managed_names: &[String], protected_registry: &mut Vec<String>) {
        let ids = match device_ids() {
            Ok(ids) => ids,
            Err(code) => {
                logging::warn(
                    "tun",
                    format_args!("枚举 WinTUN 设备失败: 配置管理器错误 {code}"),
                );
                return;
            }
        };
        let mut removed = 0;
        for id in ids.split(|&c| c == 0).take_while(|id| !id.is_empty()) {
            let id = String::from_utf16_lossy(id);
            if !id.to_ascii_uppercase().contains("WINTUN") {
                continue;
            }
            let wide_id: Vec<_> = id.encode_utf16().chain(Some(0)).collect();
            let mut node = 0;
            let located = unsafe {
                CM_Locate_DevNodeW(&mut node, wide_id.as_ptr(), CM_LOCATE_DEVNODE_PHANTOM)
            };
            if located != CR_SUCCESS && located != CR_NO_SUCH_DEVNODE {
                continue;
            }
            let name = (located == CR_SUCCESS).then(|| device_name(node)).flatten();
            let mut status = 0;
            let mut problem = 0;
            let result = if located == CR_SUCCESS {
                unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, node, 0) }
            } else {
                located
            };
            if !removable_device(name.as_deref(), managed_names, result, status) {
                // 已启动但尚未出现在接口表中的设备, 也必须保留其网络记录。
                protected_registry.extend(name);
                continue;
            }
            match pnputil(&["/remove-device", &id]) {
                Ok(()) => removed += 1,
                Err(error) => {
                    logging::warn("tun", format_args!("移除残留 WinTUN 设备失败: {error}"))
                }
            }
        }
        if removed > 0 {
            logging::info("tun", format_args!("已移除 {removed} 个残留 WinTUN 设备"));
            if let Err(error) = pnputil(&["/scan-devices"]) {
                logging::warn("tun", format_args!("重新扫描网络设备失败: {error}"));
            }
        }
    }

    fn cleanup_registry(path: &str, protected: &[String]) {
        let result = (|| -> windows_registry::Result<usize> {
            let parent = LOCAL_MACHINE.open(path)?;
            let names: Vec<_> = parent.keys()?.collect();
            let mut removed = 0;
            for name in names {
                let Ok(key) = parent.open(&name) else {
                    continue;
                };
                let Ok(description) = key.get_string("Description") else {
                    continue;
                };
                if unprotected_name(&description, protected) {
                    match LOCAL_MACHINE.remove_tree(format!("{path}\\{name}")) {
                        Ok(()) => removed += 1,
                        Err(error) => {
                            logging::warn("tun", format_args!("删除 TUN 网络记录失败: {error}"))
                        }
                    }
                }
            }
            Ok(removed)
        })();
        match result {
            Ok(0) => {}
            Ok(count) => logging::info("tun", format_args!("已清理 {count} 条 TUN 网络记录")),
            Err(error) => logging::warn("tun", format_args!("清理 TUN 网络记录失败: {error}")),
        }
    }

    fn pnputil(args: &[&str]) -> io::Result<()> {
        let mut directory = [0u16; 32768];
        let length = unsafe { GetSystemDirectoryW(directory.as_mut_ptr(), directory.len() as u32) };
        if length == 0 || length as usize >= directory.len() {
            return Err(io::Error::last_os_error());
        }
        let exe =
            PathBuf::from(OsString::from_wide(&directory[..length as usize])).join("pnputil.exe");
        let mut child = Command::new(exe)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = match child.try_wait() {
                Ok(status) => status,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
            };
            if let Some(status) = status {
                return if status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other(
                        "无法清理残留 TUN 设备, 请检查管理员权限及设备是否被占用",
                    ))
                };
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(io::ErrorKind::TimedOut, "等待 pnputil 超时"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tun_network_records_outside_the_protected_set_are_candidates() {
        let protected = vec!["tun-0123456789abcdef".into(), "tun-fedcba9876543210".into()];
        assert!(unprotected_name("tun-0000000000000000", &protected));
        assert!(unprotected_name("tun-other", &protected));
        assert!(unprotected_name("tun-123abc", &protected));
        assert!(!unprotected_name("tun-0123456789abcdef", &protected));
        assert!(!unprotected_name("tun-fedcba9876543210", &protected));
        assert!(!unprotected_name("tun-0123456789ABCDEF", &protected));
        for foreign in ["Ethernet", "Wintun", "WireGuard", "sing-tun"] {
            assert!(!unprotected_name(foreign, &protected));
        }
    }

    #[test]
    fn healthy_devices_and_unknown_status_failures_are_never_removed() {
        assert!(orphaned_status(CR_NO_SUCH_DEVNODE, 0));
        assert!(orphaned_status(CR_SUCCESS, 0));
        assert!(orphaned_status(CR_SUCCESS, DN_STARTED | DN_HAS_PROBLEM));
        assert!(!orphaned_status(CR_SUCCESS, DN_STARTED));
        assert!(!orphaned_status(0x33, 0));
        assert!(!orphaned_status(0x33, DN_HAS_PROBLEM));
    }

    #[test]
    fn all_orphaned_wintun_names_are_eligible_except_managed_sessions() {
        let protected = vec!["tun-running".into()];
        for name in [None, Some("another-vpn"), Some("tun-old")] {
            assert!(removable_device(name, &protected, CR_SUCCESS, 0));
            assert!(removable_device(name, &protected, CR_NO_SUCH_DEVNODE, 0));
            assert!(!removable_device(name, &protected, CR_SUCCESS, DN_STARTED));
            assert!(!removable_device(name, &protected, 0x33, 0));
        }
        assert!(!removable_device(
            Some("tun-running"),
            &protected,
            CR_SUCCESS,
            DN_HAS_PROBLEM
        ));
        assert!(!removable_device(
            Some("TUN-RUNNING"),
            &protected,
            CR_NO_SUCH_DEVNODE,
            0
        ));
    }
}
