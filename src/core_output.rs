use std::io::{self, Read};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::{logging, runtime::Core};

pub(crate) struct CoreOutput {
    stdout: JoinHandle<Option<&'static str>>,
    stderr: JoinHandle<Option<&'static str>>,
    error: Arc<Mutex<Option<String>>>,
}

#[derive(Debug, Default)]
pub(crate) struct CoreDiagnostics {
    pub(crate) reason: Option<&'static str>,
    pub(crate) error: Option<String>,
}

impl CoreOutput {
    pub(crate) fn capture(
        core: Core,
        stdout: impl Read + Send + 'static,
        stderr: impl Read + Send + 'static,
    ) -> Self {
        let error = Arc::new(Mutex::new(None));
        let stdout_error = error.clone();
        let stderr_error = error.clone();
        Self {
            stdout: thread::spawn(move || capture_reason(core, "stdout", stdout, &stdout_error)),
            stderr: thread::spawn(move || capture_reason(core, "stderr", stderr, &stderr_error)),
            error,
        }
    }

    pub(crate) fn error(&self) -> Option<String> {
        self.error
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    // 核心退出后等待两条管道读完, 再生成诊断。
    pub(crate) fn finish(self) -> CoreDiagnostics {
        let stdout = self
            .stdout
            .join()
            .unwrap_or(Some("核心输出读取线程异常退出"));
        let stderr = self
            .stderr
            .join()
            .unwrap_or(Some("核心输出读取线程异常退出"));
        let error = self
            .error
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        CoreDiagnostics {
            reason: stderr.or(stdout),
            error,
        }
    }
}

fn capture_reason(
    core: Core,
    stream: &'static str,
    pipe: impl Read,
    output_error: &Mutex<Option<String>>,
) -> Option<&'static str> {
    match read_output(pipe, |line| {
        if logging::log_level(&line) == Some(logging::LogLevel::Error) {
            *output_error
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(line.clone());
        }
        logging::record_core(core, line);
    }) {
        Ok(()) => None,
        Err(error) => {
            // IO 错误的自定义文本也可能来自核心，日志只记录系统代码和种类。
            logging::warn(
                "core",
                format_args!(
                    "{} {stream} 读取失败（{:?}，系统错误码 {:?}）",
                    core.label(),
                    error.kind(),
                    error.raw_os_error()
                ),
            );
            Some("读取核心输出失败，诊断可能不完整")
        }
    }
}

fn read_output(mut pipe: impl Read, mut on_line: impl FnMut(String)) -> io::Result<()> {
    let mut buffer = [0; 4096];
    let mut lines = OutputLines::default();
    loop {
        let size = match pipe.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                lines.finish(&mut on_line);
                return Err(error);
            }
            Ok(size) => size,
        };
        if size == 0 {
            break;
        }
        lines.push(&buffer[..size], &mut on_line);
    }
    lines.finish(&mut on_line);
    Ok(())
}

const LINE_LIMIT: usize = 8 * 1024;

pub(crate) fn sanitized_output(bytes: &[u8]) -> String {
    let mut output = Vec::new();
    let mut lines = OutputLines::default();
    lines.push(bytes, &mut |line| output.push(line));
    lines.finish(&mut |line| output.push(line));
    output.join("\n")
}

#[derive(Default)]
enum Escape {
    #[default]
    None,
    Start,
    Csi,
    Osc,
    OscEnd,
}

#[derive(Default)]
struct OutputLines {
    line: Vec<u8>,
    truncated: bool,
    escape: Escape,
}

impl OutputLines {
    fn push(&mut self, bytes: &[u8], on_line: &mut impl FnMut(String)) {
        for &byte in bytes {
            if byte == b'\n' {
                self.finish(on_line);
                self.escape = Escape::None;
                continue;
            }
            match self.escape {
                Escape::None => match byte {
                    0x1b => self.escape = Escape::Start,
                    b'\r' => {}
                    _ if byte < 0x20 || byte == 0x7f => {}
                    _ => {
                        if self.line.len() < LINE_LIMIT {
                            self.line.push(byte);
                        } else {
                            self.truncated = true;
                        }
                    }
                },
                Escape::Start => {
                    self.escape = match byte {
                        b'[' => Escape::Csi,
                        b']' => Escape::Osc,
                        _ => Escape::None,
                    };
                }
                Escape::Csi => {
                    if (0x40..=0x7e).contains(&byte) {
                        self.escape = Escape::None;
                    }
                }
                Escape::Osc => match byte {
                    0x07 => self.escape = Escape::None,
                    0x1b => self.escape = Escape::OscEnd,
                    _ => {}
                },
                Escape::OscEnd => {
                    self.escape = if byte == b'\\' {
                        Escape::None
                    } else {
                        Escape::Osc
                    };
                }
            }
        }
    }

    fn finish(&mut self, on_line: &mut impl FnMut(String)) {
        if self.line.is_empty() && !self.truncated {
            return;
        }
        let mut text: String = String::from_utf8_lossy(&self.line)
            .chars()
            .filter(|ch| !ch.is_control())
            .collect();
        if self.truncated || text.len() > LINE_LIMIT {
            let mut end = (LINE_LIMIT - 3).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str("...");
        }
        if !text.is_empty() {
            on_line(text);
        }
        self.line.clear();
        self.truncated = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::Duration;

    #[test]
    fn streams_utf8_crlf_and_ansi_split_at_every_byte() {
        let text = "\u{1b}[31mERROR 中文🪜\u{1b}[0m\r\n\u{1b}]8;;https://example.invalid\u{7}link\u{1b}]8;;\u{1b}\\\nlast line";
        for split in 1..text.len() {
            let pipe = Cursor::new(&text.as_bytes()[..split])
                .chain(Cursor::new(&text.as_bytes()[split..]));
            let mut lines = Vec::new();
            read_output(pipe, |line| lines.push(line)).unwrap();
            assert_eq!(
                lines,
                ["ERROR 中文🪜", "link", "last line"],
                "split {split}"
            );
        }
    }

    #[test]
    fn long_lines_are_bounded_without_splitting_utf8() {
        let text = format!("{} unsupported\nnext\n", "界".repeat(LINE_LIMIT));
        let mut lines = Vec::new();
        read_output(Cursor::new(text), |line| lines.push(line)).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].len() <= LINE_LIMIT);
        assert!(lines[0].ends_with("..."));
        assert!(!lines[0].contains('\u{fffd}'));
        assert_eq!(lines[1], "next");
    }

    #[test]
    fn credential_markers_and_multiline_text_are_preserved() {
        let text = concat!(
            "INFO ready\n",
            "ERROR unsupported Password=fixture-value\n",
            "Authorization: Bearer fixture-value\n",
            "endpoint https://user:fixture-value@example.invalid/path\n",
            "outbound vmess://fixture-value\n",
            "-----BEGIN RSA PRIVATE KEY-----\n",
            "base64-fixture\n",
            "-----END RSA PRIVATE KEY-----\n",
            "INFO still running\n",
        );
        let mut lines = Vec::new();
        read_output(Cursor::new(text), |line| lines.push(line)).unwrap();
        assert_eq!(lines, text.lines().collect::<Vec<_>>());
        assert_eq!(sanitized_output(text.as_bytes()), text.trim_end());
    }

    #[test]
    fn truncated_credential_markers_do_not_change_following_lines() {
        for offset in [LINE_LIMIT - 8, LINE_LIMIT, LINE_LIMIT * 2] {
            let padding = "x".repeat(offset);
            let text = format!(
                "{padding}-----BEGIN PRIVATE KEY-----\nQUJDREVGRw==\n\
                 {padding}-----END PRIVATE KEY-----\nINFO still running\n"
            );
            let mut lines = Vec::new();
            read_output(Cursor::new(text), |line| lines.push(line)).unwrap();
            assert_eq!(lines.len(), 4, "marker offset {offset}");
            assert!(lines[0].len() <= LINE_LIMIT && lines[0].ends_with("..."));
            assert_eq!(lines[1], "QUJDREVGRw==");
            assert!(lines[2].len() <= LINE_LIMIT && lines[2].ends_with("..."));
            assert_eq!(lines[3], "INFO still running");
        }
    }

    #[test]
    fn captured_lines_are_available_before_the_pipes_close() {
        use std::io::Write;
        use std::time::Instant;

        let (reader, mut writer) = io::pipe().unwrap();
        let output = CoreOutput::capture(Core::Xray, reader, Cursor::new(b""));
        writer.write_all(b"core-output-live-before-eof\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !logging::recent_for(logging::LogSource::Xray)
            .iter()
            .any(|line| line == "core-output-live-before-eof")
        {
            assert!(
                Instant::now() < deadline,
                "line was not available while the pipe was open"
            );
            thread::sleep(Duration::from_millis(5));
        }
        drop(writer);
        assert_eq!(output.finish().reason, None);
    }

    #[test]
    fn live_errors_are_bounded_preserved_and_independent_before_eof() {
        use std::io::Write;
        use std::time::Instant;

        let (reader, mut writer) = io::pipe().unwrap();
        let output = CoreOutput::capture(Core::Mihomo, reader, Cursor::new(b""));
        let wait_error = |output: &CoreOutput, expected: &str| {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if let Some(error) = output.error()
                    && error.contains(expected)
                {
                    return error;
                }
                assert!(
                    Instant::now() < deadline,
                    "live error was not available before EOF"
                );
                thread::sleep(Duration::from_millis(5));
            }
        };
        writeln!(
            writer,
            "time=\"fixture\" level=error msg=\"bounded-live-error {}\"",
            "界".repeat(LINE_LIMIT)
        )
        .unwrap();
        let error = wait_error(&output, "bounded-live-error");
        assert!(error.len() <= LINE_LIMIT && error.ends_with("..."));
        writeln!(writer, "ERROR password=LIVE_FIXTURE_VALUE").unwrap();
        let original = wait_error(&output, "LIVE_FIXTURE_VALUE");
        assert_eq!(original, "ERROR password=LIVE_FIXTURE_VALUE");
        let other = CoreOutput::capture(
            Core::Xray,
            Cursor::new(b"[Error] separate-live-error\n"),
            Cursor::new(b""),
        );
        assert_eq!(
            wait_error(&other, "separate-live-error"),
            "[Error] separate-live-error"
        );
        assert_eq!(output.error(), Some(original));
        assert_eq!(other.finish().reason, None);
        drop(writer);
        assert_eq!(output.finish().reason, None);
        assert!(
            logging::recent_for(logging::LogSource::Mihomo)
                .iter()
                .any(|line| line == "ERROR password=LIVE_FIXTURE_VALUE")
        );
        assert!(
            !logging::recent()
                .iter()
                .any(|line| line.contains("bounded-live-error")
                    || line.contains("separate-live-error")
                    || line.contains("LIVE_FIXTURE_VALUE"))
        );
    }

    #[test]
    fn finished_diagnostics_preserve_final_unterminated_native_errors() {
        for text in [
            "Failed to start: main: cannot find the file",
            "Failed to start: main: cannot find the file; password=EXIT_FIXTURE_VALUE",
        ] {
            let output = CoreOutput::capture(
                Core::Xray,
                Cursor::new(b""),
                Cursor::new(text.as_bytes().to_vec()),
            );
            let diagnostics = output.finish();
            assert_eq!(diagnostics.reason, None);
            assert_eq!(diagnostics.error.as_deref(), Some(text));
        }
        assert!(
            !logging::recent().iter().any(
                |line| line.contains("Failed to start:") || line.contains("EXIT_FIXTURE_VALUE")
            )
        );
        assert!(
            logging::recent_for(logging::LogSource::Xray)
                .iter()
                .any(|line| line.contains("EXIT_FIXTURE_VALUE"))
        );
    }

    #[test]
    fn preserves_native_errors_split_at_every_byte() {
        for phrase in [
            "address already in use",
            "only one usage",
            "access is denied",
            "permission denied",
            "unknown field",
            "unsupported",
            "unknown type",
            "unknown inbound",
            "unknown outbound",
            "unknown config id",
            "cannot find the file",
            "no such file",
        ] {
            for split in 1..phrase.len() {
                let pipe = Cursor::new(&phrase.as_bytes()[..split])
                    .chain(Cursor::new(&phrase.as_bytes()[split..]));
                let mut lines = Vec::new();
                read_output(pipe, |line| lines.push(line)).unwrap();
                assert_eq!(lines, [phrase], "split {split} of {phrase}");
            }
        }
        let mut text = vec![b'x'; 4090];
        text.extend_from_slice(b"ACCESS IS DENIED: PRIVATE_FIXTURE_VALUE");
        let mut lines = Vec::new();
        read_output(Cursor::new(&text), |line| lines.push(line)).unwrap();
        assert_eq!(lines, [String::from_utf8(text).unwrap()]);
    }

    #[test]
    fn finish_waits_for_delayed_output() {
        struct DelayedReader {
            gate: Option<Arc<Barrier>>,
            text: Cursor<&'static [u8]>,
        }
        impl Read for DelayedReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if let Some(gate) = self.gate.take() {
                    gate.wait();
                }
                self.text.read(buffer)
            }
        }
        let gate = Arc::new(Barrier::new(2));
        let output = CoreOutput::capture(
            Core::SingBox,
            Cursor::new(b"unknown field"),
            DelayedReader {
                gate: Some(gate.clone()),
                text: Cursor::new(b"ERROR access is denied: PRIVATE_FIXTURE_VALUE"),
            },
        );
        let (send, receive) = mpsc::channel();
        let finished = thread::spawn(move || send.send(output.finish()).unwrap());
        assert!(receive.try_recv().is_err());
        gate.wait();
        let diagnostics = receive.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(diagnostics.reason, None);
        assert_eq!(
            diagnostics.error.as_deref(),
            Some("ERROR access is denied: PRIVATE_FIXTURE_VALUE")
        );
        finished.join().unwrap();
    }

    #[test]
    fn interrupted_reads_retry_and_failures_return_a_safe_summary() {
        struct FailingReader(bool);
        impl Read for FailingReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                if std::mem::take(&mut self.0) {
                    Err(io::ErrorKind::Interrupted.into())
                } else {
                    Err(io::Error::other("PRIVATE_PIPE_ERROR"))
                }
            }
        }
        assert_eq!(
            read_output(FailingReader(true), |_| {}).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(
            capture_reason(Core::Xray, "stderr", FailingReader(true), &Mutex::new(None)),
            Some("读取核心输出失败，诊断可能不完整")
        );
        assert!(
            !logging::recent()
                .iter()
                .any(|line| line.contains("PRIVATE_PIPE_ERROR"))
        );
    }
}
