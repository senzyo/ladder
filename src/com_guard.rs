//! 当前线程的 COM 初始化与释放守卫。

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

use crate::app_error::AppError;

/// COM 初始化守卫, Drop 时自动调用 CoUninitialize。
#[must_use = "保留守卫直到当前线程的 COM 操作结束"]
pub(crate) struct ComGuard;

impl ComGuard {
    pub(crate) fn try_new() -> Result<Self, AppError> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| AppError::Msg(format!("初始化 COM 失败: {e}")))?;
        }
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
