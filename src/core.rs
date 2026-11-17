use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Core {
    SingBox,
    Xray,
    Mihomo,
}

impl Core {
    pub const ALL: [Core; 3] = [Core::Mihomo, Core::SingBox, Core::Xray];

    pub fn label(self) -> &'static str {
        match self {
            Self::SingBox => "sing-box",
            Self::Xray => "xray",
            Self::Mihomo => "mihomo",
        }
    }

    pub(crate) fn config_dir(self, root: &Path) -> PathBuf {
        root.join("configs").join(self.label())
    }

    pub(crate) fn core_dir(self, root: &Path) -> PathBuf {
        root.join(format!("{}_core", self.label()))
    }

    #[cfg(test)]
    pub(crate) fn executable(self, root: &Path) -> PathBuf {
        self.core_dir(root)
            .join("1.0.0")
            .join(format!("{}.exe", self.label()))
    }
}
