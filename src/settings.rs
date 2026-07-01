use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

use crate::error::AppError;

/// 核心运行模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CoreMode {
    #[default]
    Xray,
    #[serde(rename = "sing-box")]
    SingBox,
    Both,
}

impl CoreMode {
    pub fn runs_sing_box(&self) -> bool {
        matches!(self, Self::SingBox | Self::Both)
    }

    pub fn runs_xray(&self) -> bool {
        matches!(self, Self::Xray | Self::Both)
    }
}

/// 核心配置。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Core {
    #[serde(default)]
    pub mode: CoreMode,
}

/// 应用配置，对应 `settings.json`。
///
/// 所有字段均提供默认值，配置文件缺失或解析失败时自动回退。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Settings {
    #[serde(default = "default_gh_proxy")]
    pub gh_proxy: GhProxy,
    #[serde(default)]
    pub log: Log,
    #[serde(default)]
    pub download: Download,
    #[serde(default)]
    pub core: Core,
}

/// 加载期间收集的警告信息，供 init_logging 之后通过 tracing 输出。
static LOAD_WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// GitHub CDN 代理配置。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GhProxy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 代理 URL 前缀，会拼接在 GitHub 原始下载链接前面。
    #[serde(default = "default_proxy_url")]
    pub url: String,
}

/// 日志配置。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Log {
    /// 日志级别，可选值: "debug", "info", "warn", "error"。
    #[serde(default = "default_log_level")]
    pub level: String,
}

/// 下载重试配置。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Download {
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_retry_delay")]
    pub retry_delay_secs: u64,
}

fn default_true() -> bool {
    true
}

fn default_proxy_url() -> String {
    "https://gh-proxy.com/".to_string()
}

fn default_log_level() -> String {
    "debug".to_string()
}

fn default_max_retries() -> u32 {
    3
}

fn default_retry_delay() -> u64 {
    2
}

fn default_gh_proxy() -> GhProxy {
    GhProxy {
        enabled: true,
        url: default_proxy_url(),
    }
}

impl Default for GhProxy {
    fn default() -> Self {
        default_gh_proxy()
    }
}

impl Default for Log {
    fn default() -> Self {
        Log {
            level: default_log_level(),
        }
    }
}

impl Default for Download {
    fn default() -> Self {
        Download {
            max_retries: default_max_retries(),
            retry_delay_secs: default_retry_delay(),
        }
    }
}

const ALLOWED_LEVELS: &[&str] = &["debug", "info", "warn", "error"];

impl Settings {
    /// 从 `exe_dir/settings.json` 加载配置。
    /// 文件不存在或格式错误时打印警告并返回默认值。
    pub fn load(exe_dir: &Path) -> Self {
        let path = exe_dir.join("settings.json");

        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                push_warning(format!("读取配置文件失败 ({}), 使用默认配置: {e}", path.display()));
                return Self::default();
            }
        };

        match serde_json::from_str::<Settings>(&text) {
            Ok(mut s) => {
                s.validate();
                s
            }
            Err(e) => {
                push_warning(format!("解析配置文件失败 ({}), 使用默认配置: {e}", path.display()));
                Self::default()
            }
        }
    }

    /// 将当前配置写入 `exe_dir/settings.json`。
    pub fn save(&self, exe_dir: &Path) -> Result<(), AppError> {
        let path = exe_dir.join("settings.json");
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::Msg(format!("序列化配置失败: {e}")))?;
        std::fs::write(&path, json).map_err(|e| AppError::Msg(format!("写入配置文件失败: {e}")))
    }

    /// 取出加载期间收集的警告。调用后清空，仅首次调用返回内容。
    pub fn take_warnings() -> Vec<String> {
        LOAD_WARNINGS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect()
    }

    /// 校验配置值合法性，非法值回退到默认值。
    fn validate(&mut self) {
        let level = self.log.level.to_lowercase();
        if ALLOWED_LEVELS.contains(&level.as_str()) {
            self.log.level = level;
        } else {
            push_warning(format!(
                "无效的日志级别 \"{}\", 可选值: {:?}, 回退到 \"debug\"",
                self.log.level, ALLOWED_LEVELS
            ));
            self.log.level = "debug".to_string();
        }

        if self.download.max_retries == 0 {
            push_warning("max_retries 不能为 0, 回退到默认值 3".to_string());
            self.download.max_retries = 3;
        } else if self.download.max_retries > 10 {
            push_warning("max_retries 超出上限 10, 已自动限制".to_string());
            self.download.max_retries = 10;
        }

        if self.download.retry_delay_secs == 0 {
            push_warning("retry_delay_secs 不能为 0, 回退到默认值 2".to_string());
            self.download.retry_delay_secs = 2;
        } else if self.download.retry_delay_secs > 30 {
            push_warning("retry_delay_secs 超出上限 30, 已自动限制".to_string());
            self.download.retry_delay_secs = 30;
        }

        if self.gh_proxy.enabled && self.gh_proxy.url.is_empty() {
            push_warning("gh_proxy 已启用但 url 为空, 自动关闭代理".to_string());
            self.gh_proxy.enabled = false;
        }
    }
}

fn push_warning(msg: String) {
    eprintln!("警告: {msg}");
    if let Ok(mut warnings) = LOAD_WARNINGS.lock() {
        warnings.push(msg);
    }
}
