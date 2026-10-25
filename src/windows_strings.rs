//! 共享的 Windows UTF-16 字符串转换。

use std::ffi::OsStr;

/// 将字符串转为 null 结尾的 UTF-16 Vec。
pub fn wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}
