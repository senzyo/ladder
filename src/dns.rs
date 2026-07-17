//! 物理网卡 DNS 管理，防止 xray TUN 模式下的 DNS 泄漏。
//!
//! xray 使用 wintun.dll 时，Windows 的"多接口并发/回退"机制会导致 DNS 查询
//! 同时通过物理网卡发送，造成 DNS 泄漏。本模块在 xray TUN 模式运行期间，
//! 将物理网卡 DNS 设置为 127.0.0.1 和 ::1，停止后恢复为 DHCP 自动获取。
//!
//! 使用 `GetIfTable2` API 检测物理网卡，通过 `InterfaceAndOperStatusFlags.ConnectorPresent`
//! 标志判断，这与 `Get-NetAdapter -Physical` 使用相同的检测机制。

use tracing::{debug, info, warn};
use windows::Win32::Foundation::NO_ERROR;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2};
use windows_registry::LOCAL_MACHINE;

/// MIB_IF_ROW2_0 bitfield 中 ConnectorPresent 标志的位偏移。
/// 根据 Windows 文档：Bit 2 = ConnectorPresent。
const CONNECTOR_PRESENT_BIT: u8 = 1 << 2;

/// MIB_IF_ROW2_0 bitfield 中 HardwareInterface 标志的位偏移。
/// Bit 0 = HardwareInterface。
const HARDWARE_INTERFACE_BIT: u8 = 1 << 0;

/// 物理网卡信息。
struct PhysicalAdapter {
    /// 接口别名（如 "Ethernet", "Wi-Fi"），用于日志显示。
    alias: String,
    /// 接口 GUID，用于注册表路径。
    guid: String,
}

/// 获取物理网卡列表。
///
/// 使用 `GetIfTable2` API 枚举所有网络接口，筛选条件：
/// - `ConnectorPresent` = true（物理连接器存在）
/// - `HardwareInterface` = true（硬件接口）
/// - `AccessType` != `NET_IF_ACCESS_LOOPBACK`（非回环接口）
///
/// 这与 `Get-NetAdapter -Physical` 使用相同的检测机制。
fn get_physical_adapters() -> Vec<PhysicalAdapter> {
    let mut adapters = Vec::new();
    let mut table_ptr: *mut windows::Win32::NetworkManagement::IpHelper::MIB_IF_TABLE2 = std::ptr::null_mut();

    let result = unsafe { GetIfTable2(&mut table_ptr) };
    if result != NO_ERROR || table_ptr.is_null() {
        warn!("GetIfTable2 调用失败: {result:?}");
        return adapters;
    }

    let table = unsafe { &*table_ptr };
    let rows = std::ptr::slice_from_raw_parts(table.Table.as_ptr(), table.NumEntries as usize);
    let rows = unsafe { &*rows };

    for row in rows {
        let flags = row.InterfaceAndOperStatusFlags._bitfield;

        // 检查 ConnectorPresent 和 HardwareInterface 标志
        let connector_present = (flags & CONNECTOR_PRESENT_BIT) != 0;
        let hardware_interface = (flags & HARDWARE_INTERFACE_BIT) != 0;

        if !connector_present || !hardware_interface {
            continue;
        }

        // 排除回环接口（AccessType = 3 = NET_IF_ACCESS_LOOPBACK）
        // NET_IF_ACCESS_LOOPBACK 的值是 3
        if row.AccessType.0 == 3 {
            continue;
        }

        // 提取接口别名（UTF-16 字符串）
        let alias_end = row.Alias.iter().position(|&c| c == 0).unwrap_or(row.Alias.len());
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

        debug!("发现物理网卡: {alias} (GUID: {guid})");
        adapters.push(PhysicalAdapter { alias, guid });
    }

    unsafe { FreeMibTable(table_ptr as *const std::ffi::c_void) };

    adapters
}

/// 设置物理网卡 DNS 防止 xray TUN 模式下的 DNS 泄漏。
///
/// IPv4 DNS 设置为 `127.0.0.1`，IPv6 DNS 设置为 `::1`。
/// 通过修改注册表 `NameServer` 值实现，Windows 会优先使用此值作为静态 DNS。
///
/// 失败仅记录警告，不阻断调用流程。
pub fn set_physical_dns_to_local() {
    let adapters = get_physical_adapters();
    if adapters.is_empty() {
        debug!("未发现物理网卡，跳过 DNS 设置");
        return;
    }

    info!("设置物理网卡 DNS 为本地回环地址防止 DNS 泄漏...");
    for adapter in &adapters {
        let ipv4_path = format!(
            r"SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\{}",
            adapter.guid
        );
        let ipv6_path = format!(
            r"SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters\Interfaces\{}",
            adapter.guid
        );
        set_registry_name_server(&ipv4_path, "127.0.0.1", &adapter.alias, "IPv4");
        set_registry_name_server(&ipv6_path, "::1", &adapter.alias, "IPv6");
    }
}

/// 恢复物理网卡 DNS 为 DHCP 自动获取。
///
/// 将 IPv4 和 IPv6 的 `NameServer` 设置为空字符串，Windows 会回退到
/// `DhcpNameServer`（由 DHCP 客户端服务维护）。
///
/// 失败仅记录警告，不阻断调用流程。
pub fn restore_dns_to_dhcp() {
    let adapters = get_physical_adapters();
    if adapters.is_empty() {
        debug!("未发现物理网卡，跳过 DNS 恢复");
        return;
    }

    info!("恢复物理网卡 DNS 为 DHCP 自动获取...");
    for adapter in &adapters {
        let ipv4_path = format!(
            r"SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\{}",
            adapter.guid
        );
        let ipv6_path = format!(
            r"SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters\Interfaces\{}",
            adapter.guid
        );
        set_registry_name_server(&ipv4_path, "", &adapter.alias, "IPv4");
        set_registry_name_server(&ipv6_path, "", &adapter.alias, "IPv6");
    }
}

/// 设置注册表中的 `NameServer` 值。
fn set_registry_name_server(reg_path: &str, value: &str, alias: &str, protocol: &str) {
    match LOCAL_MACHINE.options().read().write().open(reg_path) {
        Ok(key) => {
            if let Err(e) = key.set_string("NameServer", value) {
                warn!("设置 {alias} 的 {protocol} DNS 失败: {e}");
            } else {
                let display_value = if value.is_empty() { "DHCP 自动获取" } else { value };
                debug!("已设置 {alias} 的 {protocol} DNS 为 '{display_value}'");
            }
        }
        Err(e) => {
            warn!("打开注册表键失败 ({alias} {protocol}): {e}");
        }
    }
}
