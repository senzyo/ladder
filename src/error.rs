//! 统一错误类型定义。

/// 应用统一错误类型。
#[derive(Debug)]
pub enum AppError {
    Msg(String),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Msg(s) => write!(f, "{s}"),
            Self::Io(e) => write!(f, "IO 错误: {e}"),
            Self::Json(e) => write!(f, "JSON 解析错误: {e}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
