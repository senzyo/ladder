use std::io::Read;
use std::thread::{self, JoinHandle};

pub(crate) struct CoreOutput {
    stdout: JoinHandle<Option<&'static str>>,
    stderr: JoinHandle<Option<&'static str>>,
}

impl CoreOutput {
    pub(crate) fn capture(
        stdout: impl Read + Send + 'static,
        stderr: impl Read + Send + 'static,
    ) -> Self {
        Self {
            stdout: thread::spawn(move || read_reason(stdout)),
            stderr: thread::spawn(move || read_reason(stderr)),
        }
    }

    // 核心退出后等待两条管道读完, 再生成诊断。
    pub(crate) fn finish(self) -> Option<&'static str> {
        let stdout = self.stdout.join().ok().flatten();
        let stderr = self.stderr.join().ok().flatten();
        stderr.or(stdout)
    }
}

fn read_reason(mut pipe: impl Read) -> Option<&'static str> {
    let mut buffer = [0; 4096];
    let mut pending = Vec::with_capacity(buffer.len() + 32);
    let mut reason = None;
    while let Ok(size) = pipe.read(&mut buffer) {
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
        // 当前最长匹配短语为 22 字节。只保留跨次读取所需的尾部, 不积累原始日志。
        let keep = pending.len().min(32);
        let start = pending.len() - keep;
        pending.copy_within(start.., 0);
        pending.truncate(keep);
    }
    reason
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::Duration;

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
            let expected = read_reason(Cursor::new(phrase.as_bytes()));
            assert!(expected.is_some());
            for split in 1..phrase.len() {
                let pipe = Cursor::new(&phrase.as_bytes()[..split])
                    .chain(Cursor::new(&phrase.as_bytes()[split..]));
                assert_eq!(read_reason(pipe), expected, "split {split} of {phrase}");
            }
        }
        let mut text = vec![b'x'; 4090];
        text.extend_from_slice(b"ACCESS IS DENIED: PRIVATE_FIXTURE_VALUE");
        assert_eq!(
            read_reason(Cursor::new(text)),
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
}
