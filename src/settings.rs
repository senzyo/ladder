use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Settings {
    #[serde(default = "default_gh_proxy")]
    pub gh_proxy: GhProxy,
    #[serde(default)]
    pub log: Log,
    #[serde(default)]
    pub download: Download,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GhProxy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_proxy_url")]
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Log {
    #[serde(default = "default_log_level")]
    pub level: String,
}

#[derive(Debug, Clone, Deserialize)]
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
    pub fn load(exe_dir: &Path) -> Self {
        let path = exe_dir.join("configs").join("settings.toml");

        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("警告: 读取配置文件失败 ({}), 使用默认配置: {e}", path.display());
                return Self::default();
            }
        };

        match toml::from_str::<Settings>(&text) {
            Ok(mut s) => {
                s.validate();
                s
            }
            Err(e) => {
                eprintln!("警告: 解析配置文件失败 ({}), 使用默认配置: {e}", path.display());
                Self::default()
            }
        }
    }

    fn validate(&mut self) {
        let level = self.log.level.to_lowercase();
        if ALLOWED_LEVELS.contains(&level.as_str()) {
            self.log.level = level;
        } else {
            eprintln!(
                "警告: 无效的日志级别 \"{}\", 可选值: {:?}, 回退到 \"debug\"",
                self.log.level, ALLOWED_LEVELS
            );
            self.log.level = "debug".to_string();
        }

        if self.download.max_retries == 0 {
            eprintln!("警告: max_retries 不能为 0, 回退到默认值 3");
            self.download.max_retries = 3;
        }

        if self.download.retry_delay_secs == 0 {
            eprintln!("警告: retry_delay_secs 不能为 0, 回退到默认值 2");
            self.download.retry_delay_secs = 2;
        }

        if self.gh_proxy.enabled && self.gh_proxy.url.is_empty() {
            eprintln!("警告: gh_proxy 已启用但 url 为空, 自动关闭代理");
            self.gh_proxy.enabled = false;
        }
    }
}


