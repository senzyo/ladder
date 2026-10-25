//! 跨登录会话的单实例互斥体守卫。

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::PCWSTR;

use crate::app_error::AppError;
use crate::windows_strings;

/// 单实例锁使用的 Global 命名互斥体名称。
///
/// 使用 Global 命名空间实现整机单实例: 即使同一台机器有多个登录会话
/// (如 RDP) , 也只会运行一个实例。本程序修改系统级网络配置, 跨会话
/// 并发运行会互相冲突。
const SINGLE_INSTANCE_MUTEX: &str = "Global\\Ladder.SingleInstance";

/// 单实例互斥体守卫, 防止软件重复启动。
///
/// 第一个实例创建命名互斥体并持有句柄; 后续实例创建同一互斥体时
/// 收到 ERROR_ALREADY_EXISTS, 据此判定已存在运行中的实例。
/// 句柄在 Drop 时关闭, 进程异常退出时内核也会自动释放互斥体, 无残留锁。
#[must_use = "应用运行期间必须持有单实例守卫"]
pub(crate) struct SingleInstanceGuard {
    handle: HANDLE,
}

impl SingleInstanceGuard {
    /// 尝试获取单实例锁。
    ///
    /// 返回 `Ok(Some(guard))` 表示本实例获得锁; `Ok(None)` 表示已有
    /// 实例在运行; `Err` 表示互斥体创建失败 (如权限不足) , 调用方
    /// 应降级为允许启动, 避免误伤正常使用。
    pub(crate) fn acquire() -> Result<Option<Self>, AppError> {
        let name = windows_strings::wide(SINGLE_INSTANCE_MUTEX);
        unsafe {
            let handle = CreateMutexW(None, false, PCWSTR(name.as_ptr()))
                .map_err(|e| AppError::Msg(format!("创建单实例互斥体失败: {e}")))?;
            // 必须紧跟 CreateMutexW 读取, 中间任何一次 Win32 调用都会覆盖它
            let already_exists = GetLastError() == ERROR_ALREADY_EXISTS;
            if already_exists {
                let _ = CloseHandle(handle);
                Ok(None)
            } else {
                Ok(Some(Self { handle }))
            }
        }
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}
