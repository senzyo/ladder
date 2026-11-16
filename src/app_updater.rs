use std::sync::{Arc, Mutex};
use std::thread;

use crate::app_update::{self, AppRelease, PreparedAppUpdate};
use crate::logging;
use crate::update::UpdateProgress;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum AppUpdateState {
    #[default]
    Idle,
    Checking,
    Latest,
    Available(String),
    Downloading(UpdateProgress),
    Ready(String),
    Installing,
    Failed(String),
}

impl AppUpdateState {
    fn busy(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Downloading(_) | Self::Installing
        )
    }
}

#[derive(Default)]
struct State {
    status: AppUpdateState,
    release: Option<AppRelease>,
    prepared: Option<PreparedAppUpdate>,
}

#[derive(Clone)]
pub(crate) struct AppUpdater(Arc<Mutex<State>>);

impl Default for AppUpdater {
    fn default() -> Self {
        let mut state = State::default();
        if std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.parent()
                    .map(|root| root.join("update-error.log").is_file())
            })
            .unwrap_or(false)
        {
            state.status = AppUpdateState::Failed(
                "上次软件更新未完成，配置已保留。请重试更新，或退出后手动替换 ladder.exe；详情见程序目录的 update-error.log。".into(),
            );
        }
        Self(Arc::new(Mutex::new(state)))
    }
}

impl AppUpdater {
    pub(crate) fn snapshot(&self) -> AppUpdateState {
        self.0.lock().unwrap().status.clone()
    }

    pub(crate) fn check(&self, prefix: String) -> Result<(), String> {
        {
            let mut state = self.0.lock().unwrap();
            if state.status.busy() {
                return Err("正在处理软件更新".into());
            }
            state.status = AppUpdateState::Checking;
            state.release = None;
            state.prepared = None;
        }
        let updater = self.clone();
        thread::Builder::new()
            .name("ladder-check-update".into())
            .spawn(move || match app_update::check_latest(&prefix) {
                Ok(release) => {
                    let mut state = updater.0.lock().unwrap();
                    state.status = release.as_ref().map_or(AppUpdateState::Latest, |release| {
                        AppUpdateState::Available(release.version.clone())
                    });
                    state.release = release;
                    logging::info("update", "软件更新检查完成");
                }
                Err(error) => updater.fail(error),
            })
            .map(|_| ())
            .map_err(|error| {
                let error = format!("无法创建软件更新检查线程：{error}");
                self.fail(error.clone());
                error
            })
    }

    pub(crate) fn download(&self, prefix: String) -> Result<(), String> {
        let release = {
            let mut state = self.0.lock().unwrap();
            if state.status.busy() {
                return Err("正在处理软件更新".into());
            }
            let release = state.release.clone().ok_or("请先检查软件更新")?;
            state.status = AppUpdateState::Downloading(UpdateProgress::Checking);
            release
        };
        let updater = self.clone();
        thread::Builder::new()
            .name("ladder-download-update".into())
            .spawn(move || {
                let result = app_update::download(&release, &prefix, |progress| {
                    updater.0.lock().unwrap().status = AppUpdateState::Downloading(progress);
                });
                match result {
                    Ok(prepared) => {
                        let mut state = updater.0.lock().unwrap();
                        state.status = AppUpdateState::Ready(release.version);
                        state.prepared = Some(prepared);
                        logging::info("update", "软件更新已下载并通过校验");
                    }
                    Err(error) => updater.fail(error),
                }
            })
            .map(|_| ())
            .map_err(|error| {
                let error = format!("无法创建软件下载线程：{error}");
                self.fail(error.clone());
                error
            })
    }

    pub(crate) fn take_prepared(&self) -> Result<PreparedAppUpdate, String> {
        let mut state = self.0.lock().unwrap();
        if !matches!(state.status, AppUpdateState::Ready(_)) {
            return Err("软件更新尚未准备好".into());
        }
        let prepared = state
            .prepared
            .take()
            .ok_or("软件更新文件不可用，请重新下载")?;
        state.status = AppUpdateState::Installing;
        Ok(prepared)
    }

    pub(crate) fn fail(&self, error: String) {
        logging::error("update", &error);
        self.0.lock().unwrap().status = AppUpdateState::Failed(error);
    }
}
