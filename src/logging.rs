//! 日志初始化、输出格式与受限文件写入。

use std::fmt;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, MakeWriter};
use tracing_subscriber::registry::LookupSpan;
use windows::Win32::System::Console::{
    CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle,
    STD_ERROR_HANDLE, SetConsoleMode,
};
use windows::Win32::System::SystemInformation::GetSystemTime;

use crate::app_error::AppError;
use crate::settings::Log;

/// 自定义日志格式: `{时间戳}{毫秒}Z [{级别}] {消息}`, 级别带 ANSI 颜色。
struct BracketedLevel;

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_BLUE: &str = "\x1b[34m";

fn level_color(level: &Level) -> &'static str {
    match *level {
        Level::INFO => ANSI_GREEN,
        Level::WARN => ANSI_YELLOW,
        Level::ERROR => ANSI_RED,
        Level::DEBUG => ANSI_BLUE,
        _ => "",
    }
}

impl<S, N> FormatEvent<S, N> for BracketedLevel
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let level = event.metadata().level();
        let color = level_color(level);
        let reset = if writer.has_ansi_escapes() {
            ANSI_RESET
        } else {
            ""
        };
        let lc = if writer.has_ansi_escapes() { color } else { "" };
        write_utc_timestamp(&mut writer)?;
        write!(&mut writer, " [{lc}{level}{reset}] ")?;
        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// 把当前 UTC 时间戳直接写进输出, 避免每条日志都分配一个 String。
fn write_utc_timestamp(writer: &mut Writer<'_>) -> fmt::Result {
    let t = unsafe { GetSystemTime() };
    write!(
        writer,
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

/// 达到日志上限时写入的最后一条提示。
const LOG_LIMIT_NOTICE: &[u8] =
    "\n[日志已达大小上限, 后续内容不再写入。可在 settings.json 的 log.max_size_mb 调整]\n"
        .as_bytes();

/// 带大小上限的日志文件。
///
/// 默认日志级别是 debug, 而子进程的 stderr 每一行都会被转发成 warn。核心进入
/// 错误重试循环时会持续刷日志, 长期挂机可能写出很大的文件。达到上限后追加一
/// 条提示, 之后静默丢弃。
struct CappedFile {
    file: File,
    written: u64,
    /// 上限字节数, 0 表示不限制。
    limit: u64,
    notified: bool,
}

impl Write for CappedFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.limit == 0 {
            return self.file.write(buf);
        }
        if self.written >= self.limit {
            if !self.notified {
                self.notified = true;
                let _ = self.file.write_all(LOG_LIMIT_NOTICE);
                let _ = self.file.flush();
            }
            // 报告写入成功: 返回错误只会让 tracing 反复往 stderr 抱怨
            return Ok(buf.len());
        }
        let n = self.file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// `CappedFile` 的共享句柄, 供 tracing 的 `MakeWriter` 使用。
#[derive(Clone)]
struct CappedLogWriter(Arc<Mutex<CappedFile>>);

impl<'a> MakeWriter<'a> for CappedLogWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Write for CappedLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).flush()
    }
}

/// 初始化日志系统。
///
/// 1. 为 Windows 控制台启用 ANSI 转义码支持 (彩色输出)
/// 2. 创建 console_layer (stderr) 和 file_layer (app.log, 带大小上限)
/// 3. 日志级别优先使用 RUST_LOG 环境变量, 否则使用 settings.json 中的配置
pub(crate) fn init_logging(exe_dir: &Path, log: &Log) -> Result<(), AppError> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    unsafe {
        if let Ok(handle) = GetStdHandle(STD_ERROR_HANDLE) {
            let mut mode = CONSOLE_MODE::default();
            if GetConsoleMode(handle, &mut mode).is_ok() {
                let _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
    }

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&log.level));

    let console_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .compact()
        .event_format(BracketedLevel)
        .with_writer(io::stderr);

    let log_path = exe_dir.join("app.log");
    let file =
        File::create(log_path).map_err(|e| AppError::Msg(format!("创建日志文件失败: {e}")))?;
    let writer = CappedLogWriter(Arc::new(Mutex::new(CappedFile {
        file,
        written: 0,
        limit: log.max_size_mb.saturating_mul(1024 * 1024),
        notified: false,
    })));
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(false)
        .compact()
        .event_format(BracketedLevel)
        .with_writer(writer);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(console_layer)
        .with(filter)
        .try_init()
        .map_err(|e| AppError::Msg(format!("初始化日志系统失败: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn capped(path: &Path, limit: u64) -> CappedFile {
        CappedFile {
            file: File::create(path).expect("应能在测试临时目录创建日志文件"),
            written: 0,
            limit,
            notified: false,
        }
    }

    #[test]
    fn test_capped_file_stops_growing_at_limit() {
        let dir = tempfile::tempdir().expect("应能创建日志测试临时目录");
        let path = dir.path().join("app.log");
        let mut writer = capped(&path, 100);

        for _ in 0..10 {
            assert_eq!(
                writer
                    .write(&[b'x'; 40])
                    .expect("日志文件写入或超限丢弃应成功"),
                40,
                "超限后也应报告写入成功"
            );
        }
        writer.flush().expect("应能刷新测试日志文件");
        drop(writer);

        // 前三次写满 120 字节后追加一条提示, 之后不再增长
        let size = fs::metadata(&path)
            .expect("应能读取测试日志文件元数据")
            .len();
        assert_eq!(size, 120 + LOG_LIMIT_NOTICE.len() as u64);
    }

    #[test]
    fn test_capped_file_zero_limit_is_unlimited() {
        let dir = tempfile::tempdir().expect("应能创建日志测试临时目录");
        let path = dir.path().join("app.log");
        let mut writer = capped(&path, 0);

        for _ in 0..10 {
            writer
                .write_all(&[b'x'; 40])
                .expect("不限量日志文件应能写入完整测试数据");
        }
        writer.flush().expect("应能刷新测试日志文件");
        drop(writer);

        assert_eq!(
            fs::metadata(&path)
                .expect("应能读取测试日志文件元数据")
                .len(),
            400
        );
    }
}
