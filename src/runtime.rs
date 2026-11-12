use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::dns::DnsLease;
use crate::managed_child::ManagedChild;
use crate::scheduler::{DesktopDns, desktop_dns};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Core {
    SingBox,
    Xray,
}

impl Core {
    pub const ALL: [Core; 2] = [Core::SingBox, Core::Xray];

    pub fn label(self) -> &'static str {
        match self {
            Self::SingBox => "sing-box",
            Self::Xray => "xray",
        }
    }

    fn config_dir(self, root: &Path) -> PathBuf {
        root.join("configs").join(self.label())
    }

    fn core_dir(self, root: &Path) -> PathBuf {
        root.join(format!("{}_core", self.label()))
    }

    fn executable(self, root: &Path) -> PathBuf {
        self.core_dir(root).join(format!("{}.exe", self.label()))
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
#[serde(default)]
pub struct Settings {
    pub theme: ThemePreference,
    pub minimize_to_tray: bool,
    pub auto_sing_box: bool,
    pub auto_xray: bool,
    pub hijack_dns: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreOperation {
    Starting,
    Stopping,
    Switching,
    Restarting,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoreState {
    pub configs: Vec<String>,
    pub current: Option<String>,
    pub installed: bool,
    pub running: bool,
    pub operation: Option<CoreOperation>,
    pub error: Option<String>,
    pub scan_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub settings: Settings,
    pub sing_box: CoreState,
    pub xray: CoreState,
    pub error: Option<String>,
    pub dns_recovery_error: Option<String>,
    pub dns_recovery_busy: bool,
    pub dns_task_warning: Option<String>,
}

impl Snapshot {
    pub fn core(&self, core: Core) -> &CoreState {
        match core {
            Core::SingBox => &self.sing_box,
            Core::Xray => &self.xray,
        }
    }

    fn core_mut(&mut self, core: Core) -> &mut CoreState {
        match core {
            Core::SingBox => &mut self.sing_box,
            Core::Xray => &mut self.xray,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    MinimizeToTray,
    AutoSingBox,
    AutoXray,
    HijackDns,
}

#[derive(Clone)]
pub enum Action {
    Start(Core),
    Stop(Core),
    Restart(Core),
    Use(Core, String),
    Set(Setting, bool),
    RetryDns,
}

enum Request {
    Action(Action),
    SetTheme(ThemePreference, async_channel::Sender<()>),
    Shutdown(mpsc::Sender<Result<Option<String>, String>>),
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct Saved {
    settings: Settings,
    sing_box: Option<String>,
    xray: Option<String>,
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
        }
    }
}

#[derive(Clone)]
pub struct Runtime {
    root: PathBuf,
    state: Arc<Mutex<Snapshot>>,
    tx: mpsc::Sender<Request>,
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
        let recovery_error = dns.lease().and_then(DnsLease::restore_if_pending).err();
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
            let state = snapshot.core_mut(core);
            state.configs = list_configs(&dir)?;
            state.current = saved.current(core).clone();
            state.installed = core.executable(&root).is_file();
            let auto = match core {
                Core::SingBox => saved.settings.auto_sing_box,
                Core::Xray => saved.settings.auto_xray,
            };
            state.operation = auto.then_some(CoreOperation::Starting);
        }
        let state = Arc::new(Mutex::new(snapshot));
        let (tx, rx) = mpsc::channel();
        let worker = Worker {
            root: root.clone(),
            state: state.clone(),
            saved,
            children: [None, None],
            dns,
        };
        thread::Builder::new()
            .name("ladder-cores".into())
            .spawn(move || worker.run(rx))
            .map_err(|e| format!("创建核心管理线程失败: {e}"))?;
        Ok(Self { root, state, tx })
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
        let mut snapshot = self.state.lock().unwrap();
        let retry_dns = matches!(action, Action::RetryDns);
        let target = match &action {
            Action::Start(core) => Some((*core, CoreOperation::Starting)),
            Action::Stop(core) => Some((*core, CoreOperation::Stopping)),
            Action::Restart(core) => Some((*core, CoreOperation::Restarting)),
            Action::Use(core, _) => Some((*core, CoreOperation::Switching)),
            Action::Set(_, _) => None,
            Action::RetryDns => {
                if snapshot.dns_recovery_error.is_none() || snapshot.dns_recovery_busy {
                    return Err("当前没有可重试的 DNS 恢复操作".into());
                }
                snapshot.dns_recovery_busy = true;
                None
            }
        };
        if let Some((core, operation)) = target {
            let active = snapshot.core_mut(core);
            if active.operation.is_some() {
                return Err(format!("{} 正在处理上一项操作", core.label()));
            }
            active.operation = Some(operation);
            active.error = None;
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

    /// 核心均已停止时返回可选的 DNS 恢复提示，失败的记录保留到下次启动。
    pub fn shutdown(&self) -> Result<Option<String>, String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Request::Shutdown(tx))
            .map_err(|_| "核心管理线程已关闭".to_string())?;
        rx.recv()
            .map_err(|_| "核心管理线程未返回退出结果".to_string())?
    }
}

fn list_configs(dir: &Path) -> Result<Vec<String>, String> {
    let mut items = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("读取配置目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取配置文件信息失败: {e}"))?;
        let path = entry.path();
        if is_json(&path)
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

struct Worker {
    root: PathBuf,
    state: Arc<Mutex<Snapshot>>,
    saved: Saved,
    children: [Option<ManagedChild>; 2],
    dns: DesktopDns,
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<Request>) {
        for core in Core::ALL {
            let auto = match core {
                Core::SingBox => self.saved.settings.auto_sing_box,
                Core::Xray => self.saved.settings.auto_xray,
            };
            if auto {
                self.action(Action::Start(core));
            }
        }
        let mut last_scan = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(Request::Action(action)) => self.action(action),
                Ok(Request::SetTheme(theme, reply)) => {
                    self.update_settings(|settings| settings.theme = theme);
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
                Err(mpsc::RecvTimeoutError::Disconnected) => {
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
    }

    fn action(&mut self, action: Action) {
        let (core, result) = match action {
            Action::RetryDns => {
                self.retry_dns(desktop_dns);
                return;
            }
            Action::Set(key, value) => {
                return self.update_settings(|settings| match key {
                    Setting::MinimizeToTray => settings.minimize_to_tray = value,
                    Setting::AutoSingBox => settings.auto_sing_box = value,
                    Setting::AutoXray => settings.auto_xray = value,
                    Setting::HijackDns => settings.hijack_dns = value,
                });
            }
            Action::Stop(core) => (core, self.stop(core)),
            Action::Start(core) => {
                let current = self.saved.current(core).clone();
                let result = match current {
                    None => Err("尚未选择配置, 请在配置列表中点击“使用此配置”".into()),
                    Some(name) if self.children[core as usize].is_none() => self
                        .readable_config(core, &name)
                        .and_then(|config| self.start(core, &config)),
                    Some(_) => Ok(()),
                };
                (core, result)
            }
            Action::Restart(core) => {
                let current = self.saved.current(core).clone();
                let result = match current {
                    None => Err("尚未选择配置, 请在配置列表中点击“使用此配置”".into()),
                    Some(name) => self.readable_config(core, &name).and_then(|config| {
                        self.stop(core)?;
                        self.start(core, &config)
                    }),
                };
                (core, result)
            }
            Action::Use(core, name) => (core, self.use_config(core, &name)),
        };
        let mut snapshot = self.state.lock().unwrap();
        let status = snapshot.core_mut(core);
        status.operation = None;
        status.running = self.children[core as usize].is_some();
        status.current = self.saved.current(core).clone();
        status.error = result.err();
    }

    fn update_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        let mut next = self.saved.clone();
        change(&mut next.settings);
        let saved = next.store(&self.root);
        let mut snapshot = self.state.lock().unwrap();
        match saved {
            Ok(()) => {
                snapshot.settings = next.settings.clone();
                self.saved = next;
                snapshot.error = None;
            }
            Err(e) => snapshot.error = Some(format!("保存软件设置失败: {e}")),
        }
    }

    fn use_config(&mut self, core: Core, name: &str) -> Result<(), String> {
        let config = self.readable_config(core, name)?;
        let mut next = self.saved.clone();
        match core {
            Core::SingBox => next.sing_box = Some(name.into()),
            Core::Xray => next.xray = Some(name.into()),
        }
        next.store(&self.root)
            .map_err(|e| format!("保存当前配置失败: {e}"))?;
        self.saved = next;
        self.state.lock().unwrap().core_mut(core).current = Some(name.into());
        self.stop(core)?;
        self.start(core, &config)
    }

    fn refresh(&self) {
        for core in Core::ALL {
            let result = list_configs(&core.config_dir(&self.root));
            let installed = core.executable(&self.root).is_file();
            let mut snapshot = self.state.lock().unwrap();
            let state = snapshot.core_mut(core);
            state.installed = installed;
            match result {
                Ok(files) => {
                    state.configs = files;
                    state.scan_error = None;
                }
                Err(e) => state.scan_error = Some(e),
            }
        }
    }

    fn readable_config(&self, core: Core, name: &str) -> Result<PathBuf, String> {
        let name_path = Path::new(name);
        if !matches!(name_path.components().next(), Some(Component::Normal(_)))
            || name_path.file_name() != Some(OsStr::new(name))
            || !is_json(name_path)
        {
            return Err("配置文件名无效".into());
        }
        let config = core.config_dir(&self.root).join(name);
        let metadata = fs::File::open(&config)
            .and_then(|file| file.metadata())
            .map_err(|error| format!("无法读取配置文件，请检查文件是否存在及访问权限: {error}"))?;
        if !metadata.is_file() {
            return Err("所选路径不是配置文件".into());
        }
        Ok(config)
    }

    fn start(&mut self, core: Core, config: &Path) -> Result<(), String> {
        let assets = core.core_dir(&self.root);
        let exe = core.executable(&self.root);
        if !exe.is_file() {
            return Err(format!("未找到核心程序: {}", exe.display()));
        }
        let hijack = self.saved.settings.hijack_dns;
        if hijack && self.state.lock().unwrap().dns_recovery_error.is_some() {
            self.restore_dns()
                .map_err(|_| "DNS 尚未恢复，无法开始新的 DNS 劫持")?;
        }
        let child = ManagedChild::spawn(core, &self.root, &exe, &assets, config)
            .map_err(|e| format!("启动 {} 失败: {e}", core.label()))?;
        self.children[core as usize] = Some(child);
        if hijack && let Err(error) = self.dns.lease().and_then(DnsLease::apply) {
            let stopped = self.stop_process(core);
            let _ = self.restore_dns();
            return match stopped {
                Ok(_) => Err(error),
                Err(cleanup) => Err(format!("{error}；{cleanup}")),
            };
        }
        Ok(())
    }

    fn stop(&mut self, core: Core) -> Result<(), String> {
        if self.children[core as usize].is_some() {
            self.stop_process(core)?;
            let _ = self.restore_dns();
        }
        Ok(())
    }

    fn restore_dns(&self) -> Result<(), String> {
        let result = self.dns.lease().and_then(DnsLease::restore);
        self.state.lock().unwrap().dns_recovery_error = result.as_ref().err().cloned();
        result
    }

    fn retry_dns(&mut self, reopen: impl FnOnce(&Path) -> (DesktopDns, Option<String>)) {
        // 排队期间另一个操作可能已恢复 DNS，甚至已开始新一轮劫持。
        if self.state.lock().unwrap().dns_recovery_error.is_some() {
            if matches!(self.dns, DesktopDns::Unavailable(_)) {
                let (dns, warning) = reopen(&self.root);
                self.dns = dns;
                self.state.lock().unwrap().dns_task_warning = warning;
            }
            let _ = self.restore_dns();
        }
        self.state.lock().unwrap().dns_recovery_busy = false;
    }

    fn stop_process(&mut self, core: Core) -> Result<Option<&'static str>, String> {
        let owned = &mut self.children[core as usize];
        if let Some(child) = owned {
            child
                .terminate()
                .map_err(|e| format!("停止 {} 失败: {e}", core.label()))?;
        }
        let reason = owned.take().and_then(|child| child.output.finish());
        self.state.lock().unwrap().core_mut(core).running = false;
        Ok(reason)
    }

    fn stop_all(&mut self) -> Result<Option<String>, String> {
        let running = self.children.iter().flatten().count();
        let failures: Vec<_> = Core::ALL
            .into_iter()
            .filter_map(|core| self.stop_process(core).err())
            .collect();
        let previous_error = self.state.lock().unwrap().dns_recovery_error.clone();
        let dns_error = if running == 0 && previous_error.is_none() {
            self.dns
                .lease()
                .and_then(DnsLease::restore_if_pending)
                .err()
        } else if failures.is_empty() || failures.len() < running {
            self.restore_dns().err()
        } else {
            previous_error
        };
        if failures.is_empty() {
            Ok(dns_error)
        } else {
            Err(failures
                .into_iter()
                .chain(dns_error)
                .collect::<Vec<_>>()
                .join("；"))
        }
    }

    fn poll(&mut self, core: Core) {
        let Some(child) = &mut self.children[core as usize] else {
            return;
        };
        let status = match child.try_wait() {
            Ok(None) => return,
            Err(e) => {
                self.state.lock().unwrap().core_mut(core).error =
                    Some(format!("检查核心状态失败: {e}"));
                return;
            }
            Ok(Some(code)) => code,
        };
        let result = self.stop_process(core);
        if result.is_ok() {
            let _ = self.restore_dns();
        }
        self.state.lock().unwrap().core_mut(core).error = Some(match result {
            Ok(reason) => format!(
                "核心已退出 ({status}) 。{}",
                reason.unwrap_or("请检查配置内容及核心所需的资源文件")
            ),
            Err(e) => format!("核心已退出 ({status}) 。{e}"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::process::{Child, Command};
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
            if state.operation.is_none() && !state.running && state.error.is_some() {
                return state;
            }
            assert!(Instant::now() < deadline, "等待核心退出超时: {state:?}");
            thread::sleep(Duration::from_millis(30));
        }
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
            let dir = root.join(format!("{}_core", core.label()));
            fs::create_dir(&dir).unwrap();
            fs::copy(&exe, dir.join(format!("{}.exe", core.label()))).unwrap();
            fs::create_dir_all(core.config_dir(root)).unwrap();
            let config = match core {
                Core::SingBox => "{ /* comment */ \"log\": {}, }",
                Core::Xray => "# comment\n{ \"log\": {} }",
            };
            fs::write(core.config_dir(root).join("日常.JSON"), config).unwrap();
            fs::write(
                core.config_dir(root).join("失败.json"),
                r#"{"fixture_failure": true}"#,
            )
            .unwrap();
        }
        guard
    }

    // 真实子进程验证 Windows 句柄归属、带空格的路径和启动失败, 不操作网络接口。
    #[test]
    fn selection_survives_start_failure_retry_and_restart_without_affecting_other_processes() {
        let folder = tempfile::Builder::new()
            .prefix("梯子 runtime ")
            .tempdir()
            .unwrap();
        let root = folder.path();
        let _process_test = lock_and_install_fixture_cores(root);
        let mut external = FixtureChild(
            Command::new(root.join("sing-box_core/sing-box.exe"))
                .arg(Core::SingBox.config_dir(root).join("日常.JSON"))
                .spawn()
                .unwrap(),
        );
        assert_eq!(
            next_pid(&Core::SingBox.config_dir(root).join("日常.pid"), 0),
            external.0.id()
        );
        let runtime = Runtime::open(root.to_owned()).unwrap();
        let mut pids = [external.0.id(), 0];
        for core in Core::ALL {
            assert_eq!(runtime.snapshot().core(core).configs.len(), 2);
            assert!(runtime.snapshot().core(core).installed);
            runtime.send(Action::Use(core, "日常.JSON".into())).unwrap();
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
            runtime.send(Action::Restart(core)).unwrap();
            let restarted = idle(&runtime, core);
            assert!(restarted.running, "{restarted:?}");
            assert!(restarted.error.is_none(), "{restarted:?}");
            assert_eq!(restarted.current.as_deref(), Some("日常.JSON"));
            pids[core as usize] =
                next_pid(&core.config_dir(root).join("日常.pid"), pids[core as usize]);
            let other = match core {
                Core::SingBox => Core::Xray,
                Core::Xray => Core::SingBox,
            };
            assert!(runtime.snapshot().core(other).running);
            assert_eq!(
                fs::read_to_string(other.config_dir(root).join("日常.pid")).unwrap(),
                pids[other as usize].to_string()
            );
            assert!(external.0.try_wait().unwrap().is_none());
        }
        assert_eq!(
            fs::read(root.join("ladder-state.json")).unwrap(),
            saved_before_restart
        );
        runtime
            .send(Action::Use(Core::SingBox, "失败.json".into()))
            .unwrap();
        let unchanged = idle(&runtime, Core::SingBox);
        assert!(unchanged.running);
        assert_eq!(unchanged.current.as_deref(), Some("日常.JSON"));
        assert!(unchanged.error.unwrap().contains("保存当前配置失败"));
        assert_eq!(
            Saved::load(root).unwrap().sing_box.as_deref(),
            Some("日常.JSON")
        );
        fs::remove_dir(blocked_save).unwrap();

        runtime
            .send(Action::Use(Core::SingBox, "失败.json".into()))
            .unwrap();
        let stopped = failed(&runtime, Core::SingBox);
        assert!(!stopped.running);
        assert_eq!(stopped.current.as_deref(), Some("失败.json"));
        assert_eq!(
            Saved::load(root).unwrap().sing_box.as_deref(),
            Some("失败.json")
        );
        let error = stopped.error.unwrap();
        assert!(error.contains("42"));
        assert!(error.contains("核心没有访问所需资源的权限"));
        assert!(!error.contains("PRIVATE_FIXTURE_VALUE"));
        assert!(runtime.snapshot().xray.running);
        runtime.send(Action::Start(Core::SingBox)).unwrap();
        let retry = failed(&runtime, Core::SingBox);
        assert_eq!(retry.current.as_deref(), Some("失败.json"));
        assert!(retry.error.unwrap().contains("42"));
        runtime
            .send(Action::Set(Setting::AutoSingBox, true))
            .unwrap();
        runtime
            .send(Action::Set(Setting::MinimizeToTray, true))
            .unwrap();
        let xray_dir = Core::Xray.config_dir(root);
        let moved_dir = root.join("移走的配置目录");
        fs::rename(&xray_dir, &moved_dir).unwrap();
        runtime.send(Action::Restart(Core::Xray)).unwrap();
        let restart_without_file = idle(&runtime, Core::Xray);
        runtime
            .send(Action::Use(Core::Xray, "失败.json".into()))
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
                    .error
                    .as_deref()
                    .is_some_and(|e| e.contains("23"))
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
        assert!(restarted.error.unwrap().contains("42"));
        assert!(!reopened.snapshot().xray.running);
        assert!(reopened.snapshot().settings.minimize_to_tray);
        reopened.shutdown().unwrap();
        for state in [restart_without_file, switch_without_file] {
            assert!(state.running, "{state:?}");
            assert!(state.error.is_some());
            assert_eq!(state.current.as_deref(), Some("日常.JSON"));
        }
        assert_eq!(
            Saved::load(root).unwrap().xray.as_deref(),
            Some("日常.JSON")
        );
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
                ..Default::default()
            },
            sing_box: Some(config.into()),
            xray: Some(config.into()),
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
        let signal = Core::SingBox
            .config_dir(root)
            .join(config)
            .with_extension("exit");
        fs::write(&signal, []).unwrap();
        assert!(failed(&runtime, Core::SingBox).error.is_some());
        exited(&sing);
        for handle in &xray {
            assert_eq!(
                unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) },
                WAIT_TIMEOUT
            );
        }
        runtime.shutdown().unwrap();
        exited(&xray);
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
        host.0.kill().unwrap();
        host.0.wait().unwrap();
        exited(&sing);
        exited(&xray);
        assert!(external.0.try_wait().unwrap().is_none());
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
            .send(Action::Use(Core::Xray, "日常.JSON".into()))
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
            .send(Action::Use(Core::Xray, "日常.JSON".into()))
            .unwrap();
        assert!(idle(&runtime, Core::Xray).running);
        runtime.send(Action::Set(Setting::HijackDns, true)).unwrap();
        runtime
            .send(Action::Use(Core::SingBox, "日常.JSON".into()))
            .unwrap();
        let refused = idle(&runtime, Core::SingBox);
        assert!(!refused.running && refused.error.is_some());
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
            children: [None, None],
            dns: DesktopDns::Portable(DnsLease::for_tests(root)),
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
            children: [None, None],
            dns: DesktopDns::Unavailable("目录不可用".into()),
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
    fn either_core_can_start_hijacking_and_each_actual_exit_restores_dns() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let _process_test = lock_and_install_fixture_cores(root);
        let runtime = Runtime::open(root.to_owned()).unwrap();
        let journal = root.join("dns-recovery.json");
        runtime.send(Action::Set(Setting::HijackDns, true)).unwrap();
        for core in Core::ALL {
            runtime.send(Action::Use(core, "日常.JSON".into())).unwrap();
            let state = idle(&runtime, core);
            assert!(state.running && state.error.is_none(), "{state:?}");
            assert!(journal.is_file(), "{} 启动后缺少 DNS 原值", core.label());
        }
        // 关闭开关只影响下一次启动。当前会话仍应在任意核心退出时恢复。
        runtime
            .send(Action::Set(Setting::HijackDns, false))
            .unwrap();
        runtime.send(Action::Stop(Core::Xray)).unwrap();
        assert!(!idle(&runtime, Core::Xray).running);
        assert!(runtime.snapshot().sing_box.running);
        assert!(!journal.exists());
        runtime.send(Action::Start(Core::Xray)).unwrap();
        assert!(idle(&runtime, Core::Xray).running);
        assert!(!journal.exists());

        runtime.send(Action::Set(Setting::HijackDns, true)).unwrap();
        runtime.send(Action::Restart(Core::Xray)).unwrap();
        assert!(idle(&runtime, Core::Xray).running);
        assert!(journal.exists());
        fs::write(Core::SingBox.config_dir(root).join("日常.exit"), []).unwrap();
        assert!(
            failed(&runtime, Core::SingBox)
                .error
                .unwrap()
                .contains("23")
        );
        assert!(runtime.snapshot().xray.running);
        assert!(!journal.exists());
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
        };
        let mut worker = Worker {
            root: runtime.root.clone(),
            state: runtime.state.clone(),
            saved: Saved::default(),
            children: [None, None],
            dns: DesktopDns::Portable(DnsLease::for_tests(folder.path())),
        };
        let core = Core::SingBox;
        for (action, operation) in [
            (Action::Start(core), CoreOperation::Starting),
            (Action::Stop(core), CoreOperation::Stopping),
            (
                Action::Use(core, "选中.json".into()),
                CoreOperation::Switching,
            ),
            (Action::Restart(core), CoreOperation::Restarting),
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
    fn old_state_defaults_and_selections_survive_settings_changes() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let defaults = Saved::load(root).unwrap().settings;
        assert_eq!(defaults.theme, ThemePreference::System);
        assert!(!defaults.hijack_dns);
        let path = root.join("ladder-state.json");
        let legacy = r#"{"settings":{"minimize_to_tray":true},"sing_box":"日常.json"}"#;
        fs::write(&path, legacy).unwrap();
        let mut runtime = Runtime::open(root.to_owned()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
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
        assert!(snapshot.error.unwrap().contains("保存软件设置失败"));
        assert_eq!(fs::read(path).unwrap(), previous);
        runtime.shutdown().unwrap();
    }

    #[test]
    fn selection_is_saved_even_when_the_core_is_missing() {
        let folder = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(folder.path().to_owned()).unwrap();
        fs::write(
            runtime.config_dir(Core::SingBox).join("选中的配置.json"),
            "{}",
        )
        .unwrap();
        assert!(!runtime.snapshot().sing_box.installed);
        runtime
            .send(Action::Use(Core::SingBox, "选中的配置.json".into()))
            .unwrap();
        let state = failed(&runtime, Core::SingBox);
        assert_eq!(state.current.as_deref(), Some("选中的配置.json"));
        assert!(state.error.unwrap().contains("未找到核心程序"));
        assert_eq!(
            Saved::load(folder.path()).unwrap().sing_box.as_deref(),
            Some("选中的配置.json")
        );
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
        state.lock().unwrap().sing_box.error = Some("核心退出错误".into());
        let worker = Worker {
            root: root.to_owned(),
            state: state.clone(),
            saved: Saved::default(),
            children: [None, None],
            dns: DesktopDns::Portable(DnsLease::for_tests(root)),
        };
        worker.refresh();
        assert_eq!(state.lock().unwrap().sing_box.configs, vec!["配置.json"]);
        assert!(!state.lock().unwrap().sing_box.installed);
        let executable = Core::SingBox.executable(root);
        fs::create_dir(Core::SingBox.core_dir(root)).unwrap();
        fs::write(&executable, []).unwrap();
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
        assert_eq!(snapshot.sing_box.error.as_deref(), Some("核心退出错误"));
    }
}
