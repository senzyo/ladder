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
                "上次软件更新未完成, 配置已保留。请重试更新, 或退出后手动替换 ladder.exe。".into(),
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
        self.check_with(move || app_update::check_latest(&prefix))
    }

    fn check_with(
        &self,
        check: impl FnOnce() -> Result<Option<AppRelease>, String> + Send + 'static,
    ) -> Result<(), String> {
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
            .spawn(move || match check() {
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
                let error = format!("无法创建软件更新检查线程: {error}");
                self.fail(error.clone());
                error
            })
    }

    pub(crate) fn download(&self, prefix: String) -> Result<(), String> {
        self.download_with(move |release, progress| {
            app_update::download(release, &prefix, progress)
        })
    }

    fn download_with(
        &self,
        download: impl FnOnce(
            &AppRelease,
            &mut dyn FnMut(UpdateProgress),
        ) -> Result<PreparedAppUpdate, String>
        + Send
        + 'static,
    ) -> Result<(), String> {
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
                let result = download(&release, &mut |progress| {
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
                let error = format!("无法创建软件下载线程: {error}");
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
            .ok_or("软件更新文件不可用, 请重新下载")?;
        state.status = AppUpdateState::Installing;
        Ok(prepared)
    }

    pub(crate) fn fail(&self, error: String) {
        logging::error("update", &error);
        self.0.lock().unwrap().status = AppUpdateState::Failed(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn wait_for(updater: &AppUpdater, expected: AppUpdateState) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let current = updater.snapshot();
            if current == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "expected {expected:?}, got {current:?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn checks_reject_overlapping_requests_and_can_retry_after_failure() {
        let updater = AppUpdater(Arc::default());
        assert!(updater.download_with(|_, _| unreachable!()).is_err());
        assert!(updater.take_prepared().is_err());
        let (complete, result) = mpsc::channel();
        updater
            .check_with(move || result.recv_timeout(Duration::from_secs(5)).unwrap())
            .unwrap();
        assert_eq!(updater.snapshot(), AppUpdateState::Checking);
        assert!(updater.check_with(|| unreachable!()).is_err());
        assert!(updater.download_with(|_, _| unreachable!()).is_err());
        complete.send(Err("check failed".into())).unwrap();
        wait_for(&updater, AppUpdateState::Failed("check failed".into()));

        let (_root, release, prepared) = app_update::tests::prepared_update();
        drop(prepared);
        updater.check_with(move || Ok(Some(release))).unwrap();
        wait_for(&updater, AppUpdateState::Available("4.0.0".into()));
        updater.check_with(|| Ok(None)).unwrap();
        wait_for(&updater, AppUpdateState::Latest);
        assert!(updater.download_with(|_, _| unreachable!()).is_err());
        assert!(updater.take_prepared().is_err());
    }

    #[test]
    fn downloads_retry_and_can_only_be_taken_once_when_ready() {
        let (root, release, prepared) = app_update::tests::prepared_update();
        let updater = AppUpdater(Arc::default());
        updater.check_with(move || Ok(Some(release))).unwrap();
        wait_for(&updater, AppUpdateState::Available("4.0.0".into()));

        let (complete, result) = mpsc::channel();
        updater
            .download_with(move |_, _| result.recv_timeout(Duration::from_secs(5)).unwrap())
            .unwrap();
        assert_eq!(
            updater.snapshot(),
            AppUpdateState::Downloading(UpdateProgress::Checking)
        );
        assert!(updater.check_with(|| unreachable!()).is_err());
        assert!(updater.download_with(|_, _| unreachable!()).is_err());
        assert!(updater.take_prepared().is_err());
        complete.send(Err("download failed".into())).unwrap();
        wait_for(&updater, AppUpdateState::Failed("download failed".into()));

        let (complete, ready) = mpsc::channel();
        updater
            .download_with(move |release, progress| {
                assert_eq!(release.version, "4.0.0");
                progress(UpdateProgress::Verifying);
                ready.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(prepared)
            })
            .unwrap();
        wait_for(
            &updater,
            AppUpdateState::Downloading(UpdateProgress::Verifying),
        );
        assert!(updater.take_prepared().is_err());
        complete.send(()).unwrap();
        wait_for(&updater, AppUpdateState::Ready("4.0.0".into()));
        let prepared = updater.take_prepared().unwrap();
        assert_eq!(updater.snapshot(), AppUpdateState::Installing);
        assert!(updater.take_prepared().is_err());
        assert!(updater.check_with(|| unreachable!()).is_err());
        assert!(updater.download_with(|_, _| unreachable!()).is_err());
        drop(prepared);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);

        updater.fail("installation cancelled".into());
        updater.check_with(|| Ok(None)).unwrap();
        wait_for(&updater, AppUpdateState::Latest);
    }
}
