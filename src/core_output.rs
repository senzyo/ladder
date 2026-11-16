use std::io::{self, Read};
use std::thread::{self, JoinHandle};

use crate::{logging, runtime::Core};

pub(crate) struct CoreOutput {
    stdout: JoinHandle<Option<&'static str>>,
    stderr: JoinHandle<Option<&'static str>>,
}

impl CoreOutput {
    pub(crate) fn capture(
        core: Core,
        stdout: impl Read + Send + 'static,
        stderr: impl Read + Send + 'static,
    ) -> Self {
        Self {
            stdout: thread::spawn(move || capture_reason(core, "stdout", stdout)),
            stderr: thread::spawn(move || capture_reason(core, "stderr", stderr)),
        }
    }

    // 核心退出后等待两条管道读完, 再生成诊断。
    pub(crate) fn finish(self) -> Option<&'static str> {
        let stdout = self
            .stdout
            .join()
            .unwrap_or(Some("核心输出读取线程异常退出"));
        let stderr = self
            .stderr
            .join()
            .unwrap_or(Some("核心输出读取线程异常退出"));
        stderr.or(stdout)
    }
}

fn capture_reason(core: Core, stream: &'static str, pipe: impl Read) -> Option<&'static str> {
    match read_output(pipe, |line| logging::record_core(core, line)) {
        Ok(reason) => reason,
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

fn read_output(
    mut pipe: impl Read,
    mut on_line: impl FnMut(String),
) -> io::Result<Option<&'static str>> {
    let mut buffer = [0; 4096];
    let mut pending = Vec::with_capacity(buffer.len() + 32);
    let mut lines = OutputLines::default();
    let mut reason = None;
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
        pending.extend_from_slice(&buffer[..size]);
        let text = String::from_utf8_lossy(&pending).to_ascii_lowercase();
        let matched = if text.contains("address already in use") || text.contains("only one usage")
        {
            Some("监听端口或地址已被占用")
        } else if text.contains("access is denied") || text.contains("permission denied") {
            Some("核心没有访问所需资源的权限")
        } else if text.contains("unknown field")
            || text.contains("unsupported")
            || text.contains("unknown type")
            || text.contains("unknown inbound")
            || text.contains("unknown outbound")
            || text.contains("unknown config id")
        {
            Some("当前核心不支持配置中的字段或类型")
        } else if text.contains("cannot find the file") || text.contains("no such file") {
            Some("核心找不到配置引用的资源文件")
        } else {
            None
        };
        reason = matched.or(reason);
        lines.push(&buffer[..size], &mut on_line);
        // 诊断只保留跨次读取所需的尾部，显示用的行缓冲单独限长。
        let keep = pending.len().min(32);
        let start = pending.len() - keep;
        pending.copy_within(start.., 0);
        pending.truncate(keep);
    }
    lines.finish(&mut on_line);
    Ok(reason)
}

const LINE_LIMIT: usize = 8 * 1024;
const HIDDEN_LINE: &str = "[已隐藏敏感日志]";

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
    redaction: Vec<u8>,
    truncated: bool,
    escape: Escape,
    private_key: bool,
    hidden: bool,
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
                        self.redaction.push(byte);
                        if self.redaction.len() == LINE_LIMIT {
                            self.scan_redaction();
                            // 保留跨缓冲区的凭据和 PEM 标记；显示截断后仍继续扫描。
                            self.redaction.drain(..LINE_LIMIT - 64);
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

    fn scan_redaction(&mut self) {
        let lower: String = String::from_utf8_lossy(&self.redaction)
            .chars()
            .filter(|ch| !ch.is_control())
            .flat_map(char::to_lowercase)
            .collect();
        let begins_key = lower.contains("-----begin ") && lower.contains("private key-----");
        self.hidden |= self.private_key || begins_key || sensitive_line(&lower);
        self.private_key |= begins_key;
        if lower.contains("-----end ") && lower.contains("private key-----") {
            self.private_key = false;
        }
    }

    fn finish(&mut self, on_line: &mut impl FnMut(String)) {
        if self.line.is_empty() && !self.truncated {
            return;
        }
        self.scan_redaction();
        let mut text: String = String::from_utf8_lossy(&self.line)
            .chars()
            .filter(|ch| !ch.is_control())
            .collect();
        if self.hidden {
            text = HIDDEN_LINE.into();
        } else if self.truncated || text.len() > LINE_LIMIT {
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
        self.redaction.clear();
        self.truncated = false;
        self.hidden = false;
    }
}

// 保守隐藏明确的凭据标记；不将核心文本送入应用诊断或磁盘日志。
fn sensitive_line(lower: &str) -> bool {
    [
        "password",
        "passwd",
        "token",
        "secret",
        "privatekey",
        "private_key",
        "private key",
        "api_key",
        "api-key",
        "apikey",
        "authorization",
        "bearer ",
        "uuid",
        "密码",
        "密钥",
        "令牌",
        "ss://",
        "ssr://",
        "vmess://",
        "vless://",
        "trojan://",
        "hysteria2://",
        "hy2://",
        "tuic://",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || lower.split("://").skip(1).any(|rest| {
            rest.split(['/', '?', '#', ' ', '\t', '"', '\''])
                .next()
                .is_some_and(|authority| authority.contains('@'))
        })
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
            assert_eq!(read_output(pipe, |line| lines.push(line)).unwrap(), None);
            assert_eq!(
                lines,
                ["ERROR 中文🪜", "link", "last line"],
                "split {split}"
            );
        }
    }

    #[test]
    fn long_lines_are_bounded_and_diagnostics_survive_truncation() {
        let text = format!("{} unsupported\nnext\n", "界".repeat(LINE_LIMIT));
        let mut lines = Vec::new();
        let reason = read_output(Cursor::new(text), |line| lines.push(line)).unwrap();
        assert_eq!(reason, Some("当前核心不支持配置中的字段或类型"));
        assert_eq!(lines.len(), 2);
        assert!(lines[0].len() <= LINE_LIMIT);
        assert!(lines[0].ends_with("..."));
        assert!(!lines[0].contains('\u{fffd}'));
        assert_eq!(lines[1], "next");
    }

    #[test]
    fn explicit_credentials_and_multiline_private_keys_are_hidden() {
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
        let reason = read_output(Cursor::new(text), |line| lines.push(line)).unwrap();
        assert_eq!(reason, Some("当前核心不支持配置中的字段或类型"));
        assert_eq!(lines.first().unwrap(), "INFO ready");
        assert!(lines[1..8].iter().all(|line| line == HIDDEN_LINE));
        assert_eq!(lines.last().unwrap(), "INFO still running");
    }

    #[test]
    fn private_key_markers_survive_line_truncation_and_scan_boundaries() {
        for offset in [LINE_LIMIT - 8, LINE_LIMIT, LINE_LIMIT * 2] {
            let padding = "x".repeat(offset);
            let text = format!(
                "{padding}-----BEGIN PRIVATE KEY-----\nQUJDREVGRw==\n\
                 {padding}-----END PRIVATE KEY-----\nINFO still running\n"
            );
            let mut lines = Vec::new();
            read_output(Cursor::new(text), |line| lines.push(line)).unwrap();
            assert_eq!(
                lines,
                [HIDDEN_LINE, HIDDEN_LINE, HIDDEN_LINE, "INFO still running"],
                "marker offset {offset}"
            );
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
        assert_eq!(output.finish(), None);
    }

    #[test]
    fn classifies_errors_split_at_every_byte() {
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
            let expected = read_output(Cursor::new(phrase.as_bytes()), |_| {}).unwrap();
            assert!(expected.is_some());
            for split in 1..phrase.len() {
                let pipe = Cursor::new(&phrase.as_bytes()[..split])
                    .chain(Cursor::new(&phrase.as_bytes()[split..]));
                assert_eq!(
                    read_output(pipe, |_| {}).unwrap(),
                    expected,
                    "split {split} of {phrase}"
                );
            }
        }
        let mut text = vec![b'x'; 4090];
        text.extend_from_slice(b"ACCESS IS DENIED: PRIVATE_FIXTURE_VALUE");
        assert_eq!(
            read_output(Cursor::new(text), |_| {}).unwrap(),
            Some("核心没有访问所需资源的权限")
        );
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
                text: Cursor::new(b"access is denied: PRIVATE_FIXTURE_VALUE"),
            },
        );
        let (send, receive) = mpsc::channel();
        let finished = thread::spawn(move || send.send(output.finish()).unwrap());
        assert!(receive.try_recv().is_err());
        gate.wait();
        assert_eq!(
            receive.recv_timeout(Duration::from_secs(5)).unwrap(),
            Some("核心没有访问所需资源的权限")
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
            capture_reason(Core::Xray, "stderr", FailingReader(true)),
            Some("读取核心输出失败，诊断可能不完整")
        );
        assert!(
            !logging::recent()
                .iter()
                .any(|line| line.contains("PRIVATE_PIPE_ERROR"))
        );
    }
}
