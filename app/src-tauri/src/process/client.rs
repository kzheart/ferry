use crate::process::error::ProcessError;
use crate::process::framing::JsonlWriter;
use std::collections::HashMap;
use std::process::ChildStdin;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

type PendingResult = Result<String, ProcessError>;

#[derive(Clone, Default)]
pub(crate) struct PendingResponses {
    waiters: Arc<Mutex<HashMap<String, mpsc::Sender<PendingResult>>>>,
}

impl PendingResponses {
    pub(crate) fn register(&self, id: &str) -> Result<mpsc::Receiver<PendingResult>, ProcessError> {
        let (sender, receiver) = mpsc::channel();
        let mut waiters = self
            .waiters
            .lock()
            .map_err(|_| ProcessError::Lock("进程 pending 锁损坏".to_owned()))?;
        if waiters.contains_key(id) {
            return Err(ProcessError::Lock("进程请求 ID 重复".to_owned()));
        }
        waiters.insert(id.to_owned(), sender);
        Ok(receiver)
    }

    pub(crate) fn remove(&self, id: &str) {
        self.waiters.lock().ok().and_then(|mut map| map.remove(id));
    }

    pub(crate) fn complete(&self, id: &str, line: String) {
        if let Some(sender) = self.waiters.lock().ok().and_then(|mut map| map.remove(id)) {
            let _ = sender.send(Ok(line));
        }
    }

    pub(crate) fn fail_all(&self, error: ProcessError) {
        if let Ok(mut waiters) = self.waiters.lock() {
            for (_, sender) in waiters.drain() {
                let _ = sender.send(Err(error.clone()));
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct JsonlProcessClient {
    label: &'static str,
    writer: JsonlWriter,
    pending: PendingResponses,
}

impl JsonlProcessClient {
    pub(crate) fn new(label: &'static str, stdin: ChildStdin) -> Self {
        Self {
            label,
            writer: JsonlWriter::new(label, stdin),
            pending: PendingResponses::default(),
        }
    }

    pub(crate) fn writer(&self) -> JsonlWriter {
        self.writer.clone()
    }

    pub(crate) fn pending(&self) -> PendingResponses {
        self.pending.clone()
    }

    pub(crate) fn request(
        &self,
        id: &str,
        line: &str,
        timeout: Duration,
    ) -> Result<String, ProcessError> {
        let receiver = self.pending.register(id)?;
        if let Err(error) = self.writer.write_line(line) {
            self.pending.remove(id);
            return Err(error);
        }
        receiver.recv_timeout(timeout).map_err(|error| {
            self.pending.remove(id);
            ProcessError::Timeout(format!("等待 {} 响应失败: {error}", self.label))
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::PendingResponses;
    use crate::process::error::ProcessError;

    #[test]
    fn pending_responses_support_out_of_order_completion() {
        let pending = PendingResponses::default();
        let first = pending.register("first").unwrap();
        let second = pending.register("second").unwrap();
        pending.complete("second", "two".to_owned());
        pending.complete("first", "one".to_owned());
        assert_eq!(first.recv().unwrap().unwrap(), "one");
        assert_eq!(second.recv().unwrap().unwrap(), "two");
    }

    #[test]
    fn process_exit_releases_every_waiter() {
        let pending = PendingResponses::default();
        let first = pending.register("first").unwrap();
        let second = pending.register("second").unwrap();
        pending.fail_all(ProcessError::Exited("退出".to_owned()));
        assert_eq!(
            first.recv().unwrap().unwrap_err(),
            ProcessError::Exited("退出".to_owned()),
        );
        assert_eq!(
            second.recv().unwrap().unwrap_err(),
            ProcessError::Exited("退出".to_owned()),
        );
    }

    #[test]
    fn duplicate_request_id_does_not_replace_existing_waiter() {
        let pending = PendingResponses::default();
        let first = pending.register("same").unwrap();
        assert!(pending.register("same").is_err());
        pending.complete("same", "done".to_owned());
        assert_eq!(first.recv().unwrap().unwrap(), "done");
    }
}
