//! 统一错误类型定义。

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;

/// 应用统一错误类型。
#[derive(Debug)]
pub enum AppError {
    Msg(String),
    Io(io::Error),
    Json(serde_json::Error),
}

impl Display for AppError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Msg(s) => write!(f, "{s}"),
            Self::Io(e) => write!(f, "IO 错误: {e}"),
            Self::Json(e) => write!(f, "JSON 解析错误: {e}"),
        }
    }
}

impl Error for AppError {}

impl From<io::Error> for AppError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
