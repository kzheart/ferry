use crate::process::error::ProcessError;
use std::io::Write;
use std::process::ChildStdin;
use std::sync::{Arc, Mutex};

pub(crate) const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct JsonlWriter {
    label: &'static str,
    stdin: Arc<Mutex<ChildStdin>>,
}

impl JsonlWriter {
    pub(crate) fn new(label: &'static str, stdin: ChildStdin) -> Self {
        Self {
            label,
            stdin: Arc::new(Mutex::new(stdin)),
        }
    }

    pub(crate) fn write_line(&self, line: &str) -> Result<(), ProcessError> {
        if line.len() > MAX_FRAME_BYTES || line.contains(['\n', '\r']) {
            return Err(ProcessError::InvalidFrame(format!(
                "{} JSONL framing 非法",
                self.label,
            )));
        }
        let mut writer = self
            .stdin
            .lock()
            .map_err(|_| ProcessError::Lock(format!("{} stdin 锁损坏", self.label)))?;
        writer
            .write_all(line.as_bytes())
            .and_then(|_| writer.write_all(b"\n"))
            .and_then(|_| writer.flush())
            .map_err(|error| ProcessError::Write(format!("写入 {} 失败: {error}", self.label)))
    }
}
