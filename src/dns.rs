//! 物理网卡 DNS 管理, 防止 xray TUN 模式下的 DNS 泄漏。
//!
//! xray 使用 wintun.dll 时, Windows 的"多接口并发/回退"机制会导致 DNS 查询
//! 同时通过物理网卡发送, 造成 DNS 泄漏。本模块在 xray TUN 模式运行期间,
//! 将物理网卡 DNS 设置为 127.0.0.1 和 ::1, 停止后恢复为 DHCP 自动获取。
//!
//! 使用 `GetIfTable2` API 检测物理网卡, 通过 `InterfaceAndOperStatusFlags.ConnectorPresent`
//! 标志判断, 这与 `Get-NetAdapter -Physical` 使用相同的检测机制。

use std::ffi::c_void;
use tracing::{debug, info, warn};
use windows::Win32::Foundation::{NO_ERROR, WIN32_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2,
};
use windows::core::Error;
use windows_registry::LOCAL_MACHINE;

/// MIB_IF_ROW2_0 bitfield 中 ConnectorPresent 标志的位偏移。
/// 根据 Windows 文档: Bit 2 = ConnectorPresent。
const CONNECTOR_PRESENT_BIT: u8 = 1 << 2;

/// MIB_IF_ROW2_0 bitfield 中 HardwareInterface 标志的位偏移。
/// Bit 0 = HardwareInterface。
const HARDWARE_INTERFACE_BIT: u8 = 1 << 0;

/// 物理网卡信息。
struct PhysicalAdapter {
    /// 接口别名 (如 "Ethernet", "Wi-Fi") , 用于日志显示。
    alias: String,
    /// 接口 GUID, 用于注册表路径。
    guid: String,
}

/// 获取物理网卡列表, 保留普通操作的发现与失败诊断。
#[must_use]
fn get_physical_adapters() -> Vec<PhysicalAdapter> {
    match get_physical_adapters_without_logging() {
        Ok(adapters) => {
            for adapter in &adapters {
                debug!("发现物理网卡: {} (GUID: {})", adapter.alias, adapter.guid);
            }
            adapters
        }
        Err(result) => {
            warn!("GetIfTable2 调用失败: {result:?}");
            Vec::new()
        }
    }
}

/// 获取物理网卡列表, 不访问日志或应用状态。
///
/// 使用 `GetIfTable2` API 枚举所有网络接口, 筛选条件:
/// - `ConnectorPresent` = true (物理连接器存在)
/// - `HardwareInterface` = true (硬件接口)
/// - `AccessType` != `NET_IF_ACCESS_LOOPBACK` (非回环接口)
///
/// 这与 `Get-NetAdapter -Physical` 使用相同的检测机制。
fn get_physical_adapters_without_logging() -> Result<Vec<PhysicalAdapter>, WIN32_ERROR> {
    let mut adapters = Vec::new();
    let mut table_ptr: *mut MIB_IF_TABLE2 = std::ptr::null_mut();

    // SAFETY: 输出指针指向本地可写变量; 成功后系统分配的表在遍历结束前保持存活。
    let result = unsafe { GetIfTable2(&mut table_ptr) };
    if result != NO_ERROR || table_ptr.is_null() {
        return Err(result);
    }

    // SAFETY: 已检查成功和非空, GetIfTable2 返回正确对齐且已初始化的表头。
    let count = unsafe { (*table_ptr).NumEntries } as usize;
    // SAFETY: 通过原始分配指针定位柔性数组, 不先借用绑定中长度为 1 的占位数组;
    // repr(C) 字段偏移及 MIB_IF_ROW2 大小保留 Windows 要求的对齐填充。
    let rows_ptr = unsafe { std::ptr::addr_of!((*table_ptr).Table).cast::<MIB_IF_ROW2>() };
    // SAFETY: 系统为 count 项分配并初始化同一块内存, 长度在分配范围内;
    // 本函数只读这些项, 所有借用在 FreeMibTable 之前结束, 没有并发释放者。
    let rows = unsafe { std::slice::from_raw_parts(rows_ptr, count) };

    for row in rows {
        let flags = row.InterfaceAndOperStatusFlags._bitfield;

        // 检查 ConnectorPresent 和 HardwareInterface 标志
        let connector_present = (flags & CONNECTOR_PRESENT_BIT) != 0;
        let hardware_interface = (flags & HARDWARE_INTERFACE_BIT) != 0;

        if !connector_present || !hardware_interface {
            continue;
        }

        // 排除回环接口 (AccessType = 3 = NET_IF_ACCESS_LOOPBACK)
        // NET_IF_ACCESS_LOOPBACK 的值是 3
        if row.AccessType.0 == 3 {
            continue;
        }

        // 提取接口别名 (UTF-16 字符串)
        let alias_end = row
            .Alias
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(row.Alias.len());
        let alias = String::from_utf16_lossy(&row.Alias[..alias_end]);

        // 提取接口 GUID
        let guid = format!(
            "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            row.InterfaceGuid.data1,
            row.InterfaceGuid.data2,
            row.InterfaceGuid.data3,
            row.InterfaceGuid.data4[0],
            row.InterfaceGuid.data4[1],
            row.InterfaceGuid.data4[2],
            row.InterfaceGuid.data4[3],
            row.InterfaceGuid.data4[4],
            row.InterfaceGuid.data4[5],
            row.InterfaceGuid.data4[6],
            row.InterfaceGuid.data4[7]
        );

        adapters.push(PhysicalAdapter { alias, guid });
    }

    // SAFETY: 这是 GetIfTable2 的原始分配地址, 所有行借用已结束, 且只释放一次。
    unsafe { FreeMibTable(table_ptr as *const c_void) };

    Ok(adapters)
}

/// 设置物理网卡 DNS 防止 xray TUN 模式下的 DNS 泄漏。
///
/// IPv4 DNS 设置为 `127.0.0.1`, IPv6 DNS 设置为 `::1`。
/// 通过修改注册表 `NameServer` 值实现, Windows 会优先使用此值作为静态 DNS。
///
/// 失败仅记录警告, 不阻断调用流程。
pub fn set_physical_dns_to_local() {
    let adapters = get_physical_adapters();
    if adapters.is_empty() {
        debug!("未发现物理网卡, 跳过 DNS 设置");
        return;
    }

    info!("设置物理网卡 DNS 为本地回环地址防止 DNS 泄漏...");
    set_adapters_dns(&adapters, "127.0.0.1", "::1", set_registry_name_server);
}

/// 恢复物理网卡 DNS 为 DHCP 自动获取。
///
/// 将 IPv4 和 IPv6 的 `NameServer` 设置为空字符串, Windows 会回退到
/// `DhcpNameServer` (由 DHCP 客户端服务维护) 。
///
/// 失败仅记录警告, 不阻断调用流程。
pub fn restore_dns_to_dhcp() {
    let adapters = get_physical_adapters();
    if adapters.is_empty() {
        debug!("未发现物理网卡, 跳过 DNS 恢复");
        return;
    }

    info!("恢复物理网卡 DNS 为 DHCP 自动获取...");
    set_adapters_dns(&adapters, "", "", set_registry_name_server);
}

/// 尽力恢复 DHCP DNS, 全调用链不使用 tracing、应用状态或通知。
///
/// 枚举失败直接结束; 单次注册表打开或写入失败不妨碍后续接口的恢复尝试。
pub(crate) fn restore_dns_to_dhcp_without_logging() {
    let Ok(adapters) = get_physical_adapters_without_logging() else {
        return;
    };
    set_adapters_dns(&adapters, "", "", |path, value, _, _| {
        let _ = set_registry_name_server_without_logging(path, value);
    });
}

/// 每张网卡依次设置 IPv4、IPv6, 由调用方选择正常诊断或无日志写入。
fn set_adapters_dns(
    adapters: &[PhysicalAdapter],
    ipv4_value: &str,
    ipv6_value: &str,
    mut set_name_server: impl FnMut(&str, &str, &str, &str),
) {
    for adapter in adapters {
        let ipv4_path = format!(
            r"SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\{}",
            adapter.guid
        );
        let ipv6_path = format!(
            r"SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters\Interfaces\{}",
            adapter.guid
        );
        set_name_server(&ipv4_path, ipv4_value, &adapter.alias, "IPv4");
        set_name_server(&ipv6_path, ipv6_value, &adapter.alias, "IPv6");
    }
}

/// 区分注册表操作阶段, 让普通路径保留原有诊断。
enum NameServerError {
    Open(Error),
    Write(Error),
}

/// 设置注册表中的 `NameServer` 值, 不调用 tracing。
fn set_registry_name_server_without_logging(
    reg_path: &str,
    value: &str,
) -> Result<(), NameServerError> {
    let key = LOCAL_MACHINE
        .options()
        .read()
        .write()
        .open(reg_path)
        .map_err(NameServerError::Open)?;
    key.set_string("NameServer", value)
        .map_err(NameServerError::Write)
}

/// 设置注册表中的 `NameServer` 值, 保留普通操作的诊断。
fn set_registry_name_server(reg_path: &str, value: &str, alias: &str, protocol: &str) {
    match set_registry_name_server_without_logging(reg_path, value) {
        Ok(()) => {
            let display_value = if value.is_empty() {
                "DHCP 自动获取"
            } else {
                value
            };
            debug!("已设置 {alias} 的 {protocol} DNS 为 '{display_value}'");
        }
        Err(NameServerError::Open(e)) => {
            warn!("打开注册表键失败 ({alias} {protocol}): {e}");
        }
        Err(NameServerError::Write(e)) => {
            warn!("设置 {alias} 的 {protocol} DNS 失败: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只记录写入请求, 不枚举真实网卡或打开注册表。
    #[test]
    fn test_dns_values_and_protocol_order_with_fake_writer() {
        let adapters = [
            PhysicalAdapter {
                alias: "Ethernet".into(),
                guid: "{first}".into(),
            },
            PhysicalAdapter {
                alias: "Wi-Fi".into(),
                guid: "{second}".into(),
            },
        ];

        for (ipv4, ipv6) in [("", ""), ("127.0.0.1", "::1")] {
            let mut writes = Vec::new();
            set_adapters_dns(&adapters, ipv4, ipv6, |path, value, alias, protocol| {
                writes.push((
                    path.to_string(),
                    value.to_string(),
                    alias.to_string(),
                    protocol.to_string(),
                ));
            });

            let expected = [
                (
                    r"SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\{first}",
                    ipv4,
                    "Ethernet",
                    "IPv4",
                ),
                (
                    r"SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters\Interfaces\{first}",
                    ipv6,
                    "Ethernet",
                    "IPv6",
                ),
                (
                    r"SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\{second}",
                    ipv4,
                    "Wi-Fi",
                    "IPv4",
                ),
                (
                    r"SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters\Interfaces\{second}",
                    ipv6,
                    "Wi-Fi",
                    "IPv6",
                ),
            ];
            assert_eq!(writes.len(), expected.len(), "每张网卡应尝试两个协议");
            let paired_writes = writes.iter().zip(expected);
            for (actual, expected) in paired_writes {
                assert_eq!(
                    (
                        actual.0.as_str(),
                        actual.1.as_str(),
                        actual.2.as_str(),
                        actual.3.as_str()
                    ),
                    expected,
                    "DNS 写入路径、值、别名和协议顺序应保持不变"
                );
            }
        }
    }
}
