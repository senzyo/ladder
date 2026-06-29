//! 子进程生命周期管理、TUN 接口配置与 DNS 缓存刷新。
//!
//! 负责 sing-box / xray 子进程的启停控制、TUN 接口名随机化、
//! 孤立 WinTUN 设备节点清理、以及 DNS 缓存刷新。

use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::ptr::null;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

use serde_json::Value;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{CREATE_NO_WINDOW, OpenProcess, PROCESS_TERMINATE, TerminateProcess};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_Status, CM_Get_Device_ID_List_SizeW, CM_Get_Device_ID_ListW, CM_Locate_DevNodeW, CR_SUCCESS,
    DN_HAS_PROBLEM, DN_STARTED,
};

use crate::error::AppError;
use crate::state::{self};

// dnsapi.dll 导入，用于刷新系统 DNS 缓存。
#[link(name = "dnsapi")]
unsafe extern "system" {
    fn DnsFlushResolverCache() -> i32;
}

/// 创建不显示控制台窗口的子进程 Command。
pub fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW.0);
    command
}

// ═══════════════════════════════════════════════
// 进程启动
// ═══════════════════════════════════════════════

/// 将子进程的 stderr 重定向到日志输出。
fn forward_stderr(child: &mut std::process::Child, label: &str) {
    if let Some(stderr) = child.stderr.take() {
        let label = label.to_owned();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                warn!("[{label}] {line}");
            }
        });
    }
}

/// 启动 sing-box 子进程。启动前清理孤立 WinTUN 设备并随机化 TUN 接口名。
pub fn start_sing_box_at(exe_dir: &Path) -> Result<(), AppError> {
    cleanup_orphaned_wintun();

    let exe = exe_dir.join("sing-box_core").join("sing-box.exe");
    let config = exe_dir.join("configs").join("sing-box.json");

    state::ensure_exists(&exe)?;
    state::ensure_exists(&config)?;
    randomize_tun_name(&config)?;

    info!("启动 sing-box");
    let mut child = hidden_command(exe)
        .args(["run", "-D"])
        .arg(exe_dir.join("sing-box_core"))
        .arg("-c")
        .arg(config)
        .current_dir(exe_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::Msg(format!("启动 sing-box 失败: {e}")))?;
    forward_stderr(&mut child, "sing-box");
    if let Some(mut app) = state::app_state_mut() {
        app.child_sing_box = Some(child);
    }

    Ok(())
}

/// 启动 xray 子进程，stderr 输出重定向到日志。
pub fn start_xray_at(exe_dir: &Path) -> Result<(), AppError> {
    let exe = exe_dir.join("xray_core").join("xray.exe");
    let config = exe_dir.join("configs").join("xray.json");

    state::ensure_exists(&exe)?;
    state::ensure_exists(&config)?;

    info!("启动 xray");
    let mut child = hidden_command(exe)
        .args(["run", "-c"])
        .arg(config)
        .current_dir(exe_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::Msg(format!("启动 xray 失败: {e}")))?;
    forward_stderr(&mut child, "xray");
    if let Some(mut app) = state::app_state_mut() {
        app.child_xray = Some(child);
    }

    Ok(())
}

// ═══════════════════════════════════════════════
// 进程停止
// ═══════════════════════════════════════════════

/// 终止所有已知子进程并刷新 DNS。
pub fn stop_all() -> Result<(), AppError> {
    stop_processes(&["sing-box.exe", "xray.exe"])
}

/// 终止指定进程列表。先通过保存的 Child 句柄直接 kill，
/// 再通过进程名枚举兜底（覆盖其他来源启动的同名进程）。
pub fn stop_processes(processes: &[&str]) -> Result<(), AppError> {
    // 阶段 1：从 mutex 中取出 Child 句柄（持锁时间极短）
    let children: Vec<(String, Option<std::process::Child>)> = match state::app_state_mut() {
        Some(mut app) => processes
            .iter()
            .map(|&p| {
                let child = match p {
                    "sing-box.exe" => app.child_sing_box.take(),
                    "xray.exe" => app.child_xray.take(),
                    _ => None,
                };
                (p.to_string(), child)
            })
            .collect(),
        None => processes.iter().map(|&p| (p.to_string(), None)).collect(),
    };
    // 阶段 2：kill（锁已释放，不阻塞其他线程）
    for (name, child) in children {
        if let Some(mut c) = child {
            info!("终止子进程: {name}");
            let _ = c.kill();
        }
    }
    // 阶段 3：通过进程名枚举兜底（覆盖其他来源启动的同名进程）
    for process in processes {
        info!("终止进程: {process}");
        kill_processes_by_name(process);
    }
    flush_dns();
    Ok(())
}

/// 终止与 `exe_name` 匹配的所有进程。
fn kill_processes_by_name(exe_name: &str) {
    for pid in state::find_pids_by_name(exe_name) {
        unsafe {
            if let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                debug!("终止进程: {} (PID {})", exe_name, pid);
                let _ = TerminateProcess(handle, 1);
                let _ = CloseHandle(handle);
            }
        }
    }
}

// ═══════════════════════════════════════════════
// 重启
// ═══════════════════════════════════════════════

pub fn restart_all_at(exe_dir: &Path) -> Result<(), AppError> {
    stop_all()?;
    start_sing_box_at(exe_dir)?;
    start_xray_at(exe_dir)
}

pub fn restart_sing_box_at(exe_dir: &Path) -> Result<(), AppError> {
    stop_processes(&["sing-box.exe"])?;
    start_sing_box_at(exe_dir)
}

pub fn restart_xray_at(exe_dir: &Path) -> Result<(), AppError> {
    stop_processes(&["xray.exe"])?;
    start_xray_at(exe_dir)
}

// ═══════════════════════════════════════════════
// TUN 管理
// ═══════════════════════════════════════════════

/// 随机化 sing-box 配置中的 TUN 接口名。
///
/// sing-box TUN 适配器在 Windows 上以固定名称注册，重启时如果旧适配器
/// 未完全释放会导致冲突。通过每次启动时生成随机 6 位十六进制名称来避免。
///
/// 使用字符串替换而非 JSON 序列化来保留原始配置的格式和注释。
/// 如果配置中没有 type=tun 的 inbound，静默跳过（用户可能不使用 TUN 功能）。
/// 如果 tun inbound 缺少 interface_name 字段，自动写入随机化后的值。
fn randomize_tun_name(config_path: &Path) -> Result<(), AppError> {
    let text = fs::read_to_string(config_path).map_err(|e| AppError::Msg(format!("读取 sing-box 配置失败: {e}")))?;
    let json: Value = serde_json::from_str(&text).map_err(|e| AppError::Msg(format!("解析 sing-box 配置失败: {e}")))?;
    let new_name = random_hex_name();

    // 找 tun inbound，找不到直接跳过
    let tun_inbound = json
        .get("inbounds")
        .and_then(Value::as_array)
        .and_then(|inbounds| {
            inbounds.iter().find(|inbound| inbound.get("type").and_then(Value::as_str) == Some("tun"))
        });

    let tun_inbound = match tun_inbound {
        Some(inbound) => inbound,
        None => {
            debug!("未发现 type=tun 的 inbound，跳过 TUN 接口名随机化");
            return Ok(());
        }
    };

    // 有 interface_name → 替换
    if let Some(old_name) = tun_inbound.get("interface_name").and_then(Value::as_str) {
        if old_name == new_name {
            return Ok(());
        }
        debug!("随机化 TUN 接口名: {old_name} -> {new_name}");
        let old_pattern = format!("\"interface_name\": \"{}\"", old_name);
        let new_pattern = format!("\"interface_name\": \"{}\"", new_name);
        let new_text = text.replacen(&old_pattern, &new_pattern, 1);
        return fs::write(config_path, new_text).map_err(|e| AppError::Msg(format!("写入 sing-box 配置失败: {e}")));
    }

    // 无 interface_name → 在 "type": "tun" 行后插入
    debug!("写入 TUN 接口名: {new_name}");
    let type_pattern = "\"type\": \"tun\"";
    let pos = text.find(type_pattern).ok_or(AppError::Msg("未在 sing-box.json 中找到 type=tun 的 inbound".into()))?;
    let line_end = text[pos..].find('\n').map(|p| pos + p).unwrap_or(text.len());
    let line_start = text[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let indent = &text[line_start..pos];
    let insert = format!("{indent}\"interface_name\": \"{new_name}\",\n");
    let mut new_text = text;
    new_text.insert_str(line_end + 1, &insert);
    fs::write(config_path, new_text).map_err(|e| AppError::Msg(format!("写入 sing-box 配置失败: {e}")))
}

/// 生成 6 位随机十六进制字符串，用作 TUN 接口名。
/// 使用 RandomState 对时间戳做哈希，每次运行种子不同。
fn random_hex_name() -> String {
    use std::hash::{BuildHasher, Hasher};
    let seed = std::hash::RandomState::new();
    let mut hasher = seed.build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    hasher.write_u128(nanos);
    format!("{:06x}", hasher.finish() & 0xFF_FFFF)
}

/// 清理孤立或异常的 WinTUN 设备节点。
///
/// sing-box TUN 模式依赖 WinTUN 驱动，异常退出后可能残留无效设备节点，
/// 导致下次启动时 TUN 接口创建失败。此函数：
///
/// 1. 通过 `CM_Get_Device_ID_ListW` 枚举所有设备实例 ID
/// 2. 解析双 null 结尾的多字符串缓冲区
/// 3. 筛选包含 "WINTUN" 的设备
/// 4. 检查设备状态：设备节点不存在（CR_NO_SUCH_DEVNODE）或未启动 / 有异常
/// 5. 通过 `pnputil /remove-device` 移除问题设备
/// 6. 最后执行 `pnputil /scan-devices` 重新扫描硬件
fn cleanup_orphaned_wintun() {
    const CR_NO_SUCH_DEVNODE: u32 = 0x0D;

    debug!("检查孤立 WinTUN 设备...");
    let mut instance_ids: Vec<String> = Vec::new();

    unsafe {
        let mut size = 0u32;
        if CM_Get_Device_ID_List_SizeW(&mut size, null(), 0) != CR_SUCCESS {
            return;
        }
        if size == 0 {
            return;
        }

        let mut buffer: Vec<u16> = vec![0u16; size as usize];
        if CM_Get_Device_ID_ListW(null(), buffer.as_mut_ptr(), size, 0) != CR_SUCCESS {
            return;
        }

        let mut start = 0usize;
        while start < buffer.len() {
            let end = buffer[start..]
                .iter()
                .position(|&c| c == 0)
                .map(|p| start + p)
                .unwrap_or(buffer.len());
            if end == start {
                break;
            }
            let id = String::from_utf16_lossy(&buffer[start..end]);

            if id.to_uppercase().contains("WINTUN") {
                let mut dev_inst = 0u32;
                let wide_id = state::wide(&id);
                let locate_ret = CM_Locate_DevNodeW(&mut dev_inst, wide_id.as_ptr(), 0);

                if locate_ret == CR_NO_SUCH_DEVNODE {
                    instance_ids.push(id);
                } else if locate_ret == CR_SUCCESS {
                    let mut status = 0u32;
                    let mut problem = 0u32;
                    if CM_Get_DevNode_Status(&mut status, &mut problem, dev_inst, 0) == CR_SUCCESS
                        && ((status & DN_STARTED) == 0 || (status & DN_HAS_PROBLEM) != 0)
                    {
                        instance_ids.push(id);
                    }
                }
            }

            start = end + 1;
        }
    }

    if instance_ids.is_empty() {
        debug!("未发现孤立 WinTUN 设备");
        return;
    }
    debug!("发现 {} 个孤立 WinTUN 设备", instance_ids.len());

    for id in &instance_ids {
        debug!("移除孤立 WinTUN 设备: {id}");
        let result = hidden_command("pnputil").args(["/remove-device", id.as_str()]).status();
        match result {
            Ok(status) => debug!("pnputil /remove-device {id}: {status}"),
            Err(e) => warn!("pnputil /remove-device {id} 执行失败: {e}"),
        }
    }

    debug!("扫描硬件变更 ({} 个设备已移除)", instance_ids.len());
    let _ = hidden_command("pnputil").arg("/scan-devices").status();
}

// ═══════════════════════════════════════════════
// DNS 缓存刷新
// ═══════════════════════════════════════════════

fn flush_dns() {
    let result = unsafe { DnsFlushResolverCache() };
    if result != 0 {
        debug!("DNS 缓存刷新成功");
    } else {
        warn!("DNS 缓存刷新失败");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_hex_name() {
        let name1 = random_hex_name();
        let name2 = random_hex_name();
        assert_eq!(name1.len(), 6);
        assert!(name1.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(name1, name2);
    }
}
