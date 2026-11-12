//! 按物理网卡和地址族保存静态/自动 DNS，再接管到本地回环。

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use windows_registry::LOCAL_MACHINE;
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_NOT_FOUND, ERROR_NOT_SUPPORTED,
    ERROR_PATH_NOT_FOUND, NO_ERROR,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    DNS_INTERFACE_SETTINGS, DNS_INTERFACE_SETTINGS_VERSION1, DNS_SETTING_IPV6,
    DNS_SETTING_NAMESERVER, FreeMibTable, GetIfTable2, GetIpInterfaceEntry,
    InitializeIpInterfaceEntry, MIB_IF_ROW2, MIB_IF_TABLE2, MIB_IPINTERFACE_ROW,
    SetInterfaceDnsSettings,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_IF_ACCESS_LOOPBACK;
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows_sys::core::GUID;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
struct Target {
    guid: u128,
    ipv6: bool,
}

impl Target {
    fn loopback(self) -> &'static str {
        if self.ipv6 { "::1" } else { "127.0.0.1" }
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

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SavedDns {
    target: Target,
    // 只读取 NameServer (静态覆盖), 空字符串表示自动, 不保存 DHCP 分配的临时地址。
    previous: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Journal {
    changes: Vec<SavedDns>,
    #[serde(default)]
    imports: Vec<LegacyImport>,
}

#[derive(Clone, Serialize, Deserialize)]
struct LegacyImport {
    source: PathBuf,
    state: ImportState,
}

#[derive(Clone, Serialize, Deserialize)]
enum ImportState {
    // 删除便携文件之前落盘；SYSTEM 完成恢复后仍保留这张收据。
    Imported(Vec<SavedDns>),
    // 旧文件暂不可读。即使稍后修好，也不能覆盖机器记录已交还的网卡。
    Superseded(Vec<Target>),
}

impl Journal {
    fn load(path: &Path) -> Result<Option<Self>, String> {
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("无法读取 DNS 恢复记录 {}: {e}", path.display())),
        };
        // 直接按具体类型反序列化，保留旧数组中完整的 128 位网卡 GUID。
        let result = if data.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'[') {
            serde_json::from_slice(&data).map(|changes| Self {
                changes,
                ..Self::default()
            })
        } else {
            serde_json::from_slice(&data)
        };
        result.map(Some).map_err(|e| format!(
            "{} 格式无效（第 {} 行，第 {} 列）。请用备份修复记录，或在手动恢复网卡 DNS 后移走此文件。",
            path.display(), e.line(), e.column()
        ))
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        let data = if self.imports.is_empty() {
            serde_json::to_vec(&self.changes)
        } else {
            serde_json::to_vec(self)
        }
        .map_err(|e| e.to_string())?;
        let staged = path.with_extension("tmp");
        let save = || -> io::Result<()> {
            let mut file = fs::File::create(&staged)?;
            file.write_all(&data)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&staged, path)
        };
        save().map_err(|e| format!("无法保存 DNS 恢复记录 {}: {e}", path.display()))
    }

    fn import_legacy(&mut self, source: &Path, machine: &Path) -> Result<bool, String> {
        let prior = self.imports.iter().position(|entry| entry.source == source);
        let old = match Self::load(source) {
            Ok(None) => {
                if let Some(index) = prior {
                    self.imports.remove(index);
                }
                return Ok(prior.is_some());
            }
            Ok(Some(old)) => old.changes,
            Err(error) => {
                let protected = match prior.map(|i| &self.imports[i].state) {
                    Some(ImportState::Superseded(targets)) => targets.as_slice(),
                    _ => &[],
                };
                if self
                    .changes
                    .iter()
                    .any(|change| !protected.contains(&change.target))
                {
                    let mut targets = match prior.map(|i| self.imports.remove(i).state) {
                        Some(ImportState::Superseded(targets)) => targets,
                        Some(ImportState::Imported(old)) => old.iter().map(|c| c.target).collect(),
                        None => Vec::new(),
                    };
                    for change in &self.changes {
                        if !targets.contains(&change.target) {
                            targets.push(change.target);
                        }
                    }
                    self.imports.push(LegacyImport {
                        source: source.to_owned(),
                        state: ImportState::Superseded(targets),
                    });
                    // 即便本次所有网卡都写失败，下一次开机也能保留机器记录的优先级。
                    self.save(machine).map_err(|e| format!("{error}；{e}"))?;
                }
                return Err(error);
            }
        };
        let receipt = prior.map(|i| &self.imports[i].state);
        if !matches!(receipt, Some(ImportState::Imported(entries)) if *entries == old) {
            for change in &old {
                if !self
                    .changes
                    .iter()
                    .any(|newer| newer.target == change.target)
                    && !matches!(receipt, Some(ImportState::Superseded(targets))
                        if targets.contains(&change.target))
                {
                    self.changes.push(change.clone());
                }
            }
            if let Some(index) = prior {
                self.imports.remove(index);
            }
            self.imports.push(LegacyImport {
                source: source.to_owned(),
                state: ImportState::Imported(old),
            });
            // 即使复制成功后立即断电，开机助手也知道哪些网卡仍需恢复。
            self.save(machine)?;
        }
        fs::remove_file(source)
            .map_err(|e| format!("无法清除旧 DNS 恢复记录 {}: {e}", source.display()))?;
        self.imports.retain(|entry| entry.source != source);
        Ok(true)
    }

    fn restore(
        mut self,
        path: &Path,
        write: &impl Fn(Target, &str) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut errors = Vec::new();
        let mut checkpoint_error = None;
        let mut index = 0;
        while index < self.changes.len() {
            let change = &self.changes[index];
            match write(change.target, &change.previous) {
                Ok(()) => {
                    self.changes.remove(index);
                    if !self.changes.is_empty() {
                        // 恢复一个协议就交还一个，后续写盘成功可补上先前失败的检查点。
                        checkpoint_error = self.save(path).err();
                    }
                }
                Err(error) => {
                    errors.push(error);
                    index += 1;
                }
            }
        }
        if !errors.is_empty() {
            errors.extend(checkpoint_error);
            return Err(format!("恢复 DNS 失败: {}", errors.join("；")));
        }
        if !self.imports.is_empty() {
            return self.save(path);
        }
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("无法清除 DNS 恢复记录 {}: {e}", path.display())),
        }
    }
}

pub struct DnsLease {
    journal: PathBuf,
    legacy: Option<PathBuf>,
    #[cfg(test)]
    simulated: Option<std::cell::RefCell<std::collections::HashMap<Target, String>>>,
}

impl DnsLease {
    pub fn new(base: &Path) -> Self {
        Self {
            journal: base.join("dns-recovery.json"),
            legacy: None,
            #[cfg(test)]
            simulated: None,
        }
    }

    pub fn with_legacy(machine_dir: &Path, portable_dir: &Path) -> Self {
        let mut lease = Self::new(machine_dir);
        if machine_dir != portable_dir {
            lease.legacy = Some(portable_dir.join("dns-recovery.json"));
        }
        lease
    }

    #[cfg(test)]
    pub fn for_tests(base: &Path) -> Self {
        let mut lease = Self::new(base);
        lease.simulated = Some(std::cell::RefCell::new(std::collections::HashMap::from([
            (
                Target {
                    guid: u128::MAX,
                    ipv6: false,
                },
                "192.0.2.1".to_owned(),
            ),
            (
                Target {
                    guid: u128::MAX,
                    ipv6: true,
                },
                String::new(),
            ),
        ])));
        lease
    }

    pub fn apply(&self) -> Result<(), String> {
        #[cfg(test)]
        if let Some(state) = &self.simulated {
            let targets = state.borrow().keys().copied().collect();
            return self.install(
                targets,
                |target| Ok(state.borrow()[&target].clone()),
                |target, value| {
                    state.borrow_mut().insert(target, value.into());
                    Ok(())
                },
            );
        }
        self.install(physical_targets()?, read_static, |target, value| {
            set_static(target, value).map_err(write_error)
        })
    }

    pub fn restore(&self) -> Result<(), String> {
        self.restore_and_flush(true)
    }

    pub fn restore_if_pending(&self) -> Result<(), String> {
        self.restore_and_flush(false)
    }

    fn restore_and_flush(&self, always_flush: bool) -> Result<(), String> {
        #[cfg(test)]
        if let Some(state) = &self.simulated {
            return self
                .restore_with(|target, value| {
                    if let Some(current) = state.borrow_mut().get_mut(&target) {
                        *current = value.into();
                    }
                    Ok(())
                })
                .map(|_| ());
        }
        let result = self.restore_with(restore_static);
        if !always_flush && matches!(result, Ok(false)) {
            return Ok(());
        }
        // SAFETY: dnsapi.dll 的无参数同步调用，刷新本机 DNS 客户端缓存。
        if unsafe { DnsFlushResolverCache() } != 0 {
            result.map(|_| ())
        } else {
            let message = "刷新 DNS 客户端缓存失败";
            Err(match result {
                Ok(_) => message.into(),
                Err(error) => format!("{error}；{message}"),
            })
        }
    }

    fn install(
        &self,
        targets: Vec<Target>,
        read: impl Fn(Target) -> Result<String, String>,
        write: impl Fn(Target, &str) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut journal = Journal::load(&self.journal)?.unwrap_or_default();
        for target in targets {
            if !journal.changes.iter().any(|change| change.target == target) {
                journal.changes.push(SavedDns {
                    target,
                    previous: read(target)?,
                });
                // 第二个核心也会接管 DNS，但每张网卡只登记首次接管前的原值。
                journal.save(&self.journal)?;
            }
            write(target, target.loopback())?;
        }
        Ok(())
    }

    fn legacy_source(&self) -> Result<Option<PathBuf>, String> {
        let Some(path) = self.legacy.as_deref() else {
            return Ok(None);
        };
        let portable = path.parent().unwrap();
        // 目录可能使用大小写、短路径或连接点别名；同一目录不需要迁移。
        let resolve = || -> io::Result<Option<PathBuf>> {
            if same_file::is_same_file(self.journal.parent().unwrap(), portable)? {
                return Ok(None);
            }
            Ok(Some(fs::canonicalize(portable)?.join("dns-recovery.json")))
        };
        resolve().map_err(|e| format!("无法核对 DNS 恢复记录 {} 的目录: {e}", path.display()))
    }

    fn restore_with(
        &self,
        write: impl Fn(Target, &str) -> Result<(), String>,
    ) -> Result<bool, String> {
        let mut errors = Vec::new();
        let mut pending = false;
        let legacy = self.legacy_source().unwrap_or_else(|e| {
            errors.push(e);
            None
        });
        match Journal::load(&self.journal) {
            Ok(machine) => {
                let exists = machine.is_some();
                let mut journal = machine.unwrap_or_default();
                let mut imported = false;
                if let Some(source) = legacy {
                    match journal.import_legacy(&source, &self.journal) {
                        Ok(changed) => imported = changed,
                        Err(e) => errors.push(e),
                    }
                }
                pending = !journal.changes.is_empty();
                if (imported || pending || (exists && journal.imports.is_empty()))
                    && let Err(e) = journal.restore(&self.journal, &write)
                {
                    errors.push(e);
                }
            }
            Err(e) => {
                errors.push(e);
                // 保留损坏的机器记录，同时尽量就地恢复可读的便携记录。
                if let Some(source) = legacy {
                    match Journal::load(&source) {
                        Ok(Some(old)) => {
                            pending = !old.changes.is_empty();
                            if let Err(e) = old.restore(&source, &write) {
                                errors.push(e);
                            }
                        }
                        Ok(None) => {}
                        Err(e) => errors.push(e),
                    }
                }
            }
        }
        if errors.is_empty() {
            Ok(pending)
        } else {
            Err(errors.join("；"))
        }
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
            row.AccessType != NET_IF_ACCESS_LOOPBACK
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
    let key = LOCAL_MACHINE
        .options()
        .read()
        .open(target.registry_path())
        .map_err(|e| format!("无法读取网卡，请检查网卡是否可用: {e}"))?;
    match key.get_string("NameServer") {
        Ok(value) => Ok(value),
        // 未配置静态 DNS 的接口可能没有 NameServer 值。
        Err(e) if e.code().0 as u32 == (0x8007_0000 | ERROR_FILE_NOT_FOUND) => Ok(String::new()),
        Err(e) => Err(format!("无法读取网卡 DNS 设置: {e}")),
    }
}

fn set_static(target: Target, servers: &str) -> Result<(), u32> {
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
    if status == NO_ERROR {
        Ok(())
    } else {
        Err(status)
    }
}

fn write_error(status: u32) -> String {
    match status {
        ERROR_ACCESS_DENIED => "Windows 拒绝设置网卡 DNS，请检查网卡权限或组织策略".into(),
        _ => format!("设置网卡 DNS 失败: {}", os_error(status)),
    }
}

fn restore_static(target: Target, previous: &str) -> Result<(), String> {
    let key = LOCAL_MACHINE
        .options()
        .read()
        .write()
        .open(target.registry_path());
    restore_existing(key, previous, || set_static(target, previous))
}

fn restore_existing(
    key: windows_registry::Result<windows_registry::Key>,
    previous: &str,
    set: impl FnOnce() -> Result<(), u32>,
) -> Result<(), String> {
    // 开机早期、禁用或暂时拔掉的网卡可能没有活动 IP 接口，但其持久设置仍在注册表。
    // 整个协议配置键都不存在时才把目标视为已移除。
    let key = match key {
        Ok(key) => key,
        Err(e)
            if [ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND]
                .iter()
                .any(|&code| e.code().0 as u32 == (0x8007_0000 | code)) =>
        {
            return Ok(());
        }
        Err(e) => return Err(format!("无法打开待恢复的网卡 DNS 设置: {e}")),
    };
    match set() {
        Ok(()) => Ok(()),
        Err(
            ERROR_NOT_FOUND | ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_NOT_SUPPORTED,
        ) => key
            .set_string("NameServer", previous)
            .map_err(|e| format!("无法恢复离线网卡 DNS: {e}")),
        Err(status) => Err(write_error(status)),
    }
}

#[link(name = "dnsapi")]
unsafe extern "system" {
    fn DnsFlushResolverCache() -> windows_sys::core::BOOL;
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

    fn save_changes(path: &Path, changes: &[SavedDns]) -> io::Result<()> {
        fs::write(path, serde_json::to_vec(changes)?)
    }

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

        // 已完成的协议不再重试，尚未恢复的协议覆盖期间的手动设置。
        let restarted = DnsLease::new(dir.path());
        write(V6, "2001:db8::9").unwrap();
        let failure = restarted.restore_with(|target, value| {
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
        restarted.restore_with(write).unwrap();
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
        let write = |target, value: &str| {
            state.borrow_mut().insert(target, value.to_owned());
            Ok(())
        };
        let result = lease.install(vec![V4, V6], read, |target, value| {
            assert!(lease.journal.exists());
            if target == V6 {
                return Err("写入失败".into());
            }
            write(target, value)
        });
        assert!(result.is_err());
        // 核心停止后统一回滚已登记的目标。
        lease.restore_with(write).unwrap();
        assert_eq!(read(V4).unwrap(), "");
        assert_eq!(read(V6).unwrap(), "2001:db8::1");
        assert!(!lease.journal.exists());
    }

    #[test]
    fn interrupted_install_never_claims_an_interface_it_did_not_reach() {
        let dir = tempfile::tempdir().unwrap();
        let lease = DnsLease::new(dir.path());
        let state = RefCell::new(HashMap::from([(V4, String::new()), (V6, String::new())]));
        let read = |target| Ok(state.borrow()[&target].clone());
        let write = |target, value: &str| {
            state.borrow_mut().insert(target, value.to_owned());
            Ok(())
        };
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            lease.install(vec![V4, V6], read, |target, value| {
                assert_eq!(target, V4);
                write(target, value)?;
                panic!("模拟写完 IPv4 后进程退出");
            })
        }));
        assert!(interrupted.is_err());
        write(V6, "::1").unwrap();
        DnsLease::new(dir.path()).restore_with(write).unwrap();
        assert_eq!(read(V4).unwrap(), "");
        assert_eq!(read(V6).unwrap(), "::1");
    }

    #[test]
    fn interrupted_restore_only_overwrites_edits_on_pending_interfaces() {
        let dir = tempfile::tempdir().unwrap();
        let lease = DnsLease::new(dir.path());
        let state = RefCell::new(HashMap::from([(V4, String::new()), (V6, String::new())]));
        let read = |target| Ok(state.borrow()[&target].clone());
        let write = |target, value: &str| {
            state.borrow_mut().insert(target, value.to_owned());
            Ok(())
        };
        lease.install(vec![V4, V6], read, write).unwrap();
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            lease.restore_with(|target, value| {
                if target == V6 {
                    panic!("模拟恢复 IPv6 前进程退出");
                }
                write(target, value)
            })
        }));
        assert!(interrupted.is_err());
        assert_eq!(read(V4).unwrap(), "");
        write(V4, "127.0.0.1").unwrap();
        write(V6, "2001:db8::2").unwrap();
        DnsLease::new(dir.path()).restore_with(write).unwrap();
        assert_eq!(read(V4).unwrap(), "127.0.0.1");
        assert_eq!(read(V6).unwrap(), "");
        assert!(!lease.journal.exists());
    }

    #[test]
    fn temporary_journal_write_failure_still_restores_the_other_adapter() {
        let dir = tempfile::tempdir().unwrap();
        let lease = DnsLease::new(dir.path());
        let state = RefCell::new(HashMap::from([(V4, String::new()), (V6, String::new())]));
        let read = |target| Ok(state.borrow()[&target].clone());
        let write = |target, value: &str| {
            state.borrow_mut().insert(target, value.to_owned());
            Ok(())
        };
        lease.install(vec![V4, V6], read, write).unwrap();
        fs::create_dir(lease.journal.with_extension("tmp")).unwrap();
        lease.restore_with(write).unwrap();
        assert!(read(V4).unwrap().is_empty());
        assert!(read(V6).unwrap().is_empty());
        assert!(!lease.journal.exists());
    }

    #[test]
    fn padded_registry_values_preserve_static_auto_and_loopback_settings() {
        for (text, expected) in [
            ("127.0.0.1\0\0\0", "127.0.0.1"),
            ("0:0:0:0:0:0:0:1\0\0", "0:0:0:0:0:0:0:1"),
            ("\0\0", ""),
            ("192.0.2.1,192.0.2.2\0\0", "192.0.2.1,192.0.2.2"),
        ] {
            // 使用与 Key::get_string 相同的 REG_SZ 转换，无需写入系统注册表。
            let value = windows_registry::Value::from(text);
            assert!(value.as_wide().ends_with(&[0, 0]));
            let current = String::try_from(value).unwrap();
            assert_eq!(current, expected);
        }
    }

    #[test]
    fn repeated_apply_tracks_new_targets_and_keeps_original_loopback() {
        let dir = tempfile::tempdir().unwrap();
        let lease = DnsLease::new(dir.path());
        let state = RefCell::new(HashMap::from([
            (V4, "127.0.0.1".into()),
            (V6, String::new()),
        ]));
        let read = |target| Ok(state.borrow()[&target].clone());
        let write = |target, value: &str| {
            state.borrow_mut().insert(target, value.into());
            Ok(())
        };
        lease.restore_with(|_, _| panic!("无需写入网卡")).unwrap();
        lease.install(vec![V4], read, write).unwrap();
        write(V4, "192.0.2.100").unwrap();
        lease.install(vec![V4, V6], read, write).unwrap();
        assert_eq!(read(V4).unwrap(), "127.0.0.1");
        assert_eq!(read(V6).unwrap(), "::1");
        write(V4, "192.0.2.100").unwrap();
        lease.restore_with(write).unwrap();
        assert_eq!(read(V4).unwrap(), "127.0.0.1");
        assert_eq!(read(V6).unwrap(), "");
        assert!(!lease.journal.exists());
    }

    #[test]
    fn migrates_portable_snapshot_without_overwriting_newer_machine_entries() {
        let portable = tempfile::tempdir().unwrap();
        let machine = tempfile::tempdir().unwrap();
        let old = DnsLease::new(portable.path());
        save_changes(
            &old.journal,
            &[
                SavedDns {
                    target: V4,
                    previous: "192.0.2.1".into(),
                },
                SavedDns {
                    target: V6,
                    previous: "2001:db8::1".into(),
                },
            ],
        )
        .unwrap();
        let lease = DnsLease::with_legacy(machine.path(), portable.path());
        save_changes(
            &lease.journal,
            &[SavedDns {
                target: V4,
                previous: "192.0.2.2".into(),
            }],
        )
        .unwrap();
        let restored = RefCell::new(HashMap::new());
        let partial = lease.restore_with(|target, value| {
            if target == V6 {
                return Err("暂时无法写入".into());
            }
            restored.borrow_mut().insert(target, value.to_owned());
            Ok(())
        });
        assert!(partial.is_err());
        assert!(!old.journal.exists());
        assert_eq!(restored.borrow()[&V4], "192.0.2.2");
        // SYSTEM 下次开机只读取机器记录即可恢复剩余项。
        DnsLease::new(machine.path())
            .restore_with(|target, value| {
                assert_eq!(target, V6);
                assert_eq!(value, "2001:db8::1");
                Ok(())
            })
            .unwrap();
        assert!(!lease.journal.exists());
    }

    #[test]
    fn a_damaged_journal_never_blocks_restoring_the_other_copy() {
        for broken_machine in [false, true] {
            let portable = tempfile::tempdir().unwrap();
            let machine = tempfile::tempdir().unwrap();
            let lease = DnsLease::with_legacy(machine.path(), portable.path());
            let legacy = lease.legacy.as_ref().unwrap();
            let (broken, good) = if broken_machine {
                (&lease.journal, legacy)
            } else {
                (legacy, &lease.journal)
            };
            fs::write(broken, b"invalid").unwrap();
            save_changes(
                good,
                &[SavedDns {
                    target: V4,
                    previous: "192.0.2.1".into(),
                }],
            )
            .unwrap();
            let restored = RefCell::new(Vec::new());
            let error = lease
                .restore_with(|target, value| {
                    restored.borrow_mut().push((target, value.to_owned()));
                    Ok(())
                })
                .unwrap_err();
            assert!(error.contains(&broken.display().to_string()));
            assert_eq!(*restored.borrow(), [(V4, "192.0.2.1".into())]);
            assert_eq!(fs::read(broken).unwrap(), b"invalid");
        }
    }

    #[test]
    fn portable_alias_cannot_remove_the_machine_snapshot_before_restore_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("child");
        fs::create_dir(&child).unwrap();
        let alias = child.join("..");
        let lease = DnsLease::with_legacy(dir.path(), &alias);
        lease
            .install(vec![V4], |_| Ok("192.0.2.1".into()), |_, _| Ok(()))
            .unwrap();
        assert!(
            lease
                .restore_with(|_, _| Err("暂时无法写入".into()))
                .is_err()
        );
        assert!(lease.journal.is_file());
        lease
            .restore_with(|target, value| {
                assert_eq!((target, value), (V4, "192.0.2.1"));
                Ok(())
            })
            .unwrap();
        assert!(!lease.journal.exists());
    }

    #[test]
    fn removed_adapter_is_released_while_the_remaining_adapter_is_restored() {
        let dir = tempfile::tempdir().unwrap();
        let lease = DnsLease::new(dir.path());
        lease
            .install(vec![V4, V6], |_| Ok(String::new()), |_, _| Ok(()))
            .unwrap();
        let restored = RefCell::new(Vec::new());
        lease
            .restore_with(|target, previous| {
                if target == V4 {
                    // 对无关的不存在路径做只读打开，取得 Windows 的“配置键不存在”错误。
                    let path = format!(
                        r"Software\LadderTest\{}",
                        dir.path().file_name().unwrap().to_string_lossy()
                    );
                    let missing = windows_registry::CURRENT_USER.options().read().open(path);
                    restore_existing(missing, previous, || panic!("不应写入已卸载网卡"))
                } else {
                    restored.borrow_mut().push(target);
                    Ok(())
                }
            })
            .unwrap();
        assert_eq!(*restored.borrow(), [V6]);
        assert!(!lease.journal.exists());
    }

    #[test]
    fn locked_portable_journal_keeps_newer_values_until_cleanup_can_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        let portable = tempfile::tempdir().unwrap();
        let machine = tempfile::tempdir().unwrap();
        let lease = DnsLease::with_legacy(machine.path(), portable.path());
        let old = lease.legacy.as_ref().unwrap();
        for (path, previous) in [(old, "192.0.2.1"), (&lease.journal, "192.0.2.2")] {
            save_changes(
                path,
                &[SavedDns {
                    target: V4,
                    previous: previous.into(),
                }],
            )
            .unwrap();
        }
        // 允许读取，禁止删除，模拟旧记录被其他程序占用。
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(old)
            .unwrap();
        let boot = DnsLease::new(machine.path());
        assert!(
            lease
                .restore_with(|target, value| {
                    assert_eq!((target, value), (V4, "192.0.2.2"));
                    Err("首次恢复失败".into())
                })
                .unwrap_err()
                .contains("旧 DNS 恢复记录")
        );
        drop(held);
        assert!(
            boot.restore_with(|target, value| {
                assert_eq!((target, value), (V4, "192.0.2.2"));
                Ok(())
            })
            .unwrap()
        );
        assert!(boot.journal.exists());
        assert!(
            !boot
                .restore_with(|_, _| panic!("开机任务不得重放成功项"))
                .unwrap()
        );
        assert!(
            !lease
                .restore_with(|_, _| panic!("桌面不得重放旧快照"))
                .unwrap()
        );
        assert!(!old.exists());
        assert!(!lease.journal.exists());
    }

    #[test]
    fn damaged_portable_journal_releases_machine_targets_before_repair() {
        for gui_write_fails in [false, true] {
            let portable = tempfile::tempdir().unwrap();
            let machine = tempfile::tempdir().unwrap();
            let lease = DnsLease::with_legacy(machine.path(), portable.path());
            let old = lease.legacy.as_ref().unwrap();
            fs::write(old, b"invalid").unwrap();
            let saved = |target, value: &str| SavedDns {
                target,
                previous: value.into(),
            };
            save_changes(&lease.journal, &[saved(V4, "192.0.2.2")]).unwrap();
            let writes = RefCell::new(Vec::new());
            let write = |target, previous: &str| {
                writes.borrow_mut().push((target, previous.to_owned()));
                Ok(())
            };
            assert!(
                lease
                    .restore_with(|target, previous| if gui_write_fails {
                        Err("暂时无法写入".into())
                    } else {
                        write(target, previous)
                    })
                    .is_err()
            );
            let boot = DnsLease::new(machine.path());
            assert_eq!(boot.restore_with(write).unwrap(), gui_write_fails);
            assert_eq!(*writes.borrow(), [(V4, "192.0.2.2".to_owned())]);
            assert!(
                lease
                    .restore_with(|_, _| panic!("退出时不得覆盖用户手改"))
                    .is_err()
            );
            assert!(
                !boot
                    .restore_with(|_, _| panic!("开机不得覆盖用户手改"))
                    .unwrap()
            );
            save_changes(old, &[saved(V4, "192.0.2.1"), saved(V6, "")]).unwrap();
            lease.restore_with(write).unwrap();
            assert_eq!(writes.borrow()[1..], [(V6, String::new())]);
            assert!(!lease.journal.exists());
            assert!(!old.exists());
        }
    }

    #[test]
    fn empty_portable_journal_leaves_no_receipt() {
        let portable = tempfile::tempdir().unwrap();
        let machine = tempfile::tempdir().unwrap();
        let lease = DnsLease::with_legacy(machine.path(), portable.path());
        let old = lease.legacy.as_ref().unwrap();
        fs::write(old, b"[]").unwrap();
        assert!(
            !lease
                .restore_with(|_, _| panic!("空快照没有待恢复项"))
                .unwrap()
        );
        assert!(!old.exists());
        assert!(!lease.journal.exists());
    }
}
