use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::config::xray_uses_tun;
use crate::core_output::CoreOutput;
use crate::dns::DnsLease;

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
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub minimize_to_tray: bool,
    pub auto_sing_box: bool,
    pub auto_xray: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoreState {
    pub configs: Vec<String>,
    pub current: Option<String>,
    pub running: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub scan_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub settings: Settings,
    pub sing_box: CoreState,
    pub xray: CoreState,
    pub error: Option<String>,
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
}

pub enum Action {
    Start(Core),
    Stop(Core),
    Use(Core, String),
    Set(Setting, bool),
}

enum Request {
    Action(Action),
    Shutdown(mpsc::Sender<Result<(), String>>),
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
    pub fn open(root: PathBuf) -> Result<Self, String> {
        let root = std::path::absolute(root).map_err(|e| format!("读取软件目录失败: {e}"))?;
        let mut dns = DnsLease::new(&root);
        dns.restore().map_err(|e| {
            format!("无法恢复上次运行的 DNS 设置, 请检查网卡或以管理员身份重新打开 Ladder。{e}")
        })?;
        let saved = Saved::load(&root)?;
        let mut snapshot = Snapshot {
            settings: saved.settings.clone(),
            ..Default::default()
        };
        for core in Core::ALL {
            let dir = core.config_dir(&root);
            fs::create_dir_all(&dir)
                .map_err(|e| format!("创建 {} 配置目录失败: {e}", core.label()))?;
            let state = snapshot.core_mut(core);
            state.configs = list_configs(&dir)?;
            state.current = saved.current(core).clone();
            state.busy = match core {
                Core::SingBox => saved.settings.auto_sing_box,
                Core::Xray => saved.settings.auto_xray,
            };
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

    pub fn send(&self, action: Action) -> Result<(), String> {
        let mut snapshot = self.state.lock().unwrap();
        let target = match &action {
            Action::Start(core) | Action::Stop(core) | Action::Use(core, _) => Some(*core),
            _ => None,
        };
        if let Some(core) = target {
            let active = snapshot.core_mut(core);
            if active.busy {
                return Err(format!("{} 正在处理上一项操作", core.label()));
            }
            active.busy = true;
            active.error = None;
        }
        self.tx.send(Request::Action(action)).map_err(|_| {
            if let Some(core) = target {
                snapshot.core_mut(core).busy = false;
            }
            "核心管理线程已关闭".to_string()
        })
    }

    pub fn shutdown(&self) -> Result<(), String> {
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

struct ManagedChild {
    process: Child,
    output: CoreOutput,
}

struct Worker {
    root: PathBuf,
    state: Arc<Mutex<Snapshot>>,
    saved: Saved,
    children: [Option<ManagedChild>; 2],
    dns: DnsLease,
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
            Action::Set(key, value) => {
                let mut next = self.saved.clone();
                match key {
                    Setting::MinimizeToTray => next.settings.minimize_to_tray = value,
                    Setting::AutoSingBox => next.settings.auto_sing_box = value,
                    Setting::AutoXray => next.settings.auto_xray = value,
                }
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
                return;
            }
            Action::Stop(core) => (core, self.stop(core)),
            Action::Start(core) => {
                let current = self.saved.current(core).clone();
                let result = match current {
                    None => Err("尚未选择配置, 请在配置列表中点击“使用此配置”".into()),
                    Some(name) if self.children[core as usize].is_none() => self.start(core, &name),
                    Some(_) => Ok(()),
                };
                (core, result)
            }
            Action::Use(core, name) => (core, self.use_config(core, &name)),
        };
        let mut snapshot = self.state.lock().unwrap();
        let status = snapshot.core_mut(core);
        status.busy = false;
        status.running = self.children[core as usize].is_some();
        status.current = self.saved.current(core).clone();
        status.error = result.err();
    }

    fn use_config(&mut self, core: Core, name: &str) -> Result<(), String> {
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
        self.start(core, name)
    }

    fn refresh(&self) {
        for core in Core::ALL {
            let result = list_configs(&core.config_dir(&self.root));
            let mut snapshot = self.state.lock().unwrap();
            let state = snapshot.core_mut(core);
            match result {
                Ok(files) => {
                    state.configs = files;
                    state.scan_error = None;
                }
                Err(e) => state.scan_error = Some(e),
            }
        }
    }

    fn start(&mut self, core: Core, name: &str) -> Result<(), String> {
        let name_path = Path::new(name);
        if !matches!(name_path.components().next(), Some(Component::Normal(_)))
            || name_path.file_name() != Some(OsStr::new(name))
            || !is_json(name_path)
        {
            return Err("配置文件名无效".into());
        }
        let config = core.config_dir(&self.root).join(name);
        let assets = self.root.join(format!("{}_core", core.label()));
        let exe = assets.join(format!("{}.exe", core.label()));
        if !exe.is_file() {
            return Err(format!("未找到核心程序: {}", exe.display()));
        }
        let mut command = Command::new(exe);
        command.current_dir(&self.root).arg("run");
        if core == Core::SingBox {
            command.arg("-D").arg(&assets);
        } else {
            command.env("XRAY_LOCATION_ASSET", &assets);
        }
        command
            .arg("-c")
            .arg(&config)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let tun = core == Core::Xray && xray_uses_tun(&config)?;
        let run = (|| {
            if core == Core::Xray {
                self.dns.restore()?;
            }
            if tun {
                self.dns.apply()?;
            }
            let mut process = command
                .spawn()
                .map_err(|e| format!("启动 {} 失败: {e}", core.label()))?;
            let output = CoreOutput::capture(
                process.stdout.take().unwrap(),
                process.stderr.take().unwrap(),
            );
            self.children[core as usize] = Some(ManagedChild { process, output });
            Ok(())
        })();
        if let Err(error) = run {
            return match self.stop(core) {
                Ok(()) => Err(error),
                Err(cleanup) => Err(format!("{error}；{cleanup}")),
            };
        }
        Ok(())
    }

    fn stop(&mut self, core: Core) -> Result<(), String> {
        let owned = &mut self.children[core as usize];
        if let Some(child) = owned {
            let result = (|| -> io::Result<()> {
                if child.process.try_wait()?.is_none() {
                    // 核心可能刚好在 try_wait 与 kill 之间退出。
                    if let Err(e) = child.process.kill()
                        && child.process.try_wait()?.is_none()
                    {
                        return Err(e);
                    }
                    child.process.wait()?;
                }
                Ok(())
            })();
            result.map_err(|e| format!("停止 {} 失败: {e}", core.label()))?;
            owned.take().unwrap().output.finish();
        }
        self.state.lock().unwrap().core_mut(core).running = false;
        if core == Core::Xray {
            self.dns.restore()?;
        }
        Ok(())
    }

    fn stop_all(&mut self) -> Result<(), String> {
        let failures: Vec<_> = Core::ALL
            .into_iter()
            .filter_map(|core| self.stop(core).err())
            .collect();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("；"))
        }
    }

    fn poll(&mut self, core: Core) {
        let Some(child) = &mut self.children[core as usize] else {
            return;
        };
        let status = match child.process.try_wait() {
            Ok(None) => return,
            Err(e) => {
                self.state.lock().unwrap().core_mut(core).error =
                    Some(format!("检查核心状态失败: {e}"));
                return;
            }
            Ok(Some(code)) => code,
        };
        let child = self.children[core as usize].take().unwrap();
        let reason = child
            .output
            .finish()
            .unwrap_or("请检查配置内容及核心所需的资源文件");
        let error = format!("核心已退出 ({status}) 。{reason}");
        let cleanup = self.stop(core);
        self.state.lock().unwrap().core_mut(core).error = Some(match cleanup {
            Ok(()) => error,
            Err(e) => format!("{error}；{e}"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle(runtime: &Runtime, core: Core) -> CoreState {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let state = runtime.snapshot().core(core).clone();
            if !state.busy {
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
            if !state.busy && !state.running && state.error.is_some() {
                return state;
            }
            assert!(Instant::now() < deadline, "等待核心退出超时: {state:?}");
            thread::sleep(Duration::from_millis(30));
        }
    }

    // 真实子进程验证 Windows 句柄归属、带空格的路径和启动失败, 不操作网络接口。
    #[test]
    fn selection_survives_start_failure_retry_and_restart_without_affecting_other_processes() {
        let folder = tempfile::Builder::new()
            .prefix("梯子 runtime ")
            .tempdir()
            .unwrap();
        let root = folder.path();
        let source = root.join("fixture.rs");
        fs::write(
            &source,
            r#"
            fn main() {
                let file = std::env::args_os().last().unwrap();
                let config = std::fs::read_to_string(file).unwrap();
                if config.contains("fixture_failure") {
                    eprintln!("access is denied: PRIVATE_FIXTURE_VALUE");
                    std::process::exit(42);
                }
                std::thread::sleep(std::time::Duration::from_secs(90));
            }
        "#,
        )
        .unwrap();
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
        let mut external = Command::new(root.join("sing-box_core/sing-box.exe"))
            .arg(Core::SingBox.config_dir(root).join("日常.JSON"))
            .spawn()
            .unwrap();
        let runtime = Runtime::open(root.to_owned()).unwrap();
        for core in Core::ALL {
            assert_eq!(runtime.snapshot().core(core).configs.len(), 2);
            runtime.send(Action::Use(core, "日常.JSON".into())).unwrap();
            let state = idle(&runtime, core);
            assert!(state.running, "{state:?}");
            assert_eq!(state.current.as_deref(), Some("日常.JSON"));
        }
        let blocked_save = root.join("ladder-state.json.tmp");
        fs::create_dir(&blocked_save).unwrap();
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
        runtime.shutdown().unwrap();
        assert!(external.try_wait().unwrap().is_none());

        let reopened = Runtime::open(root.to_owned()).unwrap();
        let restarted = failed(&reopened, Core::SingBox);
        assert_eq!(restarted.current.as_deref(), Some("失败.json"));
        assert!(restarted.error.unwrap().contains("42"));
        assert!(!reopened.snapshot().xray.running);
        assert!(reopened.snapshot().settings.minimize_to_tray);
        reopened.shutdown().unwrap();
        external.kill().unwrap();
        external.wait().unwrap();
    }

    #[test]
    fn selection_is_saved_even_when_the_core_is_missing() {
        let folder = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(folder.path().to_owned()).unwrap();
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
            dns: DnsLease::new(root),
        };
        worker.refresh();
        assert_eq!(state.lock().unwrap().sing_box.configs, vec!["配置.json"]);
        fs::remove_file(config_dir.join("配置.json")).unwrap();
        fs::remove_dir(&config_dir).unwrap();
        worker.refresh();
        assert!(state.lock().unwrap().sing_box.scan_error.is_some());
        fs::create_dir(&config_dir).unwrap();
        fs::write(config_dir.join("恢复.json"), "{}").unwrap();
        worker.refresh();
        let snapshot = state.lock().unwrap();
        assert!(snapshot.sing_box.scan_error.is_none());
        assert_eq!(snapshot.sing_box.configs, vec!["恢复.json"]);
        assert_eq!(snapshot.sing_box.error.as_deref(), Some("核心退出错误"));
    }
}
