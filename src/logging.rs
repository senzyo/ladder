//! 应用诊断只落盘安全摘要；核心日志清理终端控制序列后仅保存在各自的内存队列。

use std::collections::VecDeque;
use std::fmt::{self, Display, Write as _};
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

use crate::runtime::Core;

const FILE_LIMIT: u64 = 2 * 1024 * 1024;
const RECENT_LIMIT: usize = 500;
const RECORD_LIMIT: usize = 8 * 1024;
const LIMIT_WARNING: &str = "本次运行日志已达 2 MiB 上限，后续日志仅在界面显示。";

static LOG: OnceLock<Mutex<LogState>> = OnceLock::new();

#[derive(Debug, PartialEq)]
pub(crate) enum LogLevel {
    Warning,
    Error,
}

pub(crate) fn log_level(line: &str) -> Option<LogLevel> {
    if line.starts_with("Failed to start:") {
        return Some(LogLevel::Error);
    }
    // mihomo 的 logrus 文本格式以 time= 开头，级别是独立的 level= 字段。
    if line.starts_with("time=") {
        let level = line
            .split_whitespace()
            .find_map(|part| part.strip_prefix("level="))?;
        return match level {
            "error" | "fatal" | "panic" => Some(LogLevel::Error),
            "warning" | "warn" => Some(LogLevel::Warning),
            _ => None,
        };
    }
    let header = line.split_whitespace().find(|word| {
        !word.bytes().all(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'/' | b':' | b'.' | b'T' | b'Z')
        })
    })?;
    let level = header
        .trim_start_matches('[')
        .split(['[', ']', ':'])
        .next()?;
    if level.eq_ignore_ascii_case("error") || level.eq_ignore_ascii_case("fatal") {
        Some(LogLevel::Error)
    } else if level.eq_ignore_ascii_case("warn") || level.eq_ignore_ascii_case("warning") {
        Some(LogLevel::Warning)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogSource {
    SingBox,
    Xray,
    Mihomo,
    App,
}

impl LogSource {
    pub(crate) const ALL: [Self; 4] = [Self::Mihomo, Self::SingBox, Self::Xray, Self::App];

    pub(crate) fn index(self) -> usize {
        self as usize
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::SingBox => "sing-box",
            Self::Xray => "Xray",
            Self::Mihomo => "mihomo",
            Self::App => "Ladder",
        }
    }
}

#[derive(Default)]
struct LogState {
    path: Option<PathBuf>,
    file: Option<CappedFile>,
    recent: VecDeque<String>,
    cores: [VecDeque<String>; 3],
}

impl LogState {
    fn open(&mut self, path: &Path) -> io::Result<()> {
        self.path = Some(path.to_owned());
        self.file = None;
        self.recent.clear();
        for lines in &mut self.cores {
            lines.clear();
        }
        match File::create(path) {
            Ok(file) => {
                self.file = Some(CappedFile { file, written: 0 });
                Ok(())
            }
            Err(error) => {
                self.file_failed(&error);
                Err(error)
            }
        }
    }

    fn record(&mut self, line: String) {
        self.remember(line.clone());
        if let Some(file) = self.file.as_mut() {
            match file.append(&line) {
                Ok(false) => {}
                Ok(true) => {
                    self.file = None;
                    self.remember(format_record("WARN", "logging", LIMIT_WARNING));
                }
                Err(error) => self.file_failed(&error),
            }
        }
    }

    fn remember(&mut self, line: String) {
        if self.recent.len() == RECENT_LIMIT {
            self.recent.pop_front();
        }
        self.recent.push_back(line);
    }

    fn record_core(&mut self, core: Core, line: String) {
        let lines = &mut self.cores[core as usize];
        if lines.len() == RECENT_LIMIT {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    fn recent_for(&self, source: LogSource) -> Vec<String> {
        let lines = match source {
            LogSource::App => &self.recent,
            _ => &self.cores[source.index()],
        };
        lines.iter().cloned().collect()
    }

    fn file_failed(&mut self, error: &io::Error) {
        self.file = None;
        // 不格式化自定义 IO 错误中的外部文本。
        let reason = match error.raw_os_error() {
            Some(code) => format!("Windows 错误 {code}"),
            None => format!("I/O 类型 {:?}", error.kind()),
        };
        let message = format!(
            "应用日志无法写入文件（{reason}）。本次仅在界面保留最近 500 条日志，请检查日志目录权限或磁盘空间后重启 Ladder。"
        );
        self.remember(format_record("WARN", "logging", message));
    }
}

fn state() -> &'static Mutex<LogState> {
    LOG.get_or_init(|| Mutex::new(LogState::default()))
}

pub(crate) fn init(path: &Path) -> io::Result<()> {
    state().lock().unwrap_or_else(|e| e.into_inner()).open(path)
}

pub(crate) fn path() -> Option<PathBuf> {
    state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .path
        .clone()
}

pub(crate) fn recent_for(source: LogSource) -> Vec<String> {
    state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .recent_for(source)
}

#[cfg(test)]
pub(crate) fn recent() -> Vec<String> {
    recent_for(LogSource::App)
}

pub(crate) fn record_core(core: Core, line: String) {
    state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record_core(core, line);
}

pub(crate) fn info(target: &'static str, message: impl Display) {
    record("INFO", target, message);
}

pub(crate) fn warn(target: &'static str, message: impl Display) {
    record("WARN", target, message);
}

pub(crate) fn error(target: &'static str, message: impl Display) {
    record("ERROR", target, message);
}

fn record(level: &str, target: &str, message: impl Display) {
    let line = format_record(level, target, message);
    state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record(line);
}

pub(crate) fn install_panic_hook() {
    std::panic::set_hook(Box::new(|panic| {
        let line = match panic.location() {
            Some(location) => format_record(
                "ERROR",
                "panic",
                format_args!(
                    "线程发生异常：{}:{}:{}（异常内容不记录）",
                    location.file(),
                    location.line(),
                    location.column()
                ),
            ),
            None => format_record("ERROR", "panic", "线程发生异常（异常内容不记录）"),
        };
        // 日志写入本身发生 panic 时不能再次等待相同的锁。
        if let Some(state) = LOG.get()
            && let Ok(mut state) = state.try_lock()
        {
            state.record(line);
            if let Some(file) = state.file.as_ref() {
                let _ = file.file.sync_data();
            }
        }
        // 默认 hook 会输出可能包含配置和密钥的 panic payload，因此不转发。
    }));
}

#[cfg(test)]
pub(crate) fn with_locked_log(f: impl FnOnce()) {
    let _guard = state().lock().unwrap();
    f();
}

// Display 分段写入有界缓冲区，避免超长错误先分配完整字符串。
struct Line {
    text: String,
    truncated: bool,
}

impl Line {
    fn new() -> Self {
        Self {
            text: String::new(),
            truncated: false,
        }
    }

    fn finish(mut self) -> String {
        if self.truncated {
            self.text.push_str("...");
        }
        self.text
    }
}

impl fmt::Write for Line {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.truncated {
            return Ok(());
        }
        for ch in text.chars() {
            let mut bytes = [0; 4];
            let escaped;
            let next = if ch.is_control() {
                escaped = ch.escape_default().to_string();
                escaped.as_str()
            } else {
                ch.encode_utf8(&mut bytes)
            };
            // 为截断标记和落盘换行预留空间。
            if self.text.len() + next.len() > RECORD_LIMIT - 4 {
                self.truncated = true;
                break;
            }
            self.text.push_str(next);
        }
        Ok(())
    }
}

fn format_record(level: &str, target: &str, message: impl Display) -> String {
    let mut time = SYSTEMTIME::default();
    // 按当前 Windows 时区读取本机时间。
    unsafe { GetLocalTime(&mut time) };
    let mut line = Line::new();
    let _ = write!(
        line,
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} [{level}] [{target}] {message}",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        time.wMilliseconds
    );
    line.finish()
}

struct CappedFile {
    file: File,
    written: u64,
}

impl CappedFile {
    fn append(&mut self, line: &str) -> io::Result<bool> {
        let bytes = line.len() as u64 + 1;
        let notice = format_record("WARN", "logging", LIMIT_WARNING);
        let reserved = notice.len() as u64 + 1;
        // 将最后一条提示也计入上限，不留下半条普通记录。
        if self.written + bytes + reserved > FILE_LIMIT {
            self.file.write_all(notice.as_bytes())?;
            self.file.write_all(b"\n")?;
            return Ok(true);
        }
        self.file.write_all(line.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.written += bytes;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn recognizes_application_and_core_levels_without_matching_message_text() {
        for (line, expected) in [
            (
                "2026-11-17 10:20:30.123 [ERROR] [app] failed",
                Some(LogLevel::Error),
            ),
            (
                "2026/11/17 10:20:30 [Warning] core: warning",
                Some(LogLevel::Warning),
            ),
            (
                "+0800 2026-11-17 10:20:30 ERROR [123 0ms] failed",
                Some(LogLevel::Error),
            ),
            ("WARN[0000] deprecated", Some(LogLevel::Warning)),
            ("[Error] failed", Some(LogLevel::Error)),
            (
                "Failed to start: main: failed to load config files",
                Some(LogLevel::Error),
            ),
            (
                "time=\"2026-11-17T10:20:30Z\" level=error msg=\"failed\"",
                Some(LogLevel::Error),
            ),
            (
                "time=\"2026-11-17T10:20:30Z\" level=warning msg=\"warning\"",
                Some(LogLevel::Warning),
            ),
            (
                "2026-11-17T10:20:30Z [WARN] warning",
                Some(LogLevel::Warning),
            ),
            ("2026-11-17 10:20:30 [INFO] [app] error count is zero", None),
            ("INFO[0000] error count is zero", None),
            ("INFO Failed to start: is a diagnostic prefix", None),
            ("upstream reports Failed to start: text", None),
            ("connection reports an error", None),
            ("", None),
        ] {
            assert_eq!(log_level(line), expected, "{line}");
        }
    }
    #[test]
    fn reopening_replaces_previous_session_without_loading_history() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app.log");
        let mut first = LogState::default();
        first.open(&path).unwrap();
        first.record("first".into());
        first.record_core(Core::Xray, "old core output".into());
        first.open(&path).unwrap();
        assert!(first.recent.is_empty());
        assert!(first.recent_for(LogSource::Xray).is_empty());
        assert_eq!(fs::metadata(&path).unwrap().len(), 0);
        first.record("second".into());
        assert_eq!(fs::read_to_string(&path).unwrap(), "second\n");
        assert_eq!(first.recent, ["second"]);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn core_streams_are_bounded_separate_and_never_written_to_app_log() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app.log");
        let mut state = LogState::default();
        state.open(&path).unwrap();
        state.record("application event".into());
        for core in Core::ALL {
            for index in 0..600 {
                state.record_core(core, format!("{} {index}", core.label()));
            }
        }
        for source in [LogSource::SingBox, LogSource::Xray] {
            let lines = state.recent_for(source);
            assert_eq!(lines.len(), RECENT_LIMIT);
            assert!(lines.first().unwrap().ends_with(" 100"));
            assert!(lines.last().unwrap().ends_with(" 599"));
            let expected = if source == LogSource::SingBox {
                "sing-box "
            } else {
                "xray "
            };
            assert!(lines.iter().all(|line| line.starts_with(expected)));
        }
        assert_eq!(state.recent_for(LogSource::App), ["application event"]);
        assert_eq!(fs::read_to_string(path).unwrap(), "application event\n");
    }

    #[test]
    fn file_limit_stops_writing_once_and_memory_keeps_recent_events() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("ladder.log");
        let mut state = LogState::default();
        state.open(&path).unwrap();
        let message = "字".repeat(RECORD_LIMIT);
        for number in 0..1300 {
            state.record(format_record(
                "INFO",
                "test",
                format_args!("{number:04} {message}"),
            ));
        }
        assert_eq!(state.recent.len(), RECENT_LIMIT);
        assert!(state.recent.front().unwrap().contains("0800 "));
        assert!(state.recent.back().unwrap().contains("1299 "));
        assert!(state.file.is_none());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        assert!(fs::metadata(&path).unwrap().len() <= FILE_LIMIT);
        let data = fs::read_to_string(&path).unwrap();
        assert!(data.ends_with('\n'));
        assert!(data.lines().all(|line| line.len() < RECORD_LIMIT));
        assert_eq!(data.matches(LIMIT_WARNING).count(), 1);
        assert!(data.lines().last().unwrap().contains(LIMIT_WARNING));
        assert!(!data.contains("1299 "));
        state.record("after limit".into());
        assert_eq!(fs::read_to_string(path).unwrap(), data);
    }

    #[test]
    fn concurrent_records_remain_whole_and_immediately_readable() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("ladder.log");
        let mut state = LogState::default();
        state.open(&path).unwrap();
        let state = Arc::new(Mutex::new(state));
        let workers: Vec<_> = (0..4)
            .map(|worker| {
                let state = state.clone();
                thread::spawn(move || {
                    for index in 0..100 {
                        state
                            .lock()
                            .unwrap()
                            .record(format!("worker={worker} record={index}"));
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let text = fs::read_to_string(&path).unwrap();
        let lines: std::collections::HashSet<_> = text.lines().collect();
        assert_eq!(lines.len(), 400);
        for worker in 0..4 {
            for index in 0..100 {
                assert!(lines.contains(format!("worker={worker} record={index}").as_str()));
            }
        }
        assert_eq!(state.lock().unwrap().recent.len(), 400);
    }

    #[test]
    fn file_failures_preserve_memory_without_repeating_io() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("ladder.log");
        fs::create_dir(&path).unwrap();
        let mut state = LogState::default();
        assert!(state.open(&path).is_err());
        assert!(state.recent.back().unwrap().contains("[WARN] [logging]"));
        for number in 0..600 {
            state.record(format!("event {number}"));
        }
        assert_eq!(state.recent.len(), RECENT_LIMIT);
        assert_eq!(state.recent.front().unwrap(), "event 100");
        assert!(state.file.is_none());

        let path = root.path().join("readonly.log");
        state.open(&path).unwrap();
        // 真实只读句柄模拟已初始化后的写入失败。
        state.file.as_mut().unwrap().file = File::open(&path).unwrap();
        state.record("write failed".into());
        assert!(state.recent.back().unwrap().contains("[WARN] [logging]"));
        assert!(state.file.is_none());
        state.record("still available".into());
        assert_eq!(state.recent.back().unwrap(), "still available");
    }

    #[test]
    fn records_escape_controls_and_truncate_without_splitting_utf8() {
        let line = format_record("WARN", "test", "一行\n伪造\r\n\t\0\u{1b}[31m");
        assert!(line.contains(" [WARN] [test] 一行\\n伪造\\r\\n\\t\\u{0}\\u{1b}[31m"));
        assert!(!line.chars().any(char::is_control));
        let line = format_record("INFO", "test", "🪜".repeat(RECORD_LIMIT));
        assert!(line.len() < RECORD_LIMIT);
        assert!(line.ends_with("..."));
        assert!(!line.contains('\u{fffd}'));
    }

    #[test]
    fn panic_hook_records_location_without_payload() {
        const CHILD_LOG: &str = "LADDER_TEST_PANIC_LOG";
        if let Some(path) = std::env::var_os(CHILD_LOG) {
            init(Path::new(&path)).unwrap();
            install_panic_hook();
            let caught = std::panic::catch_unwind(|| panic!("PRIVATE_PANIC_PAYLOAD"));
            assert!(caught.is_err());
            info("test", "异常捕获后继续记录");
            return;
        }

        // 独立测试进程安装全局 hook，不改变并行用例的异常处理。
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app.log");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "logging::tests::panic_hook_records_location_without_payload",
                "--nocapture",
            ])
            .env(CHILD_LOG, &path)
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains("[ERROR] [panic]"));
        assert!(text.contains("logging.rs:"));
        assert!(text.contains("异常捕获后继续记录"));
        for content in [
            text,
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ] {
            assert!(!content.contains("PRIVATE_PANIC_PAYLOAD"));
        }
    }
}
