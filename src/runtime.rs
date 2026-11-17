use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::core_output::CoreDiagnostics;
use crate::dns::DnsLease;
use crate::logging;
use crate::managed_child::{JobProcess, ManagedChild};
use crate::scheduler::{DesktopDns, desktop_dns};
use crate::update::{self, CoreVersion, PreparedUpdate, UpdateProgress};
use crate::{tun_cleanup, tun_config};

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

    fn config_dir(self, root: &Path) -> PathBuf {
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Settings {
    pub theme: ThemePreference,
    pub minimize_to_tray: bool,
    pub auto_sing_box: bool,
    pub auto_xray: bool,
    #[serde(default)]
    pub auto_mihomo: bool,
    pub hijack_dns: bool,
    #[serde(default)]
    pub github_proxy: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigValidation {
    Checking,
    Valid,
    Invalid(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreOperation {
    Starting,
    Stopping,
    Switching,
    Updating,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoreState {
    pub configs: Vec<String>,
    pub current: Option<String>,
    pub installed: bool,
    pub running: bool,
    pub operation: Option<CoreOperation>,
    pub run_error: Option<String>,
    pub output_error: Option<String>,
    pub scan_error: Option<String>,
    pub version: Option<String>,
    pub version_error: Option<String>,
    pub update: Option<UpdateProgress>,
    pub update_error: Option<String>,
    pub versions: Vec<CoreVersion>,
    pub selected_version: Option<String>,
    pub versions_loading: bool,
    pub versions_loaded: bool,
    pub versions_has_more: bool,
    pub versions_error: Option<String>,
    pub local_versions_error: Option<String>,
    pub downloading_version: Option<String>,
    pub validation: BTreeMap<String, ConfigValidation>,
    validation_stamps: BTreeMap<String, (u64, Option<SystemTime>)>,
    pub validation_busy: bool,
    versions_page: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub settings: Settings,
    pub sing_box: CoreState,
    pub xray: CoreState,
    pub mihomo: CoreState,
    pub settings_error: Option<String>,
    pub dns_recovery_error: Option<String>,
    pub dns_recovery_busy: bool,
    pub dns_task_warning: Option<String>,
}

impl Snapshot {
    pub fn core(&self, core: Core) -> &CoreState {
        match core {
            Core::SingBox => &self.sing_box,
            Core::Xray => &self.xray,
            Core::Mihomo => &self.mihomo,
        }
    }

    fn core_mut(&mut self, core: Core) -> &mut CoreState {
        match core {
            Core::SingBox => &mut self.sing_box,
            Core::Xray => &mut self.xray,
            Core::Mihomo => &mut self.mihomo,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    MinimizeToTray,
    AutoSingBox,
    AutoXray,
    AutoMihomo,
    HijackDns,
}

#[derive(Clone)]
pub enum Action {
    Start(Core),
    Stop(Core),
    ValidateConfig(Core, String),
    ValidateAllConfigs(Core),
    SwitchConfig(Core, String),
    DeleteConfig(Core, String),
    Set(Setting, bool),
    RetryDns,
    RefreshVersions(Core),
    LoadMoreVersions(Core),
    DownloadVersion(Core, String),
    SwitchVersion(Core, String),
    DeleteVersion(Core, String),
}

enum Request {
    Action(Action),
    SetTheme(ThemePreference, async_channel::Sender<()>),
    SetGithubProxy(String, async_channel::Sender<()>),
    Shutdown(mpsc::Sender<Result<Option<String>, String>>),
    PrepareAppUpdate(mpsc::Sender<Result<(), String>>),
    Prepared(Core, PreparedUpdate),
    #[cfg(test)]
    Panic,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct Saved {
    settings: Settings,
    sing_box: Option<String>,
    xray: Option<String>,
    mihomo: Option<String>,
    sing_box_version: Option<String>,
    xray_version: Option<String>,
    mihomo_version: Option<String>,
}

impl Saved {
    fn load(root: &Path) -> Result<Self, String> {
        let data = match fs::read(root.join("ladder-state.json")) {
            Ok(data) => data,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("读取 ladder-state.json 失败: {e}")),
        };
        serde_json::from_slice(&data).map_err(|e| {
            format!(
                "ladder-state.json 格式无效 (第 {} 行, 第 {} 列) ",
                e.line(),
                e.column()
            )
        })
    }

    fn store(&self, root: &Path) -> io::Result<()> {
        let temp = root.join("ladder-state.json.tmp");
        let mut file = fs::File::create(&temp)?;
        serde_json::to_writer_pretty(&mut file, self)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(temp, root.join("ladder-state.json"))
    }

    fn current(&self, core: Core) -> &Option<String> {
        match core {
            Core::SingBox => &self.sing_box,
            Core::Xray => &self.xray,
            Core::Mihomo => &self.mihomo,
        }
    }

    fn selected_version(&self, core: Core) -> &Option<String> {
        match core {
            Core::SingBox => &self.sing_box_version,
            Core::Xray => &self.xray_version,
            Core::Mihomo => &self.mihomo_version,
        }
    }

    fn select_version(&mut self, core: Core, version: String) {
        match core {
            Core::SingBox => self.sing_box_version = Some(version),
            Core::Xray => self.xray_version = Some(version),
            Core::Mihomo => self.mihomo_version = Some(version),
        }
    }

    fn executable(&self, core: Core, root: &Path) -> Result<PathBuf, String> {
        let version = self
            .selected_version(core)
            .as_deref()
            .ok_or("未找到核心程序，请先下载并选择核心版本")?;
        Ok(update::version_directory(core, root, version)?.join(format!("{}.exe", core.label())))
    }
}

#[derive(Clone)]
pub struct Runtime {
    root: PathBuf,
    state: Arc<Mutex<Snapshot>>,
    tx: mpsc::Sender<Request>,
    quitting: Arc<AtomicBool>,
}

impl Runtime {
    #[cfg(test)]
    fn open(root: PathBuf) -> Result<Self, String> {
        let dns = DnsLease::for_tests(&root);
        Self::open_with_dns(root, DesktopDns::Portable(dns), None)
    }

    pub(crate) fn open_with_dns(
        root: PathBuf,
        dns: DesktopDns,
        task_warning: Option<String>,
    ) -> Result<Self, String> {
        crate::emergency::prepare(&dns);
        logging::info("dns", "检查启动时待恢复的 DNS 记录");
        let recovery_error = dns.lease().and_then(DnsLease::restore_if_pending).err();
        match &recovery_error {
            Some(error) => logging::error("dns", format_args!("启动时恢复 DNS 失败：{error}")),
            None => logging::info("dns", "启动时 DNS 恢复检查完成"),
        }
        if let Some(warning) = &task_warning {
            logging::warn("dns", warning);
        }
        Self::start_worker(root, dns, recovery_error.clone(), task_warning.clone()).map_err(
            |mut error| {
                if let Some(message) = recovery_error {
                    error.push_str(&format!("\nDNS 尚未恢复：{message}"));
                }
                if let Some(message) = task_warning {
                    error.push('\n');
                    error.push_str(&message);
                }
                error
            },
        )
    }

    fn start_worker(
        root: PathBuf,
        dns: DesktopDns,
        dns_recovery_error: Option<String>,
        dns_task_warning: Option<String>,
    ) -> Result<Self, String> {
        let root = std::path::absolute(root).map_err(|e| format!("读取软件目录失败: {e}"))?;
        let saved = Saved::load(&root)?;
        let mut snapshot = Snapshot {
            settings: saved.settings.clone(),
            dns_recovery_error,
            dns_task_warning,
            ..Default::default()
        };
        for core in Core::ALL {
            let dir = core.config_dir(&root);
            fs::create_dir_all(&dir)
                .map_err(|e| format!("创建 {} 配置目录失败: {e}", core.label()))?;
            cleanup_runtime_configs(&dir);
            let state = snapshot.core_mut(core);
            state.configs = list_configs(core, &dir)?;
            state.current = saved.current(core).clone();
            state.selected_version = saved.selected_version(core).clone();
            state.installed = saved.executable(core, &root).is_ok_and(|exe| exe.is_file());
            match update::installed_versions(core, &root) {
                Ok(mut versions) => {
                    update::sort_versions(&mut versions, state.selected_version.as_deref());
                    state.versions = versions;
                }
                Err(error) => state.local_versions_error = Some(error),
            }
            let auto = match core {
                Core::SingBox => saved.settings.auto_sing_box,
                Core::Xray => saved.settings.auto_xray,
                Core::Mihomo => saved.settings.auto_mihomo,
            };
            state.operation = auto.then_some(CoreOperation::Starting);
        }
        let state = Arc::new(Mutex::new(snapshot));
        let (tx, rx) = mpsc::channel();
        let quitting = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            root: root.clone(),
            state: state.clone(),
            saved,
            children: Default::default(),
            dns,
            quitting: quitting.clone(),
            version_files: Default::default(),
        };
        thread::Builder::new()
            .name("ladder-cores".into())
            .spawn(move || worker.run(rx))
            .map_err(|e| format!("创建核心管理线程失败: {e}"))?;
        logging::info("runtime", "核心管理线程已启动");
        Ok(Self {
            root,
            state,
            tx,
            quitting,
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().clone()
    }

    pub fn config_dir(&self, core: Core) -> PathBuf {
        core.config_dir(&self.root)
    }

    pub fn core_dir(&self, core: Core) -> PathBuf {
        core.core_dir(&self.root)
    }

    pub fn send(&self, action: Action) -> Result<(), String> {
        if self.quitting.load(Ordering::Acquire) {
            return Err("正在退出或准备更新，请稍候".into());
        }
        match action {
            Action::ValidateConfig(core, ref name) => {
                return self.validate_configs(core, Some(name));
            }
            Action::ValidateAllConfigs(core) => return self.validate_configs(core, None),
            Action::RefreshVersions(core) => return self.load_versions(core, false),
            Action::LoadMoreVersions(core) => return self.load_versions(core, true),
            Action::DownloadVersion(core, ref version) => {
                return self.begin_download(core, version);
            }
            _ => {}
        }
        let mut snapshot = self.state.lock().unwrap();
        let retry_dns = matches!(action, Action::RetryDns);
        let target = match &action {
            Action::Start(core) => Some((*core, CoreOperation::Starting)),
            Action::Stop(core) => Some((*core, CoreOperation::Stopping)),
            Action::SwitchConfig(core, _) | Action::DeleteConfig(core, _) => {
                Some((*core, CoreOperation::Switching))
            }
            Action::SwitchVersion(core, _) | Action::DeleteVersion(core, _) => {
                if snapshot.core(*core).update.is_some() {
                    return Err("正在下载核心，完成后可切换或删除版本".into());
                }
                Some((*core, CoreOperation::Updating))
            }
            Action::Set(_, _) => None,
            Action::RetryDns => {
                if snapshot.dns_recovery_error.is_none() || snapshot.dns_recovery_busy {
                    return Err("当前没有可重试的 DNS 恢复操作".into());
                }
                snapshot.dns_recovery_busy = true;
                None
            }
            Action::RefreshVersions(_)
            | Action::ValidateConfig(..)
            | Action::ValidateAllConfigs(_)
            | Action::LoadMoreVersions(_)
            | Action::DownloadVersion(_, _) => unreachable!(),
        };
        if let Some((core, operation)) = target {
            let active = snapshot.core_mut(core);
            if active.operation.is_some() {
                return Err(format!("{} 正在处理上一项操作", core.label()));
            }
            active.operation = Some(operation);
            if operation == CoreOperation::Updating {
                active.update_error = None;
            } else {
                active.run_error = None;
            }
        }
        self.tx.send(Request::Action(action)).map_err(|_| {
            if let Some((core, _)) = target {
                snapshot.core_mut(core).operation = None;
            }
            if retry_dns {
                snapshot.dns_recovery_busy = false;
            }
            "核心管理线程已关闭".to_string()
        })
    }

    pub fn set_theme(&self, theme: ThemePreference) -> Result<async_channel::Receiver<()>, String> {
        let (tx, rx) = async_channel::bounded(1);
        self.tx
            .send(Request::SetTheme(theme, tx))
            .map_err(|_| "核心管理线程已关闭".to_string())?;
        Ok(rx)
    }

    pub fn set_github_proxy(&self, prefix: String) -> Result<async_channel::Receiver<()>, String> {
        let prefix = crate::github::normalized_prefix(&prefix)?;
        let (tx, rx) = async_channel::bounded(1);
        self.tx
            .send(Request::SetGithubProxy(prefix, tx))
            .map_err(|_| "核心管理线程已关闭".to_string())?;
        Ok(rx)
    }

    fn validate_configs(&self, core: Core, name: Option<&str>) -> Result<(), String> {
        if self.quitting.load(Ordering::Acquire) {
            return Err("正在退出，无法验证配置".into());
        }
        let mut snapshot = self.state.lock().unwrap();
        let state = snapshot.core_mut(core);
        if state.validation_busy {
            return Err("正在验证配置，请稍候".into());
        }
        let version = state
            .selected_version
            .clone()
            .ok_or("请先下载并选择核心版本")?;
        let exe = update::version_directory(core, &self.root, &version)?
            .join(format!("{}.exe", core.label()));
        let exe_stamp = executable_stamp(&exe).ok_or("请先下载并选择核心版本")?;
        let names = match name {
            Some(name) => {
                config_path(core, &self.root, name)?;
                vec![name.to_owned()]
            }
            None => list_configs(core, &self.config_dir(core))?,
        };
        let jobs = names
            .iter()
            .map(|name| {
                let path = readable_config(core, &self.root, name);
                let stamp = executable_stamp(&core.config_dir(&self.root).join(name));
                (name.clone(), path, stamp)
            })
            .collect::<Vec<_>>();
        for (name, _, stamp) in &jobs {
            state
                .validation
                .insert(name.clone(), ConfigValidation::Checking);
            if let Some(stamp) = stamp {
                state.validation_stamps.insert(name.clone(), *stamp);
            }
        }
        state.validation_busy = true;
        let runtime = self.clone();
        thread::Builder::new()
            .name(format!("{}-validate", core.label()))
            .spawn(move || {
                for (name, path, stamp) in jobs {
                    if runtime.quitting.load(Ordering::Acquire) {
                        break;
                    }
                    let result = path.as_ref().map_err(Clone::clone).and_then(|path| {
                        validate_config(core, &exe, &runtime.root, path, &runtime.quitting)
                    });
                    let mut snapshot = runtime.state.lock().unwrap();
                    let state = snapshot.core_mut(core);
                    if state.selected_version.as_deref() != Some(&version)
                        || executable_stamp(&exe) != Some(exe_stamp)
                        || path
                            .as_ref()
                            .is_ok_and(|path| executable_stamp(path) != stamp)
                        || state.validation.get(&name) != Some(&ConfigValidation::Checking)
                    {
                        state.validation.remove(&name);
                        continue;
                    }
                    let value = match result {
                        Ok(()) => ConfigValidation::Valid,
                        Err(error) => {
                            for line in error.lines() {
                                logging::record_core(core, format!("[ERROR] 配置验证：{line}"));
                            }
                            ConfigValidation::Invalid(error)
                        }
                    };
                    state.validation.insert(name, value);
                }
                let mut snapshot = runtime.state.lock().unwrap();
                let state = snapshot.core_mut(core);
                state
                    .validation
                    .retain(|_, value| *value != ConfigValidation::Checking);
                state
                    .validation_stamps
                    .retain(|name, _| state.validation.contains_key(name));
                state.validation_busy = false;
            })
            .map_err(|error| {
                for name in names {
                    state.validation.remove(&name);
                }
                state.validation_busy = false;
                format!("创建配置验证线程失败: {error}")
            })?;
        Ok(())
    }

    /// 核心均已停止时返回可选的 DNS 恢复提示，失败的记录保留到下次启动。
    pub fn shutdown(&self) -> Result<Option<String>, String> {
        logging::info("runtime", "请求停止核心并退出");
        self.quitting.store(true, Ordering::Release);
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Request::Shutdown(tx))
            .map_err(|_| "核心管理线程已关闭".to_string())?;
        let result = rx
            .recv()
            .map_err(|_| "核心管理线程未返回退出结果".to_string())?;
        if result.is_err() {
            self.cancel_app_update();
        }
        result
    }

    pub fn prepare_app_update(&self) -> Result<(), String> {
        self.quitting.store(true, Ordering::Release);
        let (tx, rx) = mpsc::channel();
        let result = self
            .tx
            .send(Request::PrepareAppUpdate(tx))
            .map_err(|_| "核心管理线程已关闭".to_string())
            .and_then(|()| {
                rx.recv()
                    .map_err(|_| "核心管理线程未返回更新准备结果".to_string())
            })
            .and_then(|result| result);
        if result.is_err() {
            self.cancel_app_update();
        }
        result
    }

    pub fn cancel_app_update(&self) {
        self.quitting.store(false, Ordering::Release);
    }

    fn load_versions(&self, core: Core, more: bool) -> Result<(), String> {
        if self.quitting.load(Ordering::Acquire) {
            return Err("正在退出，无法获取版本列表".into());
        }
        let mut snapshot = self.state.lock().unwrap();
        let prefix = snapshot.settings.github_proxy.clone();
        let state = snapshot.core_mut(core);
        if state.versions_loading {
            return Err("正在获取版本列表".into());
        }
        if more && (!state.versions_loaded || !state.versions_has_more) {
            return Err("没有更多版本".into());
        }
        let page = if more { state.versions_page + 1 } else { 1 };
        state.versions_loading = true;
        state.versions_error = None;
        let runtime = self.clone();
        thread::Builder::new()
            .name(format!("{}-releases", core.label()))
            .spawn(move || {
                let result = update::fetch_releases(core, page, &prefix, &runtime.quitting);
                let mut snapshot = runtime.state.lock().unwrap();
                let state = snapshot.core_mut(core);
                state.versions_loading = false;
                if runtime.quitting.load(Ordering::Acquire) {
                    return;
                }
                match result {
                    Ok(releases) => {
                        if !more {
                            state.versions.retain(|version| {
                                version.installed
                                    || state.downloading_version.as_deref()
                                        == Some(version.version.as_str())
                            });
                        }
                        merge_versions(
                            &mut state.versions,
                            releases.versions,
                            state.selected_version.as_deref(),
                        );
                        state.versions_loaded = true;
                        state.versions_has_more = releases.has_more;
                        state.versions_page = page;
                        logging::info(
                            "update",
                            format_args!("{} 已获取第 {page} 页版本列表", core.label()),
                        );
                    }
                    Err(error) => {
                        logging::warn(
                            "update",
                            format_args!("{} 获取版本列表失败：{error}", core.label()),
                        );
                        state.versions_error =
                            Some(format!("{} 版本列表加载失败：{error}", core.label()));
                    }
                }
            })
            .map_err(|error| {
                state.versions_loading = false;
                format!("创建版本列表线程失败: {error}")
            })?;
        Ok(())
    }

    fn begin_download(&self, core: Core, version: &str) -> Result<(), String> {
        let mut snapshot = self.state.lock().unwrap();
        if self.quitting.load(Ordering::Acquire) {
            return Err("正在退出，无法开始下载".into());
        }
        let state = snapshot.core_mut(core);
        if state.update.is_some() || state.operation.is_some() {
            return Err(format!("{} 正在处理上一项操作", core.label()));
        }
        let release = state
            .versions
            .iter()
            .find(|release| {
                release.version == version && !release.installed && release.release_tag.is_some()
            })
            .cloned()
            .ok_or("请先获取版本列表并选择尚未下载的版本")?;
        state.update = Some(UpdateProgress::Checking);
        state.downloading_version = Some(version.into());
        state.update_error = None;
        logging::info(
            "update",
            format_args!("{} 开始下载版本 {version}", core.label()),
        );
        let runtime = self.clone();
        thread::Builder::new()
            .name(format!("{}-download", core.label()))
            .spawn(move || {
                let result = runtime.prepare_download(core, &release);
                if let Err(error) = result {
                    logging::warn("update", format_args!("{} 下载失败：{error}", core.label()));
                    let mut snapshot = runtime.state.lock().unwrap();
                    let state = snapshot.core_mut(core);
                    state.update = None;
                    state.downloading_version = None;
                    state.update_error = Some(format!(
                        "{} v{} 下载失败：{error}",
                        core.label(),
                        release.version
                    ));
                    if state.operation == Some(CoreOperation::Updating) {
                        state.operation = None;
                    }
                }
            })
            .map_err(|error| {
                state.update = None;
                state.downloading_version = None;
                format!("创建下载线程失败: {error}")
            })?;
        Ok(())
    }

    fn prepare_download(&self, core: Core, release: &CoreVersion) -> Result<(), String> {
        let prefix = self.snapshot().settings.github_proxy;
        let prepared = update::prepare_version(
            core,
            &self.root,
            release,
            &prefix,
            &self.quitting,
            |progress| {
                self.state.lock().unwrap().core_mut(core).update = Some(progress);
            },
        )?;
        self.queue_prepared(core, prepared)
    }

    fn queue_prepared(&self, core: Core, prepared: PreparedUpdate) -> Result<(), String> {
        loop {
            if self.quitting.load(Ordering::Acquire) {
                return Err("更新已取消，现有核心文件保持不变".into());
            }
            let mut snapshot = self.state.lock().unwrap();
            let state = snapshot.core_mut(core);
            if state.operation.is_none() {
                state.operation = Some(CoreOperation::Updating);
                state.update = Some(UpdateProgress::Installing);
                logging::info(
                    "update",
                    format_args!("{} 更新准备完成，等待安装", core.label()),
                );
                return self
                    .tx
                    .send(Request::Prepared(core, prepared))
                    .map_err(|_| "核心管理线程已关闭，更新未应用".into());
            }
            drop(snapshot);
            thread::sleep(Duration::from_millis(30));
        }
    }
}

fn merge_versions(
    existing: &mut Vec<CoreVersion>,
    versions: Vec<CoreVersion>,
    selected: Option<&str>,
) {
    for mut version in versions {
        if let Some(current) = existing
            .iter_mut()
            .find(|current| current.version == version.version)
        {
            version.installed |= current.installed;
            if version.release_tag.is_none() {
                version.release_tag = current.release_tag.clone();
                version.channel = current.channel;
            }
            *current = version;
        } else {
            existing.push(version);
        }
    }
    update::sort_versions(existing, selected);
}

fn list_configs(core: Core, dir: &Path) -> Result<Vec<String>, String> {
    let mut items = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("读取配置目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取配置文件信息失败: {e}"))?;
        let path = entry.path();
        if is_config(core, &path)
            && !is_runtime_config(&path)
            && path.is_file()
            && let Some(name) = entry.file_name().to_str()
        {
            items.push(name.to_owned());
        }
    }
    items.sort_by_cached_key(|name| (name.to_lowercase(), name.clone()));
    Ok(items)
}

fn is_json(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
}

fn is_config(core: Core, path: &Path) -> bool {
    is_json(path)
        || (core == Core::Mihomo
            && path
                .extension()
                .and_then(OsStr::to_str)
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
                }))
}

fn is_runtime_config(path: &Path) -> bool {
    is_config(Core::Mihomo, path)
        && path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(tun_config::RUNTIME_PREFIX))
}

fn cleanup_runtime_configs(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if is_runtime_config(&entry.path())
            && entry.file_type().is_ok_and(|kind| kind.is_file())
            && let Err(error) = fs::remove_file(entry.path())
        {
            logging::warn("tun", format_args!("删除遗留运行配置失败：{error}"));
        }
    }
}

fn executable_stamp(path: &Path) -> Option<(u64, Option<SystemTime>)> {
    fs::metadata(path)
        .ok()
        .filter(|file| file.is_file())
        .map(|file| (file.len(), file.modified().ok()))
}

fn readable_config(core: Core, root: &Path, name: &str) -> Result<PathBuf, String> {
    let config = config_path(core, root, name)?;
    let metadata = fs::File::open(&config)
        .and_then(|file| file.metadata())
        .map_err(|error| format!("无法读取配置文件，请检查文件是否存在及访问权限: {error}"))?;
    if !metadata.is_file() {
        return Err("所选路径不是配置文件".into());
    }
    Ok(config)
}

fn config_path(core: Core, root: &Path, name: &str) -> Result<PathBuf, String> {
    let name_path = Path::new(name);
    if !matches!(name_path.components().next(), Some(Component::Normal(_)))
        || name_path.file_name() != Some(OsStr::new(name))
        || name.contains(':')
        || !is_config(core, name_path)
        || is_runtime_config(name_path)
    {
        return Err("配置文件名无效".into());
    }
    Ok(core.config_dir(root).join(name))
}

fn validate_config(
    core: Core,
    exe: &Path,
    root: &Path,
    config: &Path,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    const OUTPUT_LIMIT: u64 = 64 * 1024;
    if cancelled.load(Ordering::Acquire) {
        return Err("配置验证已取消".into());
    }
    let mut output =
        tempfile::tempfile().map_err(|error| format!("创建验证输出文件失败: {error}"))?;
    let child = JobProcess::spawn_validation(core, exe, root, config, &output)
        .map_err(|error| format!("无法运行核心验证命令: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let result = loop {
        if cancelled.load(Ordering::Acquire) {
            break Err("配置验证已取消".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status.success()),
            Err(_) => break Err("无法读取核心验证状态".into()),
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            break Err("配置验证超时，请检查配置引用的资源是否可用".into());
        }
        if output
            .metadata()
            .is_ok_and(|metadata| metadata.len() > OUTPUT_LIMIT)
        {
            break Err("核心验证输出过多，已停止验证".into());
        }
        thread::sleep(Duration::from_millis(20));
    };
    child
        .terminate()
        .map_err(|_| "验证进程尚未完全退出，请重试")?;
    drop(child);
    output
        .seek(SeekFrom::Start(0))
        .map_err(|_| "无法读取验证输出")?;
    let mut bytes = Vec::new();
    output
        .take(OUTPUT_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取验证输出")?;
    let message = crate::core_output::sanitized_output(&bytes);
    match result {
        Ok(true) => Ok(()),
        Ok(false) if message.trim().is_empty() => Err("配置验证失败，核心未提供错误详情".into()),
        Ok(false) => Err(message),
        Err(error) if message.trim().is_empty() => Err(error),
        Err(error) => Err(format!("{error}\n{message}")),
    }
}

struct Worker {
    root: PathBuf,
    state: Arc<Mutex<Snapshot>>,
    saved: Saved,
    children: [Option<ManagedChild>; 3],
    dns: DesktopDns,
    quitting: Arc<AtomicBool>,
    version_files: [Option<(u64, Option<SystemTime>)>; 3],
}

impl Worker {
    fn apply_update(&mut self, core: Core, mut prepared: PreparedUpdate) {
        let result = if self.quitting.load(Ordering::Acquire) {
            Err("下载已取消，现有核心文件保持不变".into())
        } else {
            prepared.install()
        };
        self.refresh();
        {
            let mut snapshot = self.state.lock().unwrap();
            let state = snapshot.core_mut(core);
            state.operation = None;
            state.update = None;
            state.downloading_version = None;
            match result {
                Ok(()) => {
                    logging::info(
                        "update",
                        format_args!("{} 已下载版本 {}", core.label(), prepared.version),
                    );
                    state.update_error = None;
                }
                Err(error) => {
                    logging::error("update", format_args!("{} 安装失败：{error}", core.label()));
                    state.update_error = Some(error);
                }
            }
        }
    }

    fn run(mut self, rx: mpsc::Receiver<Request>) {
        self.cleanup_tun();
        for core in Core::ALL {
            let auto = match core {
                Core::SingBox => self.saved.settings.auto_sing_box,
                Core::Xray => self.saved.settings.auto_xray,
                Core::Mihomo => self.saved.settings.auto_mihomo,
            };
            if auto {
                logging::info("core", format_args!("{} 自动启动已启用", core.label()));
                self.action(Action::Start(core));
            }
        }
        self.refresh();
        let mut last_scan = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                #[cfg(test)]
                Ok(Request::Panic) => {
                    let _guard = self.state.lock().unwrap();
                    logging::with_locked_log(|| panic!("PRIVATE_WORKER_PANIC"));
                }
                Ok(Request::Action(action)) => self.action(action),
                Ok(Request::Prepared(core, prepared)) => self.apply_update(core, prepared),
                Ok(Request::SetTheme(theme, reply)) => {
                    logging::info("settings", format_args!("更改主题：{theme:?}"));
                    self.update_settings(|settings| settings.theme = theme);
                    let _ = reply.try_send(());
                }
                Ok(Request::SetGithubProxy(prefix, reply)) => {
                    self.update_settings(|settings| settings.github_proxy = prefix);
                    let _ = reply.try_send(());
                }
                Ok(Request::Shutdown(reply)) => {
                    let result = self.stop_all();
                    let done = result.is_ok();
                    let _ = reply.send(result);
                    if done {
                        break;
                    }
                }
                Ok(Request::PrepareAppUpdate(reply)) => {
                    let result = self
                        .stop_all()
                        .and_then(|warning| warning.map_or(Ok(()), Err));
                    let _ = reply.send(result);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    logging::warn("runtime", "核心管理请求通道已断开，停止托管核心");
                    let _ = self.stop_all();
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            for core in Core::ALL {
                self.poll(core);
            }
            if last_scan.elapsed() >= Duration::from_secs(2) {
                self.refresh();
                last_scan = Instant::now();
            }
        }
        logging::info("runtime", "核心管理线程已退出");
    }

    fn action(&mut self, action: Action) {
        let version_action = matches!(
            action,
            Action::SwitchVersion(..) | Action::DeleteVersion(..)
        );
        let operation = match &action {
            Action::Start(core) => Some((*core, "启动")),
            Action::Stop(core) => Some((*core, "停止")),
            Action::SwitchConfig(core, _) => Some((*core, "切换配置")),
            Action::DeleteConfig(core, _) => Some((*core, "删除配置")),
            Action::SwitchVersion(core, _) => Some((*core, "切换版本")),
            Action::DeleteVersion(core, _) => Some((*core, "删除版本")),
            _ => None,
        };
        if let Some((core, operation)) = operation {
            logging::info("core", format_args!("{} 开始{operation}", core.label()));
        }
        let (core, result) = match action {
            Action::RetryDns => {
                self.retry_dns(desktop_dns);
                return;
            }
            Action::Set(key, value) => {
                logging::info("settings", format_args!("更改设置 {key:?}={value}"));
                return self.update_settings(|settings| match key {
                    Setting::MinimizeToTray => settings.minimize_to_tray = value,
                    Setting::AutoSingBox => settings.auto_sing_box = value,
                    Setting::AutoXray => settings.auto_xray = value,
                    Setting::AutoMihomo => settings.auto_mihomo = value,
                    Setting::HijackDns => settings.hijack_dns = value,
                });
            }
            Action::Stop(core) => (core, self.stop(core)),
            Action::Start(core) => {
                let current = self.saved.current(core).clone();
                let result = match current {
                    None => Err("尚未选择配置，请在配置列表中点击“切换”".into()),
                    Some(name) if self.children[core as usize].is_none() => self
                        .readable_config(core, &name)
                        .and_then(|config| self.start(core, &config)),
                    Some(_) => Ok(()),
                };
                (core, result)
            }
            Action::SwitchConfig(core, name) => (core, self.switch_config(core, &name)),
            Action::DeleteConfig(core, name) => (core, self.delete_config(core, &name)),
            Action::SwitchVersion(core, version) => (core, self.switch_version(core, &version)),
            Action::DeleteVersion(core, version) => (core, self.delete_version(core, &version)),
            Action::RefreshVersions(_)
            | Action::ValidateConfig(..)
            | Action::ValidateAllConfigs(_)
            | Action::LoadMoreVersions(_)
            | Action::DownloadVersion(_, _) => unreachable!(),
        };
        let (_, operation) = operation.unwrap();
        match &result {
            Ok(()) => logging::info("core", format_args!("{} {operation}操作完成", core.label())),
            Err(error) => logging::error(
                "core",
                format_args!("{} {operation}失败：{error}", core.label()),
            ),
        }
        self.refresh();
        let mut snapshot = self.state.lock().unwrap();
        let status = snapshot.core_mut(core);
        status.operation = None;
        status.running = self.children[core as usize].is_some();
        status.current = self.saved.current(core).clone();
        if version_action {
            status.update_error = result.err();
        } else {
            status.run_error = result.err();
        }
    }

    fn update_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        let mut next = self.saved.clone();
        change(&mut next.settings);
        let saved = next.store(&self.root);
        let mut snapshot = self.state.lock().unwrap();
        match saved {
            Ok(()) => {
                logging::info("settings", "软件设置已保存");
                snapshot.settings = next.settings.clone();
                self.saved = next;
                snapshot.settings_error = None;
            }
            Err(e) => {
                logging::error("settings", format_args!("保存软件设置失败：{e}"));
                snapshot.settings_error = Some(format!("保存软件设置失败: {e}"));
            }
        }
    }

    fn switch_version(&mut self, core: Core, version: &str) -> Result<(), String> {
        let path = update::version_directory(core, &self.root, version)?;
        if !path.join(format!("{}.exe", core.label())).is_file() {
            return Err("所选版本尚未下载或核心文件已丢失".into());
        }
        if self.saved.selected_version(core).as_deref() == Some(version) {
            return Ok(());
        }
        let mut next = self.saved.clone();
        next.select_version(core, version.into());
        next.store(&self.root)
            .map_err(|error| format!("保存核心版本失败: {error}"))?;
        // 保存失败不能中断当前进程；停止失败时仍保留其运行版本，禁止误删。
        if let Err(error) = self.stop(core) {
            if let Err(save_error) = self.saved.store(&self.root) {
                return Err(format!("{error}；恢复版本选择失败: {save_error}"));
            }
            return Err(error);
        }
        self.saved = next;
        self.version_files[core as usize] = None;
        Ok(())
    }

    fn delete_version(&mut self, core: Core, version: &str) -> Result<(), String> {
        if self.saved.selected_version(core).as_deref() == Some(version) {
            return Err("当前使用的版本无法删除，请先切换到其他版本".into());
        }
        update::delete_version(core, &self.root, version)?;
        Ok(())
    }

    fn switch_config(&mut self, core: Core, name: &str) -> Result<(), String> {
        let config = self.readable_config(core, name)?;
        let mut next = self.saved.clone();
        match core {
            Core::SingBox => next.sing_box = Some(name.into()),
            Core::Xray => next.xray = Some(name.into()),
            Core::Mihomo => next.mihomo = Some(name.into()),
        }
        next.store(&self.root)
            .map_err(|e| format!("保存当前配置失败: {e}"))?;
        if let Err(error) = self.stop(core) {
            if let Err(save_error) = self.saved.store(&self.root) {
                return Err(format!("{error}；恢复配置选择失败: {save_error}"));
            }
            return Err(error);
        }
        self.saved = next;
        self.state.lock().unwrap().core_mut(core).current = Some(name.into());
        self.start(core, &config)
    }

    fn delete_config(&mut self, core: Core, name: &str) -> Result<(), String> {
        let config = self.readable_config(core, name)?;
        if self.saved.current(core).as_ref().is_some_and(|current| {
            current == name
                || same_file::is_same_file(&config, core.config_dir(&self.root).join(current))
                    .unwrap_or(false)
        }) {
            return Err("当前使用的配置无法删除，请先切换到其他配置".into());
        }
        fs::remove_file(config).map_err(|error| format!("删除配置文件失败: {error}"))
    }

    fn refresh(&mut self) {
        for core in Core::ALL {
            let result = list_configs(core, &core.config_dir(&self.root));
            let exe = self.saved.executable(core, &self.root).ok();
            let installed = exe.as_ref().is_some_and(|path| path.is_file());
            let local_versions = update::installed_versions(core, &self.root);
            let mut snapshot = self.state.lock().unwrap();
            let state = snapshot.core_mut(core);
            state.validation.retain(|name, _| {
                state.selected_version == *self.saved.selected_version(core)
                    && state.validation_stamps.get(name).copied()
                        == executable_stamp(&core.config_dir(&self.root).join(name))
            });
            state
                .validation_stamps
                .retain(|name, _| state.validation.contains_key(name));
            state.installed = installed;
            state.selected_version = self.saved.selected_version(core).clone();
            match local_versions {
                Ok(versions) => {
                    state.local_versions_error = None;
                    for release in &mut state.versions {
                        release.installed = false;
                    }
                    merge_versions(
                        &mut state.versions,
                        versions,
                        state.selected_version.as_deref(),
                    );
                    state
                        .versions
                        .retain(|release| release.installed || release.release_tag.is_some());
                }
                Err(error) => state.local_versions_error = Some(error),
            }
            if !installed {
                state.version = None;
                state.version_error = None;
            }
            match result {
                Ok(files) => {
                    if state.scan_error.is_some() {
                        logging::info(
                            "config",
                            format_args!("{} 配置目录读取已恢复", core.label()),
                        );
                    }
                    state.configs = files;
                    state.scan_error = None;
                }
                Err(e) => {
                    if state.scan_error.as_deref() != Some(e.as_str()) {
                        logging::error("config", format_args!("{} {e}", core.label()));
                    }
                    state.scan_error = Some(e);
                }
            }
            let stamp = exe.as_deref().and_then(executable_stamp);
            if self.version_files[core as usize] != stamp {
                state.validation.clear();
                state.validation_stamps.clear();
                self.version_files[core as usize] = stamp;
                state.version = None;
                state.version_error = None;
                if let Some(exe) = exe.filter(|_| stamp.is_some()) {
                    let snapshot = self.state.clone();
                    let quitting = self.quitting.clone();
                    let selected_version = state.selected_version.clone();
                    if let Err(error) = thread::Builder::new()
                        .name(format!("{}-version", core.label()))
                        .spawn(move || {
                            let result = update::read_version_for(core, &exe);
                            let mut snapshot = snapshot.lock().unwrap();
                            let state = snapshot.core_mut(core);
                            if !quitting.load(Ordering::Acquire)
                                && state.selected_version == selected_version
                                && executable_stamp(&exe) == stamp
                            {
                                match result {
                                    Ok(version) => {
                                        logging::info(
                                            "core",
                                            format_args!("{} 本地版本：{version}", core.label()),
                                        );
                                        state.version = Some(version);
                                    }
                                    Err(error) => {
                                        logging::warn(
                                            "core",
                                            format_args!("{} 版本检测失败：{error}", core.label()),
                                        );
                                        state.version_error = Some(error);
                                    }
                                }
                            }
                        })
                    {
                        logging::error(
                            "core",
                            format_args!("{} 创建版本检测线程失败：{error}", core.label()),
                        );
                        state.version_error = Some(format!("创建版本检测线程失败: {error}"));
                    }
                }
            }
        }
    }

    fn readable_config(&self, core: Core, name: &str) -> Result<PathBuf, String> {
        readable_config(core, &self.root, name)
    }

    fn start(&mut self, core: Core, config: &Path) -> Result<(), String> {
        self.state.lock().unwrap().core_mut(core).output_error = None;
        let exe = self.saved.executable(core, &self.root)?;
        let assets = exe.parent().expect("version executable has a parent");
        if !exe.is_file() {
            return Err(format!("未找到核心程序: {}", exe.display()));
        }
        let hijack = self.saved.settings.hijack_dns;
        if hijack && self.state.lock().unwrap().dns_recovery_error.is_some() {
            self.restore_dns()
                .map_err(|_| "DNS 尚未恢复，无法开始新的 DNS 劫持")?;
        }
        let config = tun_config::prepare(core, config)?;
        if !config.names().is_empty() {
            logging::info(
                "tun",
                format_args!(
                    "{} 已生成 {} 个随机 TUN 接口名",
                    core.label(),
                    config.names().len()
                ),
            );
            self.cleanup_tun();
        }
        let child = ManagedChild::spawn(core, &self.root, &exe, assets, config)
            .map_err(|e| format!("启动 {} 失败: {e}", core.label()))?;
        self.children[core as usize] = Some(child);
        logging::info("core", format_args!("{} 托管进程已创建", core.label()));
        if hijack {
            logging::info(
                "dns",
                format_args!("{} 请求劫持 DNS 到本地回环", core.label()),
            );
            if let Err(error) = self.dns.lease().and_then(DnsLease::apply) {
                logging::error("dns", format_args!("劫持 DNS 失败：{error}"));
                let stopped = self.stop_process(core);
                let _ = self.restore_dns();
                self.cleanup_tun();
                return match stopped {
                    Ok(_) => Err(error),
                    Err(cleanup) => Err(format!("{error}；{cleanup}")),
                };
            }
            logging::info("dns", "DNS 劫持完成");
        }
        Ok(())
    }

    fn stop(&mut self, core: Core) -> Result<(), String> {
        let running = self.children[core as usize].is_some();
        self.stop_process(core)?;
        if running {
            let _ = self.restore_dns();
            self.cleanup_tun();
        }
        Ok(())
    }

    fn restore_dns(&self) -> Result<(), String> {
        logging::info("dns", "开始恢复 DNS");
        let result = self.dns.lease().and_then(DnsLease::restore);
        match &result {
            Ok(()) => logging::info("dns", "DNS 恢复完成"),
            Err(error) => logging::error("dns", format_args!("恢复 DNS 失败：{error}")),
        }
        self.state.lock().unwrap().dns_recovery_error = result.as_ref().err().cloned();
        result
    }

    fn retry_dns(&mut self, reopen: impl FnOnce(&Path) -> (DesktopDns, Option<String>)) {
        logging::info("dns", "收到重试恢复 DNS 请求");
        // 排队期间另一个操作可能已恢复 DNS，甚至已开始新一轮劫持。
        if self.state.lock().unwrap().dns_recovery_error.is_some() {
            if matches!(self.dns, DesktopDns::Unavailable(_)) {
                let (dns, warning) = reopen(&self.root);
                self.dns = dns;
                crate::emergency::prepare(&self.dns);
                if let Some(warning) = &warning {
                    logging::warn("dns", warning);
                }
                self.state.lock().unwrap().dns_task_warning = warning;
            }
            let _ = self.restore_dns();
        }
        self.state.lock().unwrap().dns_recovery_busy = false;
    }

    fn stop_process(&mut self, core: Core) -> Result<CoreDiagnostics, String> {
        let owned = &mut self.children[core as usize];
        if let Some(child) = owned {
            child
                .terminate()
                .map_err(|e| format!("停止 {} 失败: {e}", core.label()))?;
        }
        let diagnostics = owned
            .take()
            .map(|child| {
                let diagnostics = child.output.finish();
                logging::info("core", format_args!("{} 托管进程树已停止", core.label()));
                diagnostics
            })
            .unwrap_or_default();
        let mut snapshot = self.state.lock().unwrap();
        let state = snapshot.core_mut(core);
        state.running = false;
        state.output_error = None;
        Ok(diagnostics)
    }

    fn cleanup_tun(&self) {
        let protected_names = self
            .children
            .iter()
            .flatten()
            .flat_map(|child| child.tun_names().iter().cloned())
            .collect::<Vec<_>>();
        tun_cleanup::cleanup(&protected_names);
    }

    fn stop_all(&mut self) -> Result<Option<String>, String> {
        let running = self.children.iter().flatten().count();
        let failures: Vec<_> = Core::ALL
            .into_iter()
            .filter_map(|core| self.stop_process(core).err())
            .collect();
        let previous_error = self.state.lock().unwrap().dns_recovery_error.clone();
        let dns_error = if running == 0 && previous_error.is_none() {
            let error = self
                .dns
                .lease()
                .and_then(DnsLease::restore_if_pending)
                .err();
            if let Some(error) = &error {
                logging::error("dns", format_args!("退出时检查待恢复 DNS 失败：{error}"));
            }
            error
        } else if failures.is_empty() || failures.len() < running {
            self.restore_dns().err()
        } else {
            previous_error
        };
        if failures.is_empty() {
            self.cleanup_tun();
            logging::info("runtime", "所有托管核心已停止");
            if dns_error.is_some() {
                logging::warn("dns", "退出时仍有 DNS 恢复失败，恢复记录保留");
            }
            Ok(dns_error)
        } else {
            let error = failures
                .into_iter()
                .chain(dns_error)
                .collect::<Vec<_>>()
                .join("；");
            logging::error("runtime", format_args!("停止托管核心失败：{error}"));
            Err(error)
        }
    }

    fn poll(&mut self, core: Core) {
        let Some(child) = &mut self.children[core as usize] else {
            return;
        };
        match child.try_wait() {
            Ok(None) => {
                self.state.lock().unwrap().core_mut(core).output_error = child.output.error();
                return;
            }
            Err(e) => {
                let message = format!("检查核心状态失败: {e}");
                let mut snapshot = self.state.lock().unwrap();
                let state = snapshot.core_mut(core);
                if state.run_error.as_deref() != Some(message.as_str()) {
                    logging::error("core", format_args!("{} {message}", core.label()));
                }
                state.run_error = Some(message);
                return;
            }
            Ok(Some(_)) => {}
        };
        let result = self.stop_process(core);
        if result.is_ok() {
            let _ = self.restore_dns();
            self.cleanup_tun();
        }
        let (message, output_error) = match result {
            Ok(diagnostics) => (
                format!(
                    "核心已退出。{}",
                    diagnostics
                        .reason
                        .unwrap_or("请检查配置内容及核心所需的资源文件")
                ),
                diagnostics.error,
            ),
            Err(e) => (format!("核心已退出。{e}"), None),
        };
        let mut snapshot = self.state.lock().unwrap();
        let state = snapshot.core_mut(core);
        if state.run_error.as_deref() != Some(message.as_str()) {
            logging::warn("core", format_args!("{} {message}", core.label()));
        }
        state.run_error = Some(message);
        state.output_error = output_error;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::process::{Child, Command};

    #[test]
    fn mihomo_config_extensions_and_saved_selections_are_independent() {
        let root = tempfile::tempdir().unwrap();
        for core in Core::ALL {
            fs::create_dir_all(core.config_dir(root.path())).unwrap();
        }
        for name in [
            "one.yaml",
            "two.YML",
            "three.JSON",
            "notes.txt",
            ".ladder-tun-hidden.yaml",
        ] {
            fs::write(Core::Mihomo.config_dir(root.path()).join(name), "{}").unwrap();
        }
        assert_eq!(
            list_configs(Core::Mihomo, &Core::Mihomo.config_dir(root.path())).unwrap(),
            ["one.yaml", "three.JSON", "two.YML"]
        );
        assert!(readable_config(Core::Mihomo, root.path(), "one.yaml").is_ok());
        for name in [
            "../outside.yaml",
            "one.yaml:stream",
            ".ladder-tun-hidden.yaml",
        ] {
            assert!(readable_config(Core::Mihomo, root.path(), name).is_err());
        }
        assert!(!is_config(Core::Xray, Path::new("config.yaml")));
        let mut saved = Saved {
            mihomo: Some("one.yaml".into()),
            ..Default::default()
        };
        saved.select_version(Core::Mihomo, "1.19.34".into());
        saved.settings.auto_mihomo = true;
        saved.store(root.path()).unwrap();
        let restored = Saved::load(root.path()).unwrap();
        assert_eq!(restored.current(Core::Mihomo).as_deref(), Some("one.yaml"));
        assert!(restored.settings.auto_mihomo);
        assert_eq!(restored.current(Core::SingBox), &None);
        assert_eq!(
            restored.selected_version(Core::Mihomo).as_deref(),
            Some("1.19.34")
        );
    }

    #[test]
    fn validation_command_returns_details_with_credentials_redacted() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("validation-fixture.rs");
        let exe = root.path().join("ladder-validation-fixture.exe");
        fs::write(
            &source,
            r#"fn main() {
            let args: Vec<_> = std::env::args().collect();
            assert!(args.iter().any(|arg| matches!(arg.as_str(), "check" | "-test" | "-t")));
            let content = std::fs::read_to_string(args.last().unwrap()).unwrap();
            if content == "invalid" {
                eprintln!("\x1b[31mERROR unknown field: broken\x1b[0m");
                eprintln!("password=DO_NOT_EXPOSE");
                std::process::exit(42);
            }
        }"#,
        )
        .unwrap();
        let built = Command::new("rustc")
            .arg(&source)
            .arg("--crate-name")
            .arg("ladder_validation_fixture")
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        assert!(built.status.success());
        let config = root.path().join("config.json");
        for core in Core::ALL {
            fs::write(&config, "valid").unwrap();
            assert!(
                validate_config(core, &exe, root.path(), &config, &AtomicBool::new(false)).is_ok()
            );
            fs::write(&config, "invalid").unwrap();
            let error = validate_config(core, &exe, root.path(), &config, &AtomicBool::new(false))
                .unwrap_err();
            assert!(error.contains("unknown field: broken"));
            assert!(error.contains("已隐藏敏感日志"));
            assert!(!error.contains("DO_NOT_EXPOSE"));
            assert!(!error.contains("42"));
            assert!(!error.contains('\x1b'));
        }
        assert_eq!(
            validate_config(
                Core::Mihomo,
                &exe,
                root.path(),
                &config,
                &AtomicBool::new(true)
            )
            .unwrap_err(),
            "配置验证已取消"
        );
    }

    #[test]
    fn validation_failures_for_locked_configs_survive_refresh() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        for core in Core::ALL {
            fs::create_dir_all(core.config_dir(root.path())).unwrap();
        }
        let core = Core::Mihomo;
        let exe = core.executable(root.path());
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"not executed").unwrap();
        let config = core.config_dir(root.path()).join("locked.yaml");
        fs::write(&config, "{}").unwrap();
        let _locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&config)
            .unwrap();
        let mut saved = Saved::default();
        saved.select_version(core, "1.0.0".into());
        let mut snapshot = Snapshot::default();
        snapshot.core_mut(core).selected_version = Some("1.0.0".into());
        snapshot.core_mut(core).configs = vec!["locked.yaml".into()];
        let state = Arc::new(Mutex::new(snapshot));
        let (tx, _rx) = mpsc::channel();
        let quitting = Arc::new(AtomicBool::new(false));
        let runtime = Runtime {
            root: root.path().into(),
            state: state.clone(),
            tx,
            quitting: quitting.clone(),
        };
        runtime.validate_configs(core, None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while runtime.snapshot().core(core).validation_busy {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(matches!(
            runtime.snapshot().core(core).validation.get("locked.yaml"),
            Some(ConfigValidation::Invalid(_))
        ));
        let mut worker = Worker {
            root: root.path().into(),
            state,
            saved,
            children: Default::default(),
            dns: DesktopDns::Portable(DnsLease::for_tests(root.path())),
            quitting,
            version_files: [None, None, executable_stamp(&exe)],
        };
        worker.refresh();
        assert!(matches!(
            runtime.snapshot().core(core).validation.get("locked.yaml"),
            Some(ConfigValidation::Invalid(_))
        ));
    }
    use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    fn idle(runtime: &Runtime, core: Core) -> CoreState {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let state = runtime.snapshot().core(core).clone();
            if state.operation.is_none() {
                return state;
            }
            assert!(Instant::now() < deadline, "等待核心操作超时: {state:?}");
            thread::sleep(Duration::from_millis(30));
        }
    }

    fn failed(runtime: &Runtime, core: Core) -> CoreState {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let state = runtime.snapshot().core(core).clone();
            if state.operation.is_none() && !state.running && state.run_error.is_some() {
                return state;
            }
            assert!(Instant::now() < deadline, "等待核心退出超时: {state:?}");
            thread::sleep(Duration::from_millis(30));
        }
    }

    fn stop_and_start(runtime: &Runtime, core: Core) {
        runtime.send(Action::Stop(core)).unwrap();
        let stopped = idle(runtime, core);
        assert!(
            !stopped.running && stopped.run_error.is_none(),
            "{stopped:?}"
        );
        runtime.send(Action::Start(core)).unwrap();
    }

    fn next_pid(path: &Path, previous: u32) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(pid) = fs::read_to_string(path)
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|pid| *pid != previous)
            {
                return pid;
            }
            assert!(Instant::now() < deadline, "等待测试核心启动超时");
            thread::sleep(Duration::from_millis(30));
        }
    }

    // 即使用例中途断言失败，也只清理此用例实际启动的进程。
    struct FixtureChild(Child);

    impl Drop for FixtureChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn lock_and_install_fixture_cores(root: &Path) -> std::sync::MutexGuard<'static, ()> {
        // 外部测试进程可能继承另一用例创建核心时短暂开放的管道写端，使其退出时等不到 EOF。
        static PROCESS_TESTS: Mutex<()> = Mutex::new(());
        let guard = PROCESS_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/core.rs");
        let exe = root.join("fixture.exe");
        let build = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        for core in Core::ALL {
            let dir = core.executable(root).parent().unwrap().to_owned();
            fs::create_dir_all(&dir).unwrap();
            fs::copy(&exe, dir.join(format!("{}.exe", core.label()))).unwrap();
            fs::create_dir_all(core.config_dir(root)).unwrap();
            let config = match core {
                Core::SingBox => "{ /* comment */ \"log\": {}, }",
                Core::Xray => "# comment\n{ \"log\": {} }",
                Core::Mihomo => "{\"mode\":\"direct\"}",
            };
            fs::write(core.config_dir(root).join("日常.JSON"), config).unwrap();
            fs::write(
                core.config_dir(root).join("失败.json"),
                r#"{"fixture_failure": true}"#,
            )
            .unwrap();
        }
        let rules = Core::Mihomo.config_dir(root).join("rules");
        fs::create_dir(&rules).unwrap();
        fs::write(rules.join("relative.yaml"), "fixture rule").unwrap();
        let mut saved = Saved::load(root).unwrap();
        for core in Core::ALL {
            saved.select_version(core, "1.0.0".into());
        }
        saved.store(root).unwrap();
        guard
    }

    #[test]
    fn live_core_errors_reach_snapshot_without_overwriting_actions_and_clear_on_new_runs() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let _process_test = lock_and_install_fixture_cores(root);
        for core in Core::ALL {
            fs::write(
                core.config_dir(root).join("live-error.json"),
                r#"{"fixture_live_error":true}"#,
            )
            .unwrap();
        }
        let runtime = Runtime::open(root.to_owned()).unwrap();
        let wait_error = |core: Core| {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let state = runtime.snapshot().core(core).clone();
                assert!(state.running, "core must still be running: {state:?}");
                if state.output_error.as_deref().is_some_and(|error| {
                    error.contains(&format!("fixture-live-output-{}", core.label()))
                }) {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "live output did not reach snapshot: {state:?}"
                );
                thread::sleep(Duration::from_millis(10));
            }
        };
        for core in Core::ALL {
            runtime
                .send(Action::SwitchConfig(core, "live-error.json".into()))
                .unwrap();
            assert!(idle(&runtime, core).running);
            wait_error(core);
            assert!(runtime.snapshot().core(core).run_error.is_none());
        }
        for core in Core::ALL {
            runtime
                .send(Action::SwitchConfig(core, "missing.json".into()))
                .unwrap();
            let state = idle(&runtime, core);
            assert!(state.running && state.run_error.is_some() && state.output_error.is_some());
            wait_error(core);
            assert!(runtime.snapshot().core(core).run_error.is_some());

            fs::write(core.config_dir(root).join("live-error.json"), "{}").unwrap();
            stop_and_start(&runtime, core);
            let state = idle(&runtime, core);
            assert!(state.running && state.run_error.is_none() && state.output_error.is_none());
            fs::write(
                core.config_dir(root).join("again.json"),
                r#"{"fixture_live_error":true}"#,
            )
            .unwrap();
            runtime
                .send(Action::SwitchConfig(core, "again.json".into()))
                .unwrap();
            assert!(idle(&runtime, core).running);
            wait_error(core);
            runtime
                .send(Action::SwitchConfig(core, "日常.JSON".into()))
                .unwrap();
            let state = idle(&runtime, core);
            assert!(state.running && state.output_error.is_none());
            runtime
                .send(Action::SwitchConfig(core, "again.json".into()))
                .unwrap();
            assert!(idle(&runtime, core).running);
            wait_error(core);
            runtime.send(Action::Stop(core)).unwrap();
            let state = idle(&runtime, core);
            assert!(!state.running && state.output_error.is_none());
            for other in Core::ALL.into_iter().filter(|other| *other != core) {
                let state = runtime.snapshot().core(other).clone();
                if state.running {
                    assert!(
                        state
                            .output_error
                            .as_deref()
                            .unwrap()
                            .contains(&format!("fixture-live-output-{}", other.label()))
                    );
                }
            }
        }
        runtime.shutdown().unwrap();
        assert!(
            !logging::recent()
                .iter()
                .any(|line| line.contains("fixture-live-output"))
        );
    }

    #[test]
    fn exited_core_errors_remain_visible_until_stop_or_new_run() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let _process_test = lock_and_install_fixture_cores(root);
        let config = r#"{"fixture_native_exit":true}"#;
        for core in Core::ALL {
            fs::write(core.config_dir(root).join("native-exit.json"), config).unwrap();
        }
        let runtime = Runtime::open(root.to_owned()).unwrap();
        for core in Core::ALL {
            runtime
                .send(Action::SwitchConfig(core, "native-exit.json".into()))
                .unwrap();
            let state = failed(&runtime, core);
            assert!(state.run_error.as_deref().unwrap().contains("核心已退出"));
            assert!(
                state.output_error.as_deref().is_some_and(
                    |error| error.contains(&format!("fixture-native-exit-{}", core.label()))
                ),
                "native exit diagnostic was lost: {state:?}"
            );
        }
        for core in Core::ALL {
            let peers = Core::ALL
                .into_iter()
                .filter(|other| *other != core)
                .map(|other| (other, runtime.snapshot().core(other).output_error.clone()))
                .collect::<Vec<_>>();
            runtime.send(Action::Stop(core)).unwrap();
            let state = idle(&runtime, core);
            assert!(!state.running && state.output_error.is_none() && state.run_error.is_none());
            runtime.send(Action::Start(core)).unwrap();
            assert!(failed(&runtime, core).output_error.is_some());
            fs::write(core.config_dir(root).join("native-exit.json"), "{}").unwrap();
            runtime.send(Action::Start(core)).unwrap();
            let state = idle(&runtime, core);
            assert!(state.running && state.output_error.is_none() && state.run_error.is_none());
            fs::write(core.config_dir(root).join("native-exit.json"), config).unwrap();
            stop_and_start(&runtime, core);
            assert!(failed(&runtime, core).output_error.is_some());
            runtime
                .send(Action::SwitchConfig(core, "日常.JSON".into()))
                .unwrap();
            let state = idle(&runtime, core);
            assert!(state.running && state.output_error.is_none());
            runtime.send(Action::Stop(core)).unwrap();
            assert!(idle(&runtime, core).output_error.is_none());
            for (other, expected) in peers {
                assert_eq!(runtime.snapshot().core(other).output_error, expected);
            }
        }
        runtime.shutdown().unwrap();
        assert!(
            !logging::recent()
                .iter()
                .any(|line| line.contains("fixture-native-exit"))
        );
    }

    // 真实子进程验证 Windows 句柄归属、带空格的路径和启动失败, 不操作网络接口。
    #[test]
    fn selection_survives_start_failure_retry_and_reconnect_without_affecting_other_processes() {
        let folder = tempfile::Builder::new()
            .prefix("梯子 runtime ")
            .tempdir()
            .unwrap();
        let root = folder.path();
        let _process_test = lock_and_install_fixture_cores(root);
        let mut external = FixtureChild(
            Command::new(Core::SingBox.executable(root))
                .arg(Core::SingBox.config_dir(root).join("日常.JSON"))
                .spawn()
                .unwrap(),
        );
        assert_eq!(
            next_pid(&Core::SingBox.config_dir(root).join("日常.pid"), 0),
            external.0.id()
        );
        let runtime = Runtime::open(root.to_owned()).unwrap();
        let mut pids = [external.0.id(), 0, 0];
        for core in Core::ALL {
            assert_eq!(runtime.snapshot().core(core).configs.len(), 2);
            assert!(runtime.snapshot().core(core).installed);
            runtime
                .send(Action::SwitchConfig(core, "日常.JSON".into()))
                .unwrap();
            let state = idle(&runtime, core);
            assert!(state.running, "{state:?}");
            assert_eq!(state.current.as_deref(), Some("日常.JSON"));
            pids[core as usize] =
                next_pid(&core.config_dir(root).join("日常.pid"), pids[core as usize]);
        }
        let blocked_save = root.join("ladder-state.json.tmp");
        fs::create_dir(&blocked_save).unwrap();
        let saved_before_restart = fs::read(root.join("ladder-state.json")).unwrap();
        for core in Core::ALL {
            stop_and_start(&runtime, core);
            let restarted = idle(&runtime, core);
            assert!(restarted.running, "{restarted:?}");
            assert!(restarted.run_error.is_none(), "{restarted:?}");
            assert_eq!(restarted.current.as_deref(), Some("日常.JSON"));
            pids[core as usize] =
                next_pid(&core.config_dir(root).join("日常.pid"), pids[core as usize]);
            for other in Core::ALL.into_iter().filter(|other| *other != core) {
                assert!(runtime.snapshot().core(other).running);
                assert_eq!(
                    fs::read_to_string(other.config_dir(root).join("日常.pid")).unwrap(),
                    pids[other as usize].to_string()
                );
            }
            assert!(external.0.try_wait().unwrap().is_none());
        }
        assert_eq!(
            fs::read(root.join("ladder-state.json")).unwrap(),
            saved_before_restart
        );
        runtime
            .send(Action::SwitchConfig(Core::SingBox, "失败.json".into()))
            .unwrap();
        let unchanged = idle(&runtime, Core::SingBox);
        assert!(unchanged.running);
        assert_eq!(unchanged.current.as_deref(), Some("日常.JSON"));
        assert!(unchanged.run_error.unwrap().contains("保存当前配置失败"));
        assert_eq!(
            Saved::load(root).unwrap().sing_box.as_deref(),
            Some("日常.JSON")
        );
        fs::remove_dir(blocked_save).unwrap();

        runtime
            .send(Action::SwitchConfig(Core::SingBox, "失败.json".into()))
            .unwrap();
        let stopped = failed(&runtime, Core::SingBox);
        assert!(!stopped.running);
        assert_eq!(stopped.current.as_deref(), Some("失败.json"));
        assert_eq!(
            Saved::load(root).unwrap().sing_box.as_deref(),
            Some("失败.json")
        );
        let error = stopped.run_error.unwrap();
        assert!(!error.contains("exit code"));
        assert!(error.contains("核心没有访问所需资源的权限"));
        assert!(!error.contains("PRIVATE_FIXTURE_VALUE"));
        let logs = logging::recent().join("\n");
        assert!(logs.contains("核心没有访问所需资源的权限"));
        assert!(!logs.contains("PRIVATE_FIXTURE_VALUE"));
        assert!(runtime.snapshot().xray.running);
        runtime.send(Action::Start(Core::SingBox)).unwrap();
        let retry = failed(&runtime, Core::SingBox);
        assert_eq!(retry.current.as_deref(), Some("失败.json"));
        assert!(
            retry
                .run_error
                .unwrap()
                .contains("核心没有访问所需资源的权限")
        );
        runtime
            .send(Action::Set(Setting::AutoSingBox, true))
            .unwrap();
        runtime
            .send(Action::Set(Setting::MinimizeToTray, true))
            .unwrap();
        let xray_dir = Core::Xray.config_dir(root);
        let moved_dir = root.join("移走的配置目录");
        fs::rename(&xray_dir, &moved_dir).unwrap();
        runtime
            .send(Action::SwitchConfig(Core::Xray, "失败.json".into()))
            .unwrap();
        let switch_without_file = idle(&runtime, Core::Xray);
        fs::rename(&moved_dir, &xray_dir).unwrap();
        assert_eq!(
            fs::read_to_string(xray_dir.join("日常.pid")).unwrap(),
            pids[Core::Xray as usize].to_string()
        );
        // 在模拟 Xray 自行退出时放入无法读取的记录，仅测试恢复触发与重试，不触碰网卡。
        let journal = root.join("dns-recovery.json");
        fs::write(&journal, b"[false]").unwrap();
        fs::write(xray_dir.join("日常.exit"), []).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let snapshot = runtime.snapshot();
            if !snapshot.xray.running
                && snapshot
                    .xray
                    .run_error
                    .as_deref()
                    .is_some_and(|e| e.contains("核心已退出"))
                && snapshot.dns_recovery_error.is_some()
            {
                break;
            }
            assert!(Instant::now() < deadline, "未收到 Xray 退出与 DNS 恢复结果");
            thread::sleep(Duration::from_millis(30));
        }
        fs::write(&journal, b"[]").unwrap();
        runtime.send(Action::RetryDns).unwrap();
        // 同一个工作线程依次处理重试和关闭。
        assert!(runtime.shutdown().unwrap().is_none());
        assert!(runtime.snapshot().dns_recovery_error.is_none());
        assert!(!journal.exists());
        assert!(external.0.try_wait().unwrap().is_none());

        let reopened = Runtime::open(root.to_owned()).unwrap();
        let restarted = failed(&reopened, Core::SingBox);
        assert_eq!(restarted.current.as_deref(), Some("失败.json"));
        assert!(
            restarted
                .run_error
                .unwrap()
                .contains("核心没有访问所需资源的权限")
        );
        assert!(!reopened.snapshot().xray.running);
        assert!(reopened.snapshot().settings.minimize_to_tray);
        reopened.shutdown().unwrap();
        assert!(switch_without_file.running, "{switch_without_file:?}");
        assert!(switch_without_file.run_error.is_some());
        assert_eq!(switch_without_file.current.as_deref(), Some("日常.JSON"));
        assert_eq!(
            Saved::load(root).unwrap().xray.as_deref(),
            Some("日常.JSON")
        );
    }

    #[test]
    fn mihomo_yaml_selection_autostart_and_failure_retry_preserve_relative_resources() {
        let folder = tempfile::Builder::new()
            .prefix("mihomo YAML ")
            .tempdir()
            .unwrap();
        let root = folder.path();
        let _process_test = lock_and_install_fixture_cores(root);
        let core = Core::Mihomo;
        let config_dir = core.config_dir(root);
        let yaml = "fixture_tree: true\nrule-providers:\n  local:\n    type: file\n    path: rules/relative.yaml\n";
        fs::write(config_dir.join("本地.yaml"), yaml).unwrap();
        fs::write(config_dir.join("失败.yml"), "fixture_failure: true\n").unwrap();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        runtime
            .send(Action::SwitchConfig(core, "日常.JSON".into()))
            .unwrap();
        assert!(idle(&runtime, core).running);
        let json_pid = next_pid(&config_dir.join("日常.pid"), 0);
        let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, json_pid) };
        assert!(!raw.is_null());
        let json_process = unsafe { OwnedHandle::from_raw_handle(raw) };
        runtime
            .send(Action::SwitchConfig(core, "本地.yaml".into()))
            .unwrap();
        let selected = idle(&runtime, core);
        assert!(selected.running, "{selected:?}");
        assert_eq!(selected.current.as_deref(), Some("本地.yaml"));
        let yaml_pid = next_pid(&config_dir.join("本地.pid"), 0);
        assert_eq!(
            unsafe { WaitForSingleObject(json_process.as_raw_handle(), 5000) },
            WAIT_OBJECT_0
        );
        runtime
            .send(Action::Set(Setting::AutoMihomo, true))
            .unwrap();
        runtime.shutdown().unwrap();

        let runtime = Runtime::open(root.to_owned()).unwrap();
        let started = idle(&runtime, core);
        assert!(started.running, "{started:?}");
        assert_eq!(started.current.as_deref(), Some("本地.yaml"));
        let autostart_pid = next_pid(&config_dir.join("本地.pid"), yaml_pid);
        let deadline = Instant::now() + Duration::from_secs(15);
        while runtime.snapshot().core(core).version.as_deref() != Some("1.0.0") {
            assert!(Instant::now() < deadline, "等待 mihomo -v 版本识别超时");
            thread::sleep(Duration::from_millis(30));
        }
        runtime
            .send(Action::SwitchConfig(core, "失败.yml".into()))
            .unwrap();
        for retry in [false, true] {
            if retry {
                runtime.send(Action::Start(core)).unwrap();
            }
            let state = failed(&runtime, core);
            assert_eq!(state.current.as_deref(), Some("失败.yml"));
            let error = state.run_error.unwrap();
            assert!(error.contains("核心没有访问所需资源的权限"));
            assert!(!error.contains("PRIVATE_FIXTURE_VALUE"));
            assert_eq!(
                Saved::load(root).unwrap().current(core).as_deref(),
                Some("失败.yml")
            );
        }
        runtime
            .send(Action::SwitchConfig(core, "本地.yaml".into()))
            .unwrap();
        assert!(idle(&runtime, core).running);
        next_pid(&config_dir.join("本地.pid"), autostart_pid);
        runtime.shutdown().unwrap();
        assert_eq!(
            fs::read_to_string(config_dir.join("本地.yaml")).unwrap(),
            yaml
        );
        assert_eq!(
            fs::read(config_dir.join("rules/relative.yaml")).unwrap(),
            b"fixture rule"
        );
    }

    #[test]
    #[ignore = "downloads and runs real cores; run only in Windows Sandbox or a VM"]
    fn official_stable_runtime_and_validation() {
        official_runtime_and_validation(update::UpdateChannel::Stable);
    }

    #[test]
    #[ignore = "downloads and runs real cores; run only in Windows Sandbox or a VM"]
    fn official_preview_runtime_and_validation() {
        official_runtime_and_validation(update::UpdateChannel::Preview);
    }

    fn official_runtime_and_validation(channel: update::UpdateChannel) {
        use serde_json::json;
        use std::net::{TcpListener, TcpStream};

        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let (cores, prefix) = update::official_test_options();
        let cancelled = AtomicBool::new(false);
        let mut saved = Saved::default();
        let mut fixtures = Vec::new();
        let mut validation_failures = Vec::new();
        for core in cores {
            let release = update::official_test_release(core, channel, &prefix);
            eprintln!(
                "{} {channel:?} {} {}: runtime download ({})",
                core.label(),
                release.version,
                std::env::consts::ARCH,
                if prefix.is_empty() {
                    "direct"
                } else {
                    "prefix"
                }
            );
            let mut prepared =
                update::prepare_version(core, root, &release, &prefix, &cancelled, |_| {})
                    .unwrap_or_else(|error| panic!("{}: {error}", core.label()));
            prepared.install().unwrap();
            saved.select_version(core, release.version.clone());
            let config_dir = core.config_dir(root);
            fs::create_dir_all(&config_dir).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            let valid = match core {
                Core::SingBox => json!({
                    "log": {"level": "info"},
                    "inbounds": [{"type": "mixed", "listen": "127.0.0.1", "listen_port": port}],
                    "outbounds": [{"type": "direct", "tag": "direct"}]
                }),
                Core::Xray => json!({
                    "log": {"loglevel": "info"},
                    "inbounds": [{"protocol": "socks", "listen": "127.0.0.1", "port": port,
                        "settings": {"auth": "noauth", "udp": false}}],
                    "outbounds": [{"protocol": "freedom", "tag": "direct"}]
                }),
                Core::Mihomo => {
                    fs::create_dir(config_dir.join("rules")).unwrap();
                    fs::write(
                        config_dir.join("rules/local.yaml"),
                        "payload:\n  - '+.example.invalid'\n",
                    )
                    .unwrap();
                    json!({
                        "mode": "rule", "log-level": "info", "mixed-port": port,
                        "allow-lan": false, "bind-address": "127.0.0.1",
                        "dns": {"enable": false}, "tun": {"enable": false},
                        "rule-providers": {"local": {"type": "file", "behavior": "domain",
                            "format": "yaml", "path": "rules/local.yaml"}},
                        "rules": ["RULE-SET,local,DIRECT", "MATCH,DIRECT"]
                    })
                }
            };
            let mut missing = valid.clone();
            match core {
                Core::SingBox => {
                    missing["route"] = json!({
                        "rule_set": [{"type": "local", "tag": "missing", "format": "source", "path": "ladder-missing.json"}],
                        "rules": [{"rule_set": ["missing"], "outbound": "direct"}]
                    })
                }
                Core::Xray => {
                    missing["routing"] = json!({
                        "rules": [{"type": "field", "domain": ["ext:ladder-missing.dat:test"], "outboundTag": "direct"}]
                    })
                }
                Core::Mihomo => {
                    missing["rule-providers"]["local"]["path"] = "rules/ladder-missing.yaml".into()
                }
            }
            let valid_text = serde_json::to_string(&valid).unwrap();
            let alternate = if core == Core::Mihomo {
                "alternate.yaml"
            } else {
                "alternate.json"
            };
            let alternate_text = if core == Core::Mihomo {
                serde_yaml_ng::to_string(&valid).unwrap()
            } else {
                serde_json::to_string_pretty(&valid).unwrap()
            };
            let files = [
                ("valid.json", valid_text),
                (alternate, alternate_text),
                ("syntax-error.json", "{".into()),
                (
                    "missing-resource.json",
                    serde_json::to_string(&missing).unwrap(),
                ),
            ];
            let exe = update::version_directory(core, root, &release.version)
                .unwrap()
                .join(format!("{}.exe", core.label()));
            let mut valid_configs = true;
            for (name, text) in &files {
                fs::write(config_dir.join(name), text).unwrap();
                let result = validate_config(core, &exe, root, &config_dir.join(name), &cancelled);
                eprintln!(
                    "{} {channel:?} {} {}: {name}: {result:?}",
                    core.label(),
                    release.version,
                    std::env::consts::ARCH
                );
                let expected_valid = *name == "valid.json" || *name == alternate;
                let matches_expectation = if expected_valid {
                    result.is_ok()
                } else if core == Core::Mihomo && *name == "missing-resource.json" && result.is_ok()
                {
                    eprintln!(
                        "mihomo {channel:?} {} {}: native -t accepts missing provider; runtime diagnosis required",
                        release.version,
                        std::env::consts::ARCH
                    );
                    true
                } else {
                    result.as_ref().err().is_some_and(|error| {
                        let infrastructure_failure = [
                            "配置验证已取消",
                            "创建验证输出文件失败",
                            "无法运行核心验证命令",
                            "无法读取核心验证状态",
                            "配置验证超时",
                            "核心验证输出过多",
                            "验证进程尚未完全退出",
                            "无法读取验证输出",
                        ]
                        .iter()
                        .any(|prefix| error.starts_with(prefix));
                        let missing_file = match core {
                            Core::SingBox => "ladder-missing.json",
                            Core::Xray => "ladder-missing.dat",
                            Core::Mihomo => "ladder-missing.yaml",
                        };
                        !infrastructure_failure
                            && (*name != "missing-resource.json" || error.contains(missing_file))
                    })
                };
                if !matches_expectation {
                    validation_failures.push(format!("{} {name}: {result:?}", core.label()));
                    if expected_valid {
                        valid_configs = false;
                        break;
                    }
                }
            }
            if valid_configs {
                fixtures.push((core, release.version, port, alternate, files));
            }
        }
        saved.store(root).unwrap();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        let ready = |runtime: &Runtime, core: Core, port: u16| {
            let deadline = Instant::now() + Duration::from_secs(15);
            let address = ([127, 0, 0, 1], port).into();
            loop {
                let state = runtime.snapshot().core(core).clone();
                assert!(
                    state.running && state.run_error.is_none(),
                    "{}: {state:?}",
                    core.label()
                );
                if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "{} listener did not start",
                    core.label()
                );
                thread::sleep(Duration::from_millis(30));
            }
        };
        for (core, _, port, _, _) in &fixtures {
            runtime
                .send(Action::SwitchConfig(*core, "valid.json".into()))
                .unwrap();
            assert!(idle(&runtime, *core).running);
            ready(&runtime, *core, *port);
        }
        for (core, version, port, alternate, files) in &fixtures {
            stop_and_start(&runtime, *core);
            assert!(idle(&runtime, *core).running);
            ready(&runtime, *core, *port);
            runtime
                .send(Action::SwitchConfig(*core, (*alternate).into()))
                .unwrap();
            assert_eq!(idle(&runtime, *core).current.as_deref(), Some(*alternate));
            ready(&runtime, *core, *port);
            runtime.send(Action::Stop(*core)).unwrap();
            assert!(!idle(&runtime, *core).running);
            drop(
                TcpListener::bind(("127.0.0.1", *port))
                    .expect("stopped core retained its listener"),
            );
            for (other, _, other_port, _, _) in &fixtures {
                if other != core {
                    ready(&runtime, *other, *other_port);
                }
            }
            runtime.send(Action::Start(*core)).unwrap();
            assert!(idle(&runtime, *core).running);
            ready(&runtime, *core, *port);
            for (name, text) in files {
                assert_eq!(
                    fs::read_to_string(core.config_dir(root).join(name)).unwrap(),
                    *text
                );
            }
            let auto = match core {
                Core::SingBox => Setting::AutoSingBox,
                Core::Xray => Setting::AutoXray,
                Core::Mihomo => Setting::AutoMihomo,
            };
            runtime.send(Action::Set(auto, true)).unwrap();
            eprintln!(
                "{} {channel:?} {version} {}: start/stop-start/config-switch/stop passed; {} peer listeners checked; DNS/TUN disabled",
                core.label(),
                std::env::consts::ARCH,
                fixtures.len() - 1
            );
        }
        runtime.shutdown().unwrap();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        for (core, version, port, alternate, _) in &fixtures {
            assert_eq!(idle(&runtime, *core).current.as_deref(), Some(*alternate));
            ready(&runtime, *core, *port);
            eprintln!(
                "{} {channel:?} {version} {}: autostart passed",
                core.label(),
                std::env::consts::ARCH
            );
        }
        runtime.shutdown().unwrap();
        assert!(
            validation_failures.is_empty(),
            "configuration validation expectations failed:\n{}",
            validation_failures.join("\n")
        );
    }

    #[test]
    fn tun_runtime_copies_follow_core_lifetimes_and_preserve_sources() {
        let folder = tempfile::Builder::new()
            .prefix("梯子 TUN ")
            .tempdir()
            .unwrap();
        let root = folder.path();
        let _process_test = lock_and_install_fixture_cores(root);
        let source_name = "TUN.json";
        let sources = [
            r#"{"fixture_tun":true,"tun":{"enable":true,"device":"original"}}"#,
            r#"{"fixture_tun":true,"inbounds":[{"type":"tun","interface_name":"original"}]}"#,
            r#"{"fixture_tun":true,"Inbounds":[{"Protocol":"TUN","Settings":{"Name":"original"}}]}"#,
        ];
        for (core, source) in Core::ALL.into_iter().zip(sources) {
            fs::write(core.config_dir(root).join(source_name), source).unwrap();
            fs::write(core.config_dir(root).join(".ladder-tun-stale.json"), "{}").unwrap();
        }
        let runtime = Runtime::open(root.to_owned()).unwrap();
        let copy_path = |core: Core, previous: &Path| {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                if let Ok(path) =
                    fs::read_to_string(root.join(format!("{}.config-path", core.label())))
                {
                    let path = PathBuf::from(path);
                    if path != previous && path.is_file() {
                        next_pid(&path.with_extension("pid"), 0);
                        return path;
                    }
                }
                assert!(Instant::now() < deadline, "等待 TUN 运行副本超时");
                thread::sleep(Duration::from_millis(30));
            }
        };
        let mut copies = [PathBuf::new(), PathBuf::new(), PathBuf::new()];
        for core in Core::ALL {
            let state = runtime.snapshot().core(core).clone();
            assert!(
                !core
                    .config_dir(root)
                    .join(".ladder-tun-stale.json")
                    .exists()
            );
            assert_eq!(state.configs.len(), 3);
            runtime
                .send(Action::SwitchConfig(core, source_name.into()))
                .unwrap();
            assert!(idle(&runtime, core).running);
            let copy = copy_path(core, Path::new(""));
            assert!(is_runtime_config(&copy));
            assert_eq!(copy.parent(), Some(core.config_dir(root).as_path()));
            assert!(
                !list_configs(core, &core.config_dir(root))
                    .unwrap()
                    .iter()
                    .any(|name| is_runtime_config(Path::new(name)))
            );
            copies[core as usize] = copy;
        }
        let mihomo_copy = &copies[Core::Mihomo as usize];
        let mihomo_before: serde_yaml_ng::Value =
            serde_yaml_ng::from_slice(&fs::read(mihomo_copy).unwrap()).unwrap();
        stop_and_start(&runtime, Core::Mihomo);
        assert!(idle(&runtime, Core::Mihomo).running);
        let mihomo_restarted = copy_path(Core::Mihomo, mihomo_copy);
        let mihomo_after: serde_yaml_ng::Value =
            serde_yaml_ng::from_slice(&fs::read(&mihomo_restarted).unwrap()).unwrap();
        assert_ne!(
            mihomo_before["tun"]["device"],
            mihomo_after["tun"]["device"]
        );
        assert!(!mihomo_copy.exists());
        fs::write(mihomo_restarted.with_extension("exit"), []).unwrap();
        failed(&runtime, Core::Mihomo);
        assert!(!mihomo_restarted.exists());
        fs::write(
            Core::Mihomo.config_dir(root).join("失败TUN.yaml"),
            "fixture_failure: true\ntun: { enable: true, device: original }\n",
        )
        .unwrap();
        runtime
            .send(Action::SwitchConfig(Core::Mihomo, "失败TUN.yaml".into()))
            .unwrap();
        failed(&runtime, Core::Mihomo);
        assert!(
            !fs::read_dir(Core::Mihomo.config_dir(root))
                .unwrap()
                .any(|entry| is_runtime_config(&entry.unwrap().path()))
        );
        runtime
            .send(Action::SwitchConfig(Core::Mihomo, source_name.into()))
            .unwrap();
        assert!(idle(&runtime, Core::Mihomo).running);
        let mihomo_shutdown_copy = copy_path(Core::Mihomo, &mihomo_restarted);
        let xray_before = fs::read(&copies[1]).unwrap();
        let sing_before: serde_json::Value =
            serde_json::from_slice(&fs::read(&copies[0]).unwrap()).unwrap();
        stop_and_start(&runtime, Core::SingBox);
        assert!(idle(&runtime, Core::SingBox).running);
        let restarted = copy_path(Core::SingBox, &copies[0]);
        let sing_after: serde_json::Value =
            serde_json::from_slice(&fs::read(&restarted).unwrap()).unwrap();
        assert_ne!(
            sing_before["inbounds"][0]["interface_name"],
            sing_after["inbounds"][0]["interface_name"]
        );
        assert!(!copies[0].exists());
        assert_eq!(fs::read(&copies[1]).unwrap(), xray_before);

        fs::write(restarted.with_extension("exit"), []).unwrap();
        failed(&runtime, Core::SingBox);
        assert!(!restarted.exists());
        assert!(runtime.snapshot().xray.running);

        fs::write(
            Core::SingBox.config_dir(root).join("失败TUN.json"),
            r#"{"fixture_failure":true,"inbounds":[{"type":"tun"}]}"#,
        )
        .unwrap();
        runtime
            .send(Action::SwitchConfig(Core::SingBox, "失败TUN.json".into()))
            .unwrap();
        failed(&runtime, Core::SingBox);
        assert!(
            !fs::read_dir(Core::SingBox.config_dir(root))
                .unwrap()
                .any(|entry| is_runtime_config(&entry.unwrap().path()))
        );

        runtime.send(Action::Stop(Core::Xray)).unwrap();
        assert!(!idle(&runtime, Core::Xray).running);
        assert!(!copies[1].exists());
        runtime.send(Action::Start(Core::Xray)).unwrap();
        assert!(idle(&runtime, Core::Xray).running);
        let shutdown_copy = copy_path(Core::Xray, &copies[1]);
        runtime.shutdown().unwrap();
        assert!(!shutdown_copy.exists());
        assert!(!mihomo_shutdown_copy.exists());
        let prepared =
            tun_config::prepare(Core::Xray, &Core::Xray.config_dir(root).join(source_name))
                .unwrap();
        let failed_spawn_copy = prepared.path().to_owned();
        assert!(
            ManagedChild::spawn(
                Core::Xray,
                root,
                &root.join("missing-core.exe"),
                &Core::Xray.core_dir(root),
                prepared,
            )
            .is_err()
        );
        assert!(!failed_spawn_copy.exists());
        for (core, source) in Core::ALL.into_iter().zip(sources) {
            assert_eq!(
                fs::read_to_string(core.config_dir(root).join(source_name)).unwrap(),
                source
            );
        }
    }

    #[test]
    fn managed_trees_end_on_core_exit_shutdown_and_forced_launcher_exit() {
        const CHILD_ROOT: &str = "LADDER_TEST_MANAGED_TREE_HOST";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let _runtime = Runtime::open(root.into()).unwrap();
            // 父测试强杀此宿主，跳过 Runtime::shutdown 以及 Rust 析构。
            thread::sleep(Duration::from_secs(45));
            return;
        }
        let folder = tempfile::Builder::new()
            .prefix("进程树 test ")
            .tempdir()
            .unwrap();
        let root = folder.path();
        let _process_test = lock_and_install_fixture_cores(root);
        let config = "进程树.json";
        for core in Core::ALL {
            fs::write(
                core.config_dir(root).join(config),
                r#"{"fixture_tree":true}"#,
            )
            .unwrap();
        }
        let saved = Saved {
            settings: Settings {
                auto_sing_box: true,
                auto_xray: true,
                auto_mihomo: true,
                ..Default::default()
            },
            sing_box: Some(config.into()),
            xray: Some(config.into()),
            mihomo: Some(config.into()),
            sing_box_version: Some("1.0.0".into()),
            xray_version: Some("1.0.0".into()),
            mihomo_version: Some("1.0.0".into()),
        };
        saved.store(root).unwrap();
        let tree = |core: Core| {
            ["pid", "child"].map(|ext| {
                let path = core.config_dir(root).join(config).with_extension(ext);
                let pid = next_pid(&path, 0);
                let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
                assert!(
                    !handle.is_null(),
                    "打开测试进程失败: {}",
                    io::Error::last_os_error()
                );
                fs::remove_file(path).unwrap();
                unsafe { OwnedHandle::from_raw_handle(handle) }
            })
        };
        let exited = |handles: &[OwnedHandle]| {
            for handle in handles {
                assert_eq!(
                    unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
                    WAIT_OBJECT_0
                );
            }
        };

        let runtime = Runtime::open(root.to_owned()).unwrap();
        let sing = tree(Core::SingBox);
        let xray = tree(Core::Xray);
        let mihomo = tree(Core::Mihomo);
        let signal = Core::SingBox
            .config_dir(root)
            .join(config)
            .with_extension("exit");
        fs::write(&signal, []).unwrap();
        assert!(failed(&runtime, Core::SingBox).run_error.is_some());
        exited(&sing);
        for handle in xray.iter().chain(&mihomo) {
            assert_eq!(
                unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) },
                WAIT_TIMEOUT
            );
        }
        runtime.shutdown().unwrap();
        exited(&xray);
        exited(&mihomo);
        fs::remove_file(signal).unwrap();

        let mut external = FixtureChild(
            Command::new(Core::Xray.executable(root))
                .arg(Core::Xray.config_dir(root).join("日常.JSON"))
                .spawn()
                .unwrap(),
        );
        let mut host = FixtureChild(Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime::tests::managed_trees_end_on_core_exit_shutdown_and_forced_launcher_exit",
            ])
            .env(CHILD_ROOT, root)
            .env("XRAY_LOCATION_ASSET", "fixture-old-value")
            .spawn()
            .unwrap());
        let sing = tree(Core::SingBox);
        let xray = tree(Core::Xray);
        let mihomo = tree(Core::Mihomo);
        host.0.kill().unwrap();
        host.0.wait().unwrap();
        exited(&sing);
        exited(&xray);
        exited(&mihomo);
        assert!(external.0.try_wait().unwrap().is_none());
    }

    #[test]
    fn fatal_panic_stops_owned_trees_and_restores_dns_without_application_locks() {
        const ROOT: &str = "LADDER_TEST_PANIC_ROOT";
        const SCENARIO: &str = "LADDER_TEST_PANIC_SCENARIO";
        if let Some(root) = std::env::var_os(ROOT) {
            let root = PathBuf::from(root);
            let scenario = std::env::var(SCENARIO).unwrap();
            logging::init(&root.join("app.log")).unwrap();
            logging::install_panic_hook();
            crate::emergency::install_panic_hook();
            // 模拟生产 Portable 会话，直到强制结束进程才交还锁。
            let session = fs::File::create(root.join("dns-session.lock")).unwrap();
            session.lock().unwrap();
            let runtime = Runtime::open(root.clone()).unwrap();
            for core in Core::ALL {
                assert!(idle(&runtime, core).running);
            }
            crate::emergency::prepare_test_helper(scenario == "spawn-failure");
            if scenario == "restore-failure" {
                fs::write(root.join("dns-recovery.json"), b"broken journal").unwrap();
            }
            fs::write(root.join("ready"), []).unwrap();
            while !root.join("panic-now").exists() {
                thread::sleep(Duration::from_millis(10));
            }
            if scenario == "worker" {
                runtime.tx.send(Request::Panic).unwrap();
                thread::sleep(Duration::from_secs(30));
                panic!("核心管理线程未终止进程");
            }
            extern "system" fn ui_callback() {
                panic!("PRIVATE_UI_PANIC");
            }
            let _guard = runtime.state.lock().unwrap();
            logging::with_locked_log(|| ui_callback());
            unreachable!();
        }

        for scenario in ["ui", "worker", "spawn-failure", "restore-failure"] {
            let folder = tempfile::Builder::new()
                .prefix("panic 恢复 ")
                .tempdir()
                .unwrap();
            let root = folder.path();
            let _process_test = lock_and_install_fixture_cores(root);
            for core in Core::ALL {
                fs::write(
                    core.config_dir(root).join("进程树.json"),
                    r#"{"fixture_tree":true}"#,
                )
                .unwrap();
            }
            Saved {
                settings: Settings {
                    auto_sing_box: true,
                    auto_xray: true,
                    auto_mihomo: true,
                    hijack_dns: true,
                    ..Default::default()
                },
                sing_box: Some("进程树.json".into()),
                xray: Some("进程树.json".into()),
                mihomo: Some("进程树.json".into()),
                sing_box_version: Some("1.0.0".into()),
                xray_version: Some("1.0.0".into()),
                mihomo_version: Some("1.0.0".into()),
            }
            .store(root)
            .unwrap();
            let mut external = FixtureChild(
                Command::new(Core::Xray.executable(root))
                    .arg(Core::Xray.config_dir(root).join("日常.JSON"))
                    .spawn()
                    .unwrap(),
            );
            let mut host = FixtureChild(
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "runtime::tests::fatal_panic_stops_owned_trees_and_restores_dns_without_application_locks", "--nocapture"])
                    .env(ROOT, root).env(SCENARIO, scenario)
                    .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
                    .spawn().unwrap(),
            );
            let deadline = Instant::now() + Duration::from_secs(20);
            while !root.join("ready").exists() {
                assert!(
                    host.0.try_wait().unwrap().is_none(),
                    "{scenario}: 宿主提前退出"
                );
                assert!(Instant::now() < deadline, "{scenario}: 宿主启动超时");
                thread::sleep(Duration::from_millis(20));
            }
            let mut handles = Vec::new();
            for core in Core::ALL {
                for extension in ["pid", "child"] {
                    let path = core
                        .config_dir(root)
                        .join("进程树.json")
                        .with_extension(extension);
                    let pid = next_pid(&path, 0);
                    let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
                    assert!(!raw.is_null());
                    handles.push(unsafe { OwnedHandle::from_raw_handle(raw) });
                }
            }
            fs::write(root.join("panic-now"), []).unwrap();
            while host.0.try_wait().unwrap().is_none() {
                assert!(Instant::now() < deadline, "{scenario}: panic 清理死锁");
                thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(host.0.wait().unwrap().code(), Some(1), "{scenario}");
            for handle in handles {
                assert_eq!(
                    unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
                    WAIT_OBJECT_0
                );
            }
            assert!(
                external.0.try_wait().unwrap().is_none(),
                "不得停止其他软件的同名核心"
            );
            let journal = root.join("dns-recovery.json");
            if scenario == "spawn-failure" {
                assert!(journal.is_file(), "助手创建失败时必须保留恢复记录");
                assert!(!root.join("recovery-result").exists());
            } else {
                let result = root.join("recovery-result");
                while !result.exists() {
                    assert!(Instant::now() < deadline, "{scenario}: 助手未完成恢复");
                    thread::sleep(Duration::from_millis(20));
                }
                if scenario == "restore-failure" {
                    assert_eq!(fs::read_to_string(result).unwrap(), "failed");
                    assert_eq!(fs::read(journal).unwrap(), b"broken journal");
                } else {
                    assert_eq!(fs::read_to_string(result).unwrap(), "ok");
                    assert!(!journal.exists(), "{scenario}: DNS 快照未交还");
                }
            }
            assert!(
                !fs::read_to_string(root.join("app.log"))
                    .unwrap()
                    .contains("PRIVATE_")
            );
        }
    }

    #[test]
    fn startup_reports_both_invalid_settings_and_pending_dns() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        fs::write(root.join("dns-recovery.json"), b"[false]").unwrap();
        fs::write(root.join("ladder-state.json"), b"false").unwrap();
        let error = Runtime::open(root.to_owned()).err().unwrap();
        assert!(error.contains("ladder-state.json"), "{error}");
        assert!(error.contains("DNS 尚未恢复"), "{error}");
        assert!(error.contains("dns-recovery.json"), "{error}");
        assert!(root.join("dns-recovery.json").exists());
    }

    #[test]
    fn ordinary_xray_can_start_while_an_old_dns_record_is_unavailable() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let _process_test = lock_and_install_fixture_cores(root);
        fs::write(root.join("dns-recovery.json"), b"[false]").unwrap();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        runtime
            .send(Action::SwitchConfig(Core::Xray, "日常.JSON".into()))
            .unwrap();
        let state = idle(&runtime, Core::Xray);
        assert!(state.running, "{state:?}");
        assert!(runtime.snapshot().dns_recovery_error.is_some());
        assert!(runtime.shutdown().unwrap().is_some());
    }

    #[test]
    fn unavailable_machine_session_keeps_ordinary_cores_usable_and_preserves_records() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let _process_test = lock_and_install_fixture_cores(root);
        let journal = root.join("dns-recovery.json");
        let bytes = b"unreadable test record";
        fs::write(&journal, bytes).unwrap();
        let error = "恢复目录权限不符合要求";
        let runtime =
            Runtime::open_with_dns(root.into(), DesktopDns::Unavailable(error.into()), None)
                .unwrap();
        assert_eq!(
            runtime.snapshot().dns_recovery_error.as_deref(),
            Some(error)
        );
        runtime
            .send(Action::SwitchConfig(Core::Xray, "日常.JSON".into()))
            .unwrap();
        assert!(idle(&runtime, Core::Xray).running);
        runtime.send(Action::Set(Setting::HijackDns, true)).unwrap();
        runtime
            .send(Action::SwitchConfig(Core::SingBox, "日常.JSON".into()))
            .unwrap();
        let refused = idle(&runtime, Core::SingBox);
        assert!(!refused.running && refused.run_error.is_some());
        assert!(runtime.snapshot().xray.running);
        assert_eq!(runtime.shutdown().unwrap().as_deref(), Some(error));
        assert_eq!(fs::read(journal).unwrap(), bytes);
    }

    #[test]
    fn uninstalled_adapter_record_does_not_prevent_xray_autostart() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let _process_test = lock_and_install_fixture_cores(root);
        let journal = root.join("dns-recovery.json");
        fs::write(
            &journal,
            br#"[{"target":{"guid":42,"ipv6":false},"previous":""}]"#,
        )
        .unwrap();
        let saved = Saved {
            xray: Some("日常.JSON".into()),
            xray_version: Some("1.0.0".into()),
            settings: Settings {
                auto_xray: true,
                ..Default::default()
            },
            ..Default::default()
        };
        saved.store(root).unwrap();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        assert!(idle(&runtime, Core::Xray).running);
        assert!(runtime.snapshot().dns_recovery_error.is_none());
        assert!(!journal.exists());
        assert!(runtime.shutdown().unwrap().is_none());
    }

    #[test]
    fn a_queued_dns_retry_leaves_a_newly_started_session_alone() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let mut worker = Worker {
            root: root.to_owned(),
            state: Arc::new(Mutex::new(Snapshot {
                dns_recovery_busy: true,
                ..Default::default()
            })),
            saved: Saved::default(),
            children: Default::default(),
            dns: DesktopDns::Portable(DnsLease::for_tests(root)),
            quitting: Arc::new(AtomicBool::new(false)),
            version_files: Default::default(),
        };
        // 前一个请求已成功重试并启动核心，排队的重试不应恢复新建的快照。
        worker.dns.lease().unwrap().apply().unwrap();
        let journal = root.join("dns-recovery.json");
        let active = fs::read(&journal).unwrap();
        worker.action(Action::RetryDns);
        assert_eq!(fs::read(journal).unwrap(), active);
        assert!(!worker.state.lock().unwrap().dns_recovery_busy);
    }

    #[test]
    fn retry_after_directory_repair_reopens_once_then_uses_the_same_lease() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let mut worker = Worker {
            root: root.into(),
            state: Arc::new(Mutex::new(Snapshot {
                dns_recovery_error: Some("目录不可用".into()),
                ..Default::default()
            })),
            saved: Saved::default(),
            children: Default::default(),
            dns: DesktopDns::Unavailable("目录不可用".into()),
            quitting: Arc::new(AtomicBool::new(false)),
            version_files: Default::default(),
        };
        let journal = root.join("dns-recovery.json");
        fs::write(&journal, b"invalid").unwrap();
        worker.retry_dns(|path| {
            assert_eq!(path, root);
            (
                DesktopDns::Portable(DnsLease::for_tests(path)),
                Some("开机恢复不可用".into()),
            )
        });
        assert!(worker.state.lock().unwrap().dns_recovery_error.is_some());
        fs::write(&journal, b"[]").unwrap();
        worker.retry_dns(|_| panic!("已取得会话，不应重新注册任务"));
        let state = worker.state.lock().unwrap();
        assert_eq!(state.dns_task_warning.as_deref(), Some("开机恢复不可用"));
        assert!(state.dns_recovery_error.is_none() && !state.dns_recovery_busy);
        assert!(!journal.exists());
    }

    #[test]
    fn every_core_can_start_hijacking_and_each_actual_exit_restores_dns() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let _process_test = lock_and_install_fixture_cores(root);
        let runtime = Runtime::open(root.to_owned()).unwrap();
        let journal = root.join("dns-recovery.json");
        runtime.send(Action::Set(Setting::HijackDns, true)).unwrap();
        for core in Core::ALL {
            runtime
                .send(Action::SwitchConfig(core, "日常.JSON".into()))
                .unwrap();
            let state = idle(&runtime, core);
            assert!(state.running && state.run_error.is_none(), "{state:?}");
            assert!(journal.is_file(), "{} 启动后缺少 DNS 原值", core.label());
        }
        for core in Core::ALL {
            // 关闭开关只影响下一次启动。当前会话仍应在任意核心退出时恢复。
            runtime
                .send(Action::Set(Setting::HijackDns, false))
                .unwrap();
            runtime.send(Action::Stop(core)).unwrap();
            assert!(!idle(&runtime, core).running);
            assert!(!journal.exists());
            runtime.send(Action::Start(core)).unwrap();
            assert!(idle(&runtime, core).running);
            assert!(!journal.exists());

            runtime.send(Action::Set(Setting::HijackDns, true)).unwrap();
            stop_and_start(&runtime, core);
            assert!(idle(&runtime, core).running);
            assert!(journal.exists());
            let signal = core.config_dir(root).join("日常.exit");
            fs::write(&signal, []).unwrap();
            assert!(
                failed(&runtime, core)
                    .run_error
                    .unwrap()
                    .contains("核心已退出")
            );
            assert!(!journal.exists());
            for other in Core::ALL.into_iter().filter(|other| *other != core) {
                assert!(runtime.snapshot().core(other).running);
            }
            fs::remove_file(signal).unwrap();
            runtime.send(Action::Start(core)).unwrap();
            assert!(idle(&runtime, core).running);
            assert!(journal.exists());
        }
        assert!(runtime.shutdown().unwrap().is_none());
    }

    #[test]
    fn pending_dns_recovery_survives_shutdown_and_can_be_retried_after_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let journal = root.join("dns-recovery.json");
        let damaged = br#"[{"target":"PRIVATE_FIXTURE_VALUE","previous":""}]"#;
        fs::write(&journal, damaged).unwrap();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        let error = runtime.snapshot().dns_recovery_error.unwrap();
        assert!(error.contains("dns-recovery.json"));
        assert!(!error.contains("PRIVATE_FIXTURE_VALUE"));
        assert!(
            !logging::recent()
                .iter()
                .any(|line| line.contains("PRIVATE_FIXTURE_VALUE"))
        );
        runtime
            .set_theme(ThemePreference::Dark)
            .unwrap()
            .recv_blocking()
            .unwrap();
        assert_eq!(
            runtime.snapshot().dns_recovery_error.as_deref(),
            Some(error.as_str())
        );
        assert_eq!(runtime.shutdown().unwrap().as_deref(), Some(error.as_str()));
        assert_eq!(fs::read(&journal).unwrap(), damaged);

        let runtime = Runtime::open(root.to_owned()).unwrap();
        assert!(runtime.snapshot().dns_recovery_error.is_some());
        fs::write(&journal, b"[]").unwrap();
        runtime.send(Action::RetryDns).unwrap();
        runtime
            .set_theme(ThemePreference::Light)
            .unwrap()
            .recv_blocking()
            .unwrap();
        let snapshot = runtime.snapshot();
        assert!(snapshot.dns_recovery_error.is_none());
        assert!(!snapshot.dns_recovery_busy);
        assert!(!journal.exists());
        assert!(runtime.shutdown().unwrap().is_none());
    }

    #[test]
    fn queued_operations_stay_visible_until_handled_and_reject_duplicate_requests() {
        let folder = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let runtime = Runtime {
            root: folder.path().to_owned(),
            state: Arc::new(Mutex::new(Snapshot::default())),
            tx,
            quitting: Arc::new(AtomicBool::new(false)),
        };
        let mut worker = Worker {
            root: runtime.root.clone(),
            state: runtime.state.clone(),
            saved: Saved::default(),
            children: Default::default(),
            dns: DesktopDns::Portable(DnsLease::for_tests(folder.path())),
            quitting: Arc::new(AtomicBool::new(false)),
            version_files: Default::default(),
        };
        let core = Core::SingBox;
        for (action, operation) in [
            (Action::Start(core), CoreOperation::Starting),
            (Action::Stop(core), CoreOperation::Stopping),
            (
                Action::SwitchConfig(core, "选中.json".into()),
                CoreOperation::Switching,
            ),
            (
                Action::DeleteConfig(core, "删除.json".into()),
                CoreOperation::Switching,
            ),
        ] {
            runtime.send(action).unwrap();
            assert!(runtime.send(Action::Stop(core)).is_err());
            assert_eq!(runtime.snapshot().core(core).operation, Some(operation));
            let Request::Action(queued) = rx.try_recv().unwrap() else {
                panic!("预期收到核心操作");
            };
            worker.action(queued);
            assert!(runtime.snapshot().core(core).operation.is_none());
            assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
        }
        drop(rx);
        assert!(runtime.send(Action::Start(core)).is_err());
        assert!(runtime.snapshot().core(core).operation.is_none());
    }

    #[test]
    fn incomplete_state_is_rejected_without_rewriting_it() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("ladder-state.json");
        for data in [
            "{}",
            r#"{"settings":{"minimize_to_tray":true},"sing_box":"日常.json"}"#,
        ] {
            fs::write(&path, data).unwrap();
            let error = Saved::load(folder.path()).err().unwrap();
            assert!(error.contains("ladder-state.json 格式无效"));
            assert_eq!(fs::read_to_string(&path).unwrap(), data);
        }
    }

    #[test]
    fn fresh_state_defaults_and_selections_survive_settings_changes() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let defaults = Saved::load(root).unwrap().settings;
        assert_eq!(defaults.theme, ThemePreference::System);
        assert!(!defaults.hijack_dns);
        let path = root.join("ladder-state.json");
        Saved {
            settings: Settings {
                minimize_to_tray: true,
                ..Settings::default()
            },
            sing_box: Some("日常.json".into()),
            xray: None,
            ..Default::default()
        }
        .store(root)
        .unwrap();
        let saved = fs::read(&path).unwrap();
        let mut runtime = Runtime::open(root.to_owned()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), saved);
        assert_eq!(runtime.snapshot().settings.theme, ThemePreference::System);
        assert!(!runtime.snapshot().settings.hijack_dns);
        runtime.send(Action::Set(Setting::HijackDns, true)).unwrap();
        for (theme, stored) in [
            (ThemePreference::Dark, "dark"),
            (ThemePreference::Light, "light"),
            (ThemePreference::System, "system"),
        ] {
            runtime.set_theme(theme).unwrap().recv_blocking().unwrap();
            assert_eq!(runtime.snapshot().settings.theme, theme);
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(value["settings"]["theme"], stored);
            runtime.shutdown().unwrap();
            runtime = Runtime::open(root.to_owned()).unwrap();
            let snapshot = runtime.snapshot();
            assert_eq!(snapshot.settings.theme, theme);
            assert!(snapshot.settings.minimize_to_tray);
            assert!(snapshot.settings.hijack_dns);
            assert_eq!(snapshot.sing_box.current.as_deref(), Some("日常.json"));
        }
        runtime
            .send(Action::Set(Setting::HijackDns, false))
            .unwrap();
        runtime.shutdown().unwrap();
        assert!(!Saved::load(root).unwrap().settings.hijack_dns);
    }

    #[test]
    fn theme_saves_follow_request_order_and_keep_the_last_successful_choice_on_failure() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        drop(runtime.set_theme(ThemePreference::Dark).unwrap());
        runtime
            .set_theme(ThemePreference::Light)
            .unwrap()
            .recv_blocking()
            .unwrap();
        assert_eq!(runtime.snapshot().settings.theme, ThemePreference::Light);
        assert_eq!(
            Saved::load(root).unwrap().settings.theme,
            ThemePreference::Light
        );
        let path = root.join("ladder-state.json");
        let previous = fs::read(&path).unwrap();
        fs::create_dir(root.join("ladder-state.json.tmp")).unwrap();
        runtime
            .set_theme(ThemePreference::Dark)
            .unwrap()
            .recv_blocking()
            .unwrap();
        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.settings.theme, ThemePreference::Light);
        assert!(
            snapshot
                .settings_error
                .unwrap()
                .contains("保存软件设置失败")
        );
        assert_eq!(fs::read(path).unwrap(), previous);
        runtime.shutdown().unwrap();
    }

    #[test]
    fn version_selection_is_persisted_and_save_failure_keeps_process_running() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let _guard = lock_and_install_fixture_cores(root);
        let core = Core::SingBox;
        let source = fs::read(core.executable(root)).unwrap();
        let mut prepared =
            PreparedUpdate::for_test(core, root, "0.9.0", &[("sing-box.exe", &source)]);
        prepared.install().unwrap();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        runtime
            .send(Action::SwitchConfig(core, "日常.JSON".into()))
            .unwrap();
        assert!(idle(&runtime, core).running);
        fs::create_dir(root.join("ladder-state.json.tmp")).unwrap();
        runtime
            .send(Action::SwitchVersion(core, "0.9.0".into()))
            .unwrap();
        let unchanged = idle(&runtime, core);
        assert!(unchanged.running);
        assert_eq!(unchanged.selected_version.as_deref(), Some("1.0.0"));
        assert!(unchanged.update_error.unwrap().contains("保存核心版本失败"));
        fs::remove_dir(root.join("ladder-state.json.tmp")).unwrap();
        runtime
            .send(Action::SwitchVersion(core, "0.9.0".into()))
            .unwrap();
        let switched = idle(&runtime, core);
        assert!(!switched.running);
        assert_eq!(switched.selected_version.as_deref(), Some("0.9.0"));
        assert_eq!(switched.current.as_deref(), Some("日常.JSON"));
        runtime.shutdown().unwrap();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        assert_eq!(
            runtime.snapshot().sing_box.selected_version.as_deref(),
            Some("0.9.0")
        );
        assert_eq!(
            runtime.snapshot().xray.selected_version.as_deref(),
            Some("1.0.0")
        );
        runtime.shutdown().unwrap();
    }

    #[test]
    fn merging_release_pages_keeps_local_versions_and_official_preview_labels() {
        use crate::update::UpdateChannel;
        let version = |value: &str, installed, channel, tag: Option<&str>| CoreVersion {
            version: value.into(),
            installed,
            channel,
            release_tag: tag.map(str::to_owned),
        };
        let mut versions = vec![version("1.0.0", true, UpdateChannel::Stable, None)];
        merge_versions(
            &mut versions,
            vec![
                version("1.1.0", false, UpdateChannel::Preview, Some("v1.1.0")),
                version("1.0.0", false, UpdateChannel::Stable, Some("v1.0.0")),
            ],
            Some("1.0.0"),
        );
        assert_eq!(versions[0].version, "1.0.0");
        merge_versions(
            &mut versions,
            vec![version("1.1.0", true, UpdateChannel::Stable, None)],
            Some("1.1.0"),
        );
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, "1.1.0");
        assert_eq!(versions[0].channel, UpdateChannel::Preview);
        assert!(versions.iter().all(|version| version.installed));
    }

    #[test]
    fn version_merges_pin_only_current_and_sort_other_versions_numerically() {
        use crate::update::UpdateChannel;
        let version = |value: &str, installed| CoreVersion {
            version: value.into(),
            channel: UpdateChannel::Stable,
            installed,
            release_tag: Some(format!("v{value}")),
        };
        let mut versions = vec![version("1.0.0", true), version("1.1.0", true)];
        merge_versions(
            &mut versions,
            vec![version("2.0.0-beta.2", false), version("2.0.0", false)],
            Some("1.0.0"),
        );
        assert_eq!(
            versions
                .iter()
                .map(|version| version.version.as_str())
                .collect::<Vec<_>>(),
            ["1.0.0", "2.0.0", "2.0.0-beta.2", "1.1.0"]
        );
        merge_versions(
            &mut versions,
            vec![version("2.0.0-beta.10", false), version("2.0.0", false)],
            Some("1.1.0"),
        );
        assert_eq!(
            versions
                .iter()
                .map(|version| version.version.as_str())
                .collect::<Vec<_>>(),
            ["1.1.0", "2.0.0", "2.0.0-beta.10", "2.0.0-beta.2", "1.0.0"]
        );
        assert!(versions[0].installed);
        assert!(!versions[1].installed);
    }

    #[test]
    fn config_selection_survives_start_failure_when_the_core_is_uninstalled() {
        let folder = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(folder.path().to_owned()).unwrap();
        for core in Core::ALL {
            fs::write(runtime.config_dir(core).join("选中的配置.json"), "{}").unwrap();
            assert!(!runtime.snapshot().core(core).installed);
            runtime
                .send(Action::SwitchConfig(core, "选中的配置.json".into()))
                .unwrap();
            let state = failed(&runtime, core);
            assert_eq!(state.current.as_deref(), Some("选中的配置.json"));
            assert!(!state.running);
            assert!(state.run_error.unwrap().contains("未找到核心程序"));
            assert_eq!(
                Saved::load(folder.path()).unwrap().current(core).as_deref(),
                Some("选中的配置.json")
            );
            runtime.send(Action::Start(core)).unwrap();
            let retry = failed(&runtime, core);
            assert_eq!(retry.current.as_deref(), Some("选中的配置.json"));
            assert!(retry.run_error.unwrap().contains("未找到核心程序"));
        }
        runtime.shutdown().unwrap();
    }

    #[test]
    fn config_selection_save_failure_keeps_the_previous_choice() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        for core in Core::ALL {
            for name in ["当前.json", "其他.json"] {
                fs::write(runtime.config_dir(core).join(name), "{}").unwrap();
            }
            runtime
                .send(Action::SwitchConfig(core, "当前.json".into()))
                .unwrap();
            assert_eq!(failed(&runtime, core).current.as_deref(), Some("当前.json"));
            let saved = fs::read(root.join("ladder-state.json")).unwrap();
            fs::create_dir(root.join("ladder-state.json.tmp")).unwrap();
            runtime
                .send(Action::SwitchConfig(core, "其他.json".into()))
                .unwrap();
            let unchanged = idle(&runtime, core);
            assert_eq!(unchanged.current.as_deref(), Some("当前.json"));
            assert!(!unchanged.running);
            assert!(unchanged.run_error.unwrap().contains("保存当前配置失败"));
            assert_eq!(fs::read(root.join("ladder-state.json")).unwrap(), saved);
            fs::remove_dir(root.join("ladder-state.json.tmp")).unwrap();
        }
        runtime.shutdown().unwrap();
    }

    #[test]
    fn config_deletion_protects_current_files_and_stays_inside_its_directory() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        let outside = root.join("outside.json");
        fs::write(&outside, "outside").unwrap();
        for core in Core::ALL {
            let dir = runtime.config_dir(core);
            fs::write(dir.join("current.json"), "current").unwrap();
            fs::write(dir.join("other.JSON"), "other").unwrap();
            fs::write(dir.join("notes.txt"), "notes").unwrap();
            fs::write(dir.join(".ladder-tun-active.json"), "runtime").unwrap();
            fs::create_dir(dir.join("directory.json")).unwrap();
            fs::hard_link(dir.join("current.json"), dir.join("alias.json")).unwrap();
            runtime
                .send(Action::SwitchConfig(core, "current.json".into()))
                .unwrap();
            assert_eq!(
                failed(&runtime, core).current.as_deref(),
                Some("current.json")
            );
            let saved = fs::read(root.join("ladder-state.json")).unwrap();
            for name in [
                "current.json",
                "CURRENT.JSON",
                "alias.json",
                "../outside.json",
                "..\\outside.json",
                "notes.txt",
                ".ladder-tun-active.json",
                "directory.json",
                "missing.json",
                "current.json:stream.json",
                outside.to_str().unwrap(),
            ] {
                runtime
                    .send(Action::DeleteConfig(core, name.into()))
                    .unwrap();
                let rejected = idle(&runtime, core);
                assert!(rejected.run_error.is_some(), "应拒绝删除 {name}");
                assert_eq!(rejected.current.as_deref(), Some("current.json"));
                assert!(!rejected.running);
            }
            runtime
                .send(Action::DeleteConfig(core, "other.JSON".into()))
                .unwrap();
            let deleted = idle(&runtime, core);
            assert!(deleted.run_error.is_none(), "{deleted:?}");
            assert!(!deleted.configs.iter().any(|name| name == "other.JSON"));
            assert!(!dir.join("other.JSON").exists());
            assert_eq!(
                fs::read_to_string(dir.join("current.json")).unwrap(),
                "current"
            );
            assert_eq!(
                fs::read_to_string(dir.join("alias.json")).unwrap(),
                "current"
            );
            assert_eq!(fs::read_to_string(dir.join("notes.txt")).unwrap(), "notes");
            assert_eq!(fs::read_to_string(&outside).unwrap(), "outside");
            assert_eq!(fs::read(root.join("ladder-state.json")).unwrap(), saved);
        }
        runtime.shutdown().unwrap();
    }

    #[test]
    fn recovering_config_directory_clears_only_the_scan_error() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        for core in Core::ALL {
            fs::create_dir_all(core.config_dir(root)).unwrap();
        }
        let config_dir = Core::SingBox.config_dir(root);
        fs::write(config_dir.join("配置.json"), "{}").unwrap();
        let state = Arc::new(Mutex::new(Snapshot::default()));
        state.lock().unwrap().sing_box.run_error = Some("核心退出错误".into());
        let mut worker = Worker {
            root: root.to_owned(),
            state: state.clone(),
            saved: Saved::default(),
            children: Default::default(),
            dns: DesktopDns::Portable(DnsLease::for_tests(root)),
            quitting: Arc::new(AtomicBool::new(false)),
            version_files: Default::default(),
        };
        worker.refresh();
        assert_eq!(state.lock().unwrap().sing_box.configs, vec!["配置.json"]);
        assert!(!state.lock().unwrap().sing_box.installed);
        let executable = Core::SingBox.executable(root);
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, []).unwrap();
        worker.saved.select_version(Core::SingBox, "1.0.0".into());
        worker.refresh();
        assert!(state.lock().unwrap().sing_box.installed);
        fs::remove_file(executable).unwrap();
        fs::remove_file(config_dir.join("配置.json")).unwrap();
        fs::remove_dir(&config_dir).unwrap();
        worker.refresh();
        assert!(state.lock().unwrap().sing_box.scan_error.is_some());
        assert!(!state.lock().unwrap().sing_box.installed);
        fs::create_dir(&config_dir).unwrap();
        fs::write(config_dir.join("恢复.json"), "{}").unwrap();
        worker.refresh();
        let snapshot = state.lock().unwrap();
        assert!(snapshot.sing_box.scan_error.is_none());
        assert_eq!(snapshot.sing_box.configs, vec!["恢复.json"]);
        assert_eq!(snapshot.sing_box.run_error.as_deref(), Some("核心退出错误"));
    }

    #[test]
    fn downloaded_versions_coexist_without_stopping_cores_and_switch_only_stops_target() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let _guard = lock_and_install_fixture_cores(root);
        fs::write(
            Core::Mihomo.config_dir(root).join("日常.JSON"),
            r#"{"fixture_tree":true,"rule-providers":{"local":{"type":"file","path":"rules/relative.yaml"}}}"#,
        )
        .unwrap();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        for core in Core::ALL {
            runtime
                .send(Action::SwitchConfig(core, "日常.JSON".into()))
                .unwrap();
            assert!(idle(&runtime, core).running);
        }
        for core in Core::ALL {
            let peers = Core::ALL
                .into_iter()
                .filter(|other| *other != core)
                .map(|other| (other, next_pid(&other.config_dir(root).join("日常.pid"), 0)))
                .collect::<Vec<_>>();
            let source = fs::read(core.executable(root)).unwrap();
            let config = fs::read(core.config_dir(root).join("日常.JSON")).unwrap();
            let saved = fs::read(root.join("ladder-state.json")).unwrap();
            let pid_path = core.config_dir(root).join("日常.pid");
            let original_pid = next_pid(&pid_path, 0);
            runtime.state.lock().unwrap().core_mut(core).update =
                Some(UpdateProgress::Downloading {
                    received: 1,
                    total: 10,
                });
            assert!(
                runtime
                    .send(Action::DownloadVersion(core, "2.0.0".into()))
                    .is_err()
            );
            runtime.send(Action::Stop(core)).unwrap();
            assert!(!idle(&runtime, core).running);
            runtime.send(Action::Start(core)).unwrap();
            assert!(idle(&runtime, core).running);
            let restarted_pid = next_pid(&pid_path, original_pid);
            let prepared = PreparedUpdate::for_test(
                core,
                root,
                "2.0.0",
                &[(&format!("{}.exe", core.label()), &source)],
            );
            runtime.queue_prepared(core, prepared).unwrap();
            let state = idle(&runtime, core);
            assert!(
                state.running && state.update.is_none() && state.update_error.is_none(),
                "{state:?}"
            );
            assert_eq!(
                state
                    .versions
                    .iter()
                    .filter(|version| version.installed)
                    .count(),
                2
            );
            assert_eq!(state.selected_version.as_deref(), Some("1.0.0"));
            assert_eq!(fs::read(root.join("ladder-state.json")).unwrap(), saved);
            runtime
                .send(Action::DeleteVersion(core, "1.0.0".into()))
                .unwrap();
            assert!(
                idle(&runtime, core)
                    .update_error
                    .unwrap()
                    .contains("当前使用")
            );
            assert!(core.executable(root).is_file());
            runtime
                .send(Action::SwitchVersion(core, "2.0.0".into()))
                .unwrap();
            let switched = idle(&runtime, core);
            assert!(!switched.running);
            assert_eq!(switched.selected_version.as_deref(), Some("2.0.0"));
            for (other, pid) in peers {
                assert!(runtime.snapshot().core(other).running);
                assert_eq!(next_pid(&other.config_dir(root).join("日常.pid"), 0), pid);
            }
            runtime
                .send(Action::DeleteVersion(core, "1.0.0".into()))
                .unwrap();
            assert!(idle(&runtime, core).update_error.is_none());
            assert!(!core.executable(root).exists());
            assert_eq!(
                fs::read(core.config_dir(root).join("日常.JSON")).unwrap(),
                config
            );
            runtime.send(Action::Start(core)).unwrap();
            assert!(idle(&runtime, core).running);
            next_pid(&pid_path, restarted_pid);
        }
        runtime.shutdown().unwrap();
    }

    #[test]
    fn late_download_after_exit_does_not_install_a_version() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        let core = Core::SingBox;
        let prepared = PreparedUpdate::for_test(core, root, "2.0.0", &[("sing-box.exe", b"late")]);
        runtime.quitting.store(true, Ordering::Release);
        runtime.state.lock().unwrap().sing_box.operation = Some(CoreOperation::Updating);
        runtime.tx.send(Request::Prepared(core, prepared)).unwrap();
        assert!(idle(&runtime, core).update_error.unwrap().contains("取消"));
        assert!(
            !update::version_directory(core, root, "2.0.0")
                .unwrap()
                .exists()
        );
        runtime.shutdown().unwrap();
        let prepared = PreparedUpdate::for_test(core, root, "2.0.0", &[("sing-box.exe", b"late")]);
        assert!(runtime.queue_prepared(core, prepared).is_err());
    }

    #[test]
    fn running_and_version_actions_keep_each_others_errors() {
        let folder = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(folder.path().to_owned()).unwrap();
        let core = Core::Xray;
        runtime.send(Action::Start(core)).unwrap();
        let run_error = idle(&runtime, core).run_error.unwrap();
        assert!(run_error.contains("尚未选择配置"));

        runtime
            .send(Action::SwitchVersion(core, "1.0.0".into()))
            .unwrap();
        let state = idle(&runtime, core);
        assert_eq!(state.run_error.as_ref(), Some(&run_error));
        let version_error = state.update_error.unwrap();
        assert!(version_error.contains("所选版本尚未下载"));

        runtime.send(Action::Stop(core)).unwrap();
        let state = idle(&runtime, core);
        assert!(state.run_error.is_none());
        assert_eq!(state.update_error.as_ref(), Some(&version_error));
        assert!(runtime.snapshot().sing_box.run_error.is_none());
        assert!(runtime.snapshot().sing_box.update_error.is_none());
        runtime.shutdown().unwrap();
    }

    #[test]
    fn current_version_probe_finishes_while_another_version_is_downloading() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let _guard = lock_and_install_fixture_cores(root);
        let core = Core::SingBox;
        let source = root.join("version_probe.rs");
        fs::write(
            &source,
            r#"
            fn main() {
                let exe = std::env::current_exe().unwrap();
                let dir = exe.parent().unwrap();
                std::fs::write(dir.join("probe-started"), []).unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while !dir.join("probe-continue").exists() && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                println!("sing-box version 1.0.0");
            }
        "#,
        )
        .unwrap();
        let compiled = Command::new("rustc")
            .arg(source)
            .arg("-o")
            .arg(core.executable(root))
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let runtime = Runtime::open(root.to_owned()).unwrap();
        let dir = core.executable(root).parent().unwrap().to_owned();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dir.join("probe-started").exists() {
            assert!(Instant::now() < deadline, "版本探针没有启动");
            thread::sleep(Duration::from_millis(10));
        }
        runtime.state.lock().unwrap().sing_box.update = Some(UpdateProgress::Downloading {
            received: 1,
            total: 10,
        });
        fs::write(dir.join("probe-continue"), []).unwrap();
        while runtime.snapshot().sing_box.version.as_deref() != Some("1.0.0") {
            assert!(Instant::now() < deadline, "下载期间丢失了当前版本探针结果");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(runtime.snapshot().sing_box.update.is_some());
        runtime.shutdown().unwrap();
    }

    #[test]
    fn manually_installed_and_replaced_executables_refresh_the_displayed_version() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let _guard = lock_and_install_fixture_cores(root);
        let runtime = Runtime::open(root.to_owned()).unwrap();
        let core = Core::SingBox;
        let deadline = Instant::now() + Duration::from_secs(10);
        while runtime.snapshot().sing_box.version.as_deref() != Some("1.0.0") {
            assert!(Instant::now() < deadline, "未检测到手动安装的版本");
            thread::sleep(Duration::from_millis(30));
        }
        fs::write(core.executable(root), b"invalid replacement").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = runtime.snapshot().sing_box;
            if state.version.is_none() && state.version_error.is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "仍显示旧版本: {state:?}");
            thread::sleep(Duration::from_millis(30));
        }
        runtime.shutdown().unwrap();
    }
}
