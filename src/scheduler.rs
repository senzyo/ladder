//! Windows 任务计划程序: 开机自动恢复物理网卡 DNS 为 DHCP。
//!
//! 通过 `schtasks.exe` 注册 BootTrigger 任务, 开机后执行 PowerShell 命令,
//! 将所有物理网卡 DNS 重置为自动获取。用于防止意外断电后 DNS 状态残留。

use std::fs;
use std::path::Path;
use tracing::{debug, info, warn};

use crate::process;

const TASK_NAME: &str = "Reset_DNS_On_Boot";

const TASK_XML: &str = r#"<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Author>ladder</Author>
    <Description>Restore the DNS server address of the physical network adapter to DHCP</Description>
    <URI>\Reset_DNS_On_Boot</URI>
  </RegistrationInfo>
  <Triggers>
    <BootTrigger>
      <Enabled>true</Enabled>
    </BootTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>S-1-5-18</UserId>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>false</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>powershell.exe</Command>
      <Arguments>-WindowStyle Hidden -Command "Get-NetAdapter -Physical | Set-DnsClientServerAddress -ResetServerAddresses"</Arguments>
    </Exec>
  </Actions>
</Task>"#;

/// 确保开机恢复 DNS 的任务计划已注册。
///
/// 先查询任务是否已存在, 不存在才创建。失败仅记录警告, 不阻断启动流程。
pub fn ensure_boot_dns_reset_task(exe_dir: &Path) {
    if let Err(e) = register_task(exe_dir) {
        warn!("注册开机 DNS 恢复任务失败: {e}");
    }
}

fn task_exists() -> bool {
    process::hidden_command("schtasks")
        .args(["/Query", "/TN", TASK_NAME])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn register_task(exe_dir: &Path) -> Result<(), String> {
    if task_exists() {
        debug!("开机 DNS 恢复任务已存在, 跳过注册");
        return Ok(());
    }

    let xml_path = exe_dir.join("_dns_task.xml");
    fs::write(&xml_path, TASK_XML).map_err(|e| format!("写入任务 XML 失败: {e}"))?;

    let result = process::hidden_command("schtasks")
        .args(["/Create", "/TN", TASK_NAME, "/XML", &xml_path.to_string_lossy(), "/F"])
        .output()
        .map_err(|e| format!("执行 schtasks 失败: {e}"))?;

    let _ = fs::remove_file(&xml_path);

    if result.status.success() {
        info!("已注册开机 DNS 恢复任务计划: {TASK_NAME}");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        Err(format!(
            "schtasks 退出码 {}: {}{}",
            result.status.code().unwrap_or(-1),
            stdout.trim(),
            stderr.trim()
        ))
    }
}
