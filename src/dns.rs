//! Xray TUN 的 DNS 接管。先保存原值, 再分网卡、分地址族设置回环 DNS。

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use windows_registry::LOCAL_MACHINE;
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_NOT_FOUND, ERROR_NOT_SUPPORTED, NO_ERROR,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    DNS_INTERFACE_SETTINGS, DNS_INTERFACE_SETTINGS_VERSION1, DNS_SETTING_IPV6,
    DNS_SETTING_NAMESERVER, FreeMibTable, GetIfTable2, GetIpInterfaceEntry,
    InitializeIpInterfaceEntry, MIB_IF_ROW2, MIB_IF_TABLE2, MIB_IPINTERFACE_ROW,
    SetInterfaceDnsSettings,
};
use windows_sys::Win32::NetworkManagement::Ndis::{IfOperStatusUp, NET_IF_ACCESS_LOOPBACK};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows_sys::core::GUID;

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
struct Target {
    guid: u128,
    ipv6: bool,
}

impl Target {
    fn loopback(self) -> &'static str {
        if self.ipv6 { "::1" } else { "127.0.0.1" }
    }

    fn is_ladder_value(self, servers: &str) -> bool {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
        let expected = if self.ipv6 {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        };
        servers.trim().parse::<IpAddr>() == Ok(expected)
    }

    fn registry_path(self) -> String {
        let id = format!("{:032X}", self.guid);
        let service = if self.ipv6 { "Tcpip6" } else { "Tcpip" };
        format!(
            r"SYSTEM\CurrentControlSet\Services\{service}\Parameters\Interfaces\{{{}-{}-{}-{}-{}}}",
            &id[..8],
            &id[8..12],
            &id[12..16],
            &id[16..20],
            &id[20..]
        )
    }
}

#[derive(Serialize, Deserialize)]
struct SavedDns {
    target: Target,
    // 只读取 NameServer (静态覆盖), 空字符串表示自动, 不保存 DHCP 分配的临时地址。
    previous: String,
}

pub struct DnsLease {
    journal: PathBuf,
}

impl DnsLease {
    pub fn new(base: &Path) -> Self {
        Self {
            journal: base.join("dns-recovery.json"),
        }
    }

    pub fn apply(&mut self) -> Result<(), String> {
        self.restore()?;
        self.install(physical_targets()?, read_static, set_static)
    }

    pub fn restore(&mut self) -> Result<(), String> {
        self.restore_with(read_static, set_static)
    }

    fn install(
        &self,
        targets: Vec<Target>,
        read: impl Fn(Target) -> Result<String, String>,
        write: impl Fn(Target, &str) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut changes = Vec::new();
        for target in targets {
            let previous = read(target)?;
            if !target.is_ladder_value(&previous) {
                changes.push(SavedDns { target, previous });
            }
        }
        if changes.is_empty() {
            return Ok(());
        }
        self.save_changes(&changes)?;
        for change in &changes {
            let target = change.target;
            if let Err(e) = write(target, target.loopback()) {
                return match self.restore_with(read, write) {
                    Ok(()) => Err(e),
                    Err(restore_error) => Err(format!("{e}；{restore_error}")),
                };
            }
        }
        Ok(())
    }

    fn save_changes(&self, changes: &[SavedDns]) -> Result<(), String> {
        let staged = self.journal.with_extension("tmp");
        let data = serde_json::to_vec(changes).map_err(|e| e.to_string())?;
        let save = || -> io::Result<()> {
            let mut file = fs::File::create(&staged)?;
            file.write_all(&data)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&staged, &self.journal)
        };
        save().map_err(|e| format!("无法保存 DNS 恢复记录: {e}"))
    }

    fn restore_with(
        &self,
        read: impl Fn(Target) -> Result<String, String>,
        write: impl Fn(Target, &str) -> Result<(), String>,
    ) -> Result<(), String> {
        let data = match fs::read(&self.journal) {
            Ok(data) => data,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("无法读取 DNS 恢复记录: {e}")),
        };
        let changes: Vec<SavedDns> =
            serde_json::from_slice(&data).map_err(|e| format!("DNS 恢复记录无效: {e}"))?;
        let mut errors = Vec::new();
        let mut pending = Vec::new();
        for change in changes {
            let target = change.target;
            // 仅还原仍为自己所设回环值的网卡。这样也可重试部分恢复、保留外部手动修改。
            let result = read(target).and_then(|current| {
                if target.is_ladder_value(&current) {
                    write(target, &change.previous)
                } else {
                    Ok(())
                }
            });
            if let Err(error) = result {
                errors.push(error);
                pending.push(change);
            }
        }
        if !errors.is_empty() {
            // 已成功恢复的网卡交还用户, 重试时即使用户自己设置了回环地址也不再改动。
            if let Err(error) = self.save_changes(&pending) {
                errors.push(error);
            }
            return Err(format!("恢复 DNS 失败: {}", errors.join("；")));
        }
        fs::remove_file(&self.journal).map_err(|e| format!("无法清除 DNS 恢复记录: {e}"))
    }
}

fn physical_targets() -> Result<Vec<Target>, String> {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: 系统填写输出指针, 复制所需行数据后释放整张表。
    let status = unsafe { GetIfTable2(&mut table) };
    if status != NO_ERROR {
        return Err(format!("无法枚举物理网卡: {}", os_error(status)));
    }
    // SAFETY: 成功返回的表包含 NumEntries 个实际分配的行, Table 是柔性数组的占位字段。
    let rows = unsafe {
        std::slice::from_raw_parts(
            std::ptr::addr_of!((*table).Table).cast::<MIB_IF_ROW2>(),
            (*table).NumEntries as usize,
        )
    };
    let adapters: Vec<_> = rows
        .iter()
        .filter(|row| {
            // MIB_IF_ROW2.InterfaceAndOperStatusFlags: HardwareInterface | ConnectorPresent.
            const PHYSICAL: u8 = 0b101;
            row.OperStatus == IfOperStatusUp
                && row.AccessType != NET_IF_ACCESS_LOOPBACK
                && row.InterfaceAndOperStatusFlags._bitfield & PHYSICAL == PHYSICAL
        })
        .map(|row| (row.InterfaceGuid, row.InterfaceIndex))
        .collect();
    // SAFETY: GetIfTable2 分配的内存已不再被引用。
    unsafe { FreeMibTable(table.cast()) };

    let mut targets = Vec::new();
    for (id, index) in adapters {
        let guid = (u128::from(id.data1) << 96)
            | (u128::from(id.data2) << 80)
            | (u128::from(id.data3) << 64)
            | u128::from(u64::from_be_bytes(id.data4));
        for ipv6 in [false, true] {
            let mut row = MIB_IPINTERFACE_ROW::default();
            // SAFETY: 本地存储有效, 查询前填写地址族及接口编号。
            unsafe { InitializeIpInterfaceEntry(&mut row) };
            row.Family = if ipv6 { AF_INET6 } else { AF_INET };
            row.InterfaceIndex = index;
            let status = unsafe { GetIpInterfaceEntry(&mut row) };
            match status {
                NO_ERROR => targets.push(Target { guid, ipv6 }),
                ERROR_NOT_FOUND | ERROR_FILE_NOT_FOUND | ERROR_NOT_SUPPORTED => {}
                _ => return Err(format!("无法查询网卡协议: {}", os_error(status))),
            }
        }
    }
    Ok(targets)
}

fn read_static(target: Target) -> Result<String, String> {
    let result = LOCAL_MACHINE
        .options()
        .read()
        .open(target.registry_path())
        .and_then(|key| key.get_string("NameServer"));
    match result {
        Ok(value) => Ok(value),
        // 未配置静态 DNS 的接口可能没有 NameServer 值。
        Err(e) if e.code().0 as u32 == (0x8007_0000 | ERROR_FILE_NOT_FOUND) => Ok(String::new()),
        Err(e) => Err(format!("无法读取网卡 DNS 设置: {e}")),
    }
}

fn set_static(target: Target, servers: &str) -> Result<(), String> {
    let mut name_server: Vec<_> = servers.encode_utf16().chain(Some(0)).collect();
    let flags = DNS_SETTING_NAMESERVER | if target.ipv6 { DNS_SETTING_IPV6 } else { 0 };
    let settings = DNS_INTERFACE_SETTINGS {
        Version: DNS_INTERFACE_SETTINGS_VERSION1,
        Flags: u64::from(flags),
        NameServer: name_server.as_mut_ptr(),
        ..Default::default()
    };
    // SAFETY: 传入目标网卡 GUID, NameServer 在同步调用结束前有效。
    let status = unsafe { SetInterfaceDnsSettings(GUID::from_u128(target.guid), &settings) };
    match status {
        NO_ERROR => Ok(()),
        ERROR_ACCESS_DENIED => Err("设置网卡 DNS 需要管理员权限, 请以管理员身份运行 Ladder".into()),
        _ => Err(format!("设置网卡 DNS 失败: {}", os_error(status))),
    }
}

fn os_error(code: u32) -> io::Error {
    io::Error::from_raw_os_error(code as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const V4: Target = Target {
        guid: u128::MAX,
        ipv6: false,
    };
    const V6: Target = Target {
        guid: u128::MAX,
        ipv6: true,
    };

    #[test]
    fn static_and_dhcp_survive_restart_and_partial_restoration() {
        let dir = tempfile::tempdir().unwrap();
        let lease = DnsLease::new(dir.path());
        let ipv4 = "192.0.2.1,192.0.2.2";
        let state = RefCell::new(HashMap::from([(V4, ipv4.to_owned()), (V6, String::new())]));
        let read = |target| Ok(state.borrow()[&target].clone());
        let write = |target, value: &str| {
            state.borrow_mut().insert(target, value.to_owned());
            Ok(())
        };
        lease.install(vec![V4, V6], read, write).unwrap();
        assert_eq!(read(V4).unwrap(), "127.0.0.1");
        assert_eq!(read(V6).unwrap(), "::1");

        // 模拟进程重启、格式归一化及单个协议恢复失败, 成功的协议不应被再次写入。
        let restarted = DnsLease::new(dir.path());
        write(V6, "0:0:0:0:0:0:0:1").unwrap();
        let failure = restarted.restore_with(read, |target, value| {
            if target == V6 {
                Err("暂时不可用".into())
            } else {
                write(target, value)
            }
        });
        assert!(failure.is_err());
        assert_eq!(read(V4).unwrap(), ipv4);
        assert!(restarted.journal.exists());
        write(V4, "127.0.0.1").unwrap();
        restarted.restore_with(read, write).unwrap();
        assert_eq!(read(V4).unwrap(), "127.0.0.1");
        assert_eq!(read(V6).unwrap(), "");
        assert!(!restarted.journal.exists());
    }

    #[test]
    fn failed_apply_rolls_back_only_the_changes_already_made() {
        let dir = tempfile::tempdir().unwrap();
        let lease = DnsLease::new(dir.path());
        let state = RefCell::new(HashMap::from([
            (V4, String::new()),
            (V6, "2001:db8::1".into()),
        ]));
        let read = |target| Ok(state.borrow()[&target].clone());
        let result = lease.install(vec![V4, V6], read, |target, value| {
            assert!(lease.journal.exists());
            if target == V6 {
                return Err("写入失败".into());
            }
            state.borrow_mut().insert(target, value.to_owned());
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(read(V4).unwrap(), "");
        assert_eq!(read(V6).unwrap(), "2001:db8::1");
        assert!(!lease.journal.exists());
    }
}
