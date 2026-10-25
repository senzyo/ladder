//! 代理核心身份、发布文件清单与安装运行状态。

use std::path::{Path, PathBuf};

/// 两个代理核心。
///
/// 与核心绑定的文件名和目录名都集中在这里, 避免 "sing-box.exe" 这类字符串
/// 散落在各模块里被当成类型用 —— 那样拼错只会在运行时表现为"进程没找到"。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Core {
    SingBox,
    Xray,
}

impl Core {
    /// 全部核心。顺序与 `CoreStates` 的字段一致, `core_states` 依赖这个顺序。
    pub const ALL: [Core; 2] = [Core::SingBox, Core::Xray];

    /// 日志与通知里使用的名称。
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Core::SingBox => "sing-box",
            Core::Xray => "xray",
        }
    }

    /// 可执行文件名。
    #[must_use]
    pub fn exe_name(self) -> &'static str {
        match self {
            Core::SingBox => "sing-box.exe",
            Core::Xray => "xray.exe",
        }
    }

    /// 核心的工作目录 (放可执行文件、规则集等) 。
    #[must_use]
    pub fn core_dir(self, exe_dir: &Path) -> PathBuf {
        let name = match self {
            Core::SingBox => "sing-box_core",
            Core::Xray => "xray_core",
        };
        exe_dir.join(name)
    }

    /// 可执行文件的完整路径。
    #[must_use]
    pub fn exe_path(self, exe_dir: &Path) -> PathBuf {
        self.core_dir(exe_dir).join(self.exe_name())
    }

    /// 当前生效的配置文件路径。
    #[must_use]
    pub fn active_config(self, exe_dir: &Path) -> PathBuf {
        let name = match self {
            Core::SingBox => "sing-box.json",
            Core::Xray => "xray.json",
        };
        exe_dir.join("configs").join(name)
    }

    /// 可切换配置所在的目录。
    #[must_use]
    pub fn config_dir(self, exe_dir: &Path) -> PathBuf {
        exe_dir.join("configs").join(self.label())
    }

    /// 更新核心时需要从 release zip 里取出的文件。
    #[must_use]
    pub fn artifacts(self) -> CoreArtifacts {
        match self {
            Core::SingBox => CoreArtifacts {
                required: &["sing-box.exe"],
                optional: &["libcronet.dll"],
            },
            Core::Xray => CoreArtifacts {
                required: &["xray.exe"],
                optional: &["wintun.dll", "geoip.dat", "geosite.dat"],
            },
        }
    }
}

/// 更新核心时从 release zip 中提取的文件清单。
///
/// 刻意只取这几个文件, 而不是解压整个包:
/// - xray 包里带的 `geoip.dat` / `geosite.dat` 是官方版, 全量解压会盖掉用户
///   自己配置的那套规则集 (settings.json 的 `download.ruleset`)
/// - `LICENSE`、`README.md`、`xray_no_window.*` 对本程序没用, 白占几十 MB
/// - 核心目录里还有 sing-box 运行时生成的 `cache.db`, 整体替换目录会连它
///   一起丢掉
pub struct CoreArtifacts {
    /// 必需文件, 任意一个在 zip 里找不到都视为更新失败。
    pub required: &'static [&'static str],
    /// 可选文件, 找不到只记录警告 —— 官方随时可能调整打包内容。
    pub optional: &'static [&'static str],
}

impl CoreArtifacts {
    /// 是否需要提取该文件名。
    #[must_use]
    pub fn wants(&self, file_name: &str) -> bool {
        self.required.contains(&file_name) || self.optional.contains(&file_name)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum CoreStatus {
    NotInstalled,
    NotRunning,
    Running,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 这些路径是 Release 目录结构的一部分, 改动会破坏已有安装, 钉在测试里。
    #[test]
    fn test_core_paths() {
        let base = Path::new("base");

        assert_eq!(
            Core::SingBox.exe_path(base),
            base.join("sing-box_core/sing-box.exe")
        );
        assert_eq!(Core::Xray.exe_path(base), base.join("xray_core/xray.exe"));
        assert_eq!(
            Core::SingBox.active_config(base),
            base.join("configs/sing-box.json")
        );
        assert_eq!(
            Core::Xray.active_config(base),
            base.join("configs/xray.json")
        );
        assert_eq!(
            Core::SingBox.config_dir(base),
            base.join("configs/sing-box")
        );
        assert_eq!(Core::Xray.config_dir(base), base.join("configs/xray"));
    }

    /// 可执行文件名在 `exe_name` 和 `artifacts().required` 里各写了一遍, 两处
    /// 不一致时, 版本检查读的路径就和更新替换的文件对不上: 更新会报"缺少必需
    /// 文件", 或者换掉一个没人加载的文件。
    #[test]
    fn test_artifacts_required_contains_exe() {
        for core in Core::ALL {
            assert!(
                core.artifacts().required.contains(&core.exe_name()),
                "{} 的 required 不含 {}",
                core.label(),
                core.exe_name()
            );
        }
    }
}
