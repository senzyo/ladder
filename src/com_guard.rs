//! 当前线程的 COM 初始化与释放守卫。

use std::marker::PhantomData;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

use crate::app_error::AppError;

/// COM 初始化守卫, Drop 时自动调用 CoUninitialize。
/// 私有标记禁止跨线程移动或共享, 使释放发生在初始化线程。
#[must_use = "保留守卫直到当前线程的 COM 操作结束"]
pub(crate) struct ComGuard {
    _thread_bound: PhantomData<*mut ()>,
}

impl ComGuard {
    pub(crate) fn try_new() -> Result<Self, AppError> {
        // SAFETY: 保留参数为空, 仅初始化当前线程; 包括 S_FALSE 在内的成功结果
        // 都由同线程的守卫配对释放, 失败时不构造守卫。
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|e| AppError::Msg(format!("初始化 COM 失败: {e}")))?;
        Ok(Self {
            _thread_bound: PhantomData,
        })
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: 守卫只能由成功初始化构造, 不可跨线程移动或复制;
        // 当前线程的这一次成功初始化仅在这里配对释放一次。
        unsafe { CoUninitialize() };
    }
}
