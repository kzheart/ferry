use std::process::Child;
use std::sync::Mutex;

pub(crate) struct ManagedProcess<Client: Clone> {
    generation: u64,
    child: Child,
    client: Client,
}

impl<Client: Clone> ManagedProcess<Client> {
    pub(crate) fn new(generation: u64, child: Child, client: Client) -> Self {
        Self {
            generation,
            child,
            client,
        }
    }

    fn has_exited(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_some()
    }
}

impl<Client: Clone> Drop for ManagedProcess<Client> {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) struct ProcessSupervisor<Client: Clone> {
    label: &'static str,
    slot: Mutex<Option<ManagedProcess<Client>>>,
}

impl<Client: Clone> ProcessSupervisor<Client> {
    pub(crate) fn new(label: &'static str) -> Self {
        Self {
            label,
            slot: Mutex::new(None),
        }
    }

    pub(crate) fn ensure(
        &self,
        spawn: impl FnOnce() -> Result<ManagedProcess<Client>, String>,
    ) -> Result<Client, String> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|_| format!("{}状态锁损坏", self.label))?;
        if slot.as_mut().is_some_and(ManagedProcess::has_exited) {
            *slot = None;
        }
        if slot.is_none() {
            *slot = Some(spawn()?);
        }
        Ok(slot.as_ref().expect("process just ensured").client.clone())
    }

    pub(crate) fn invalidate(&self, generation: u64) {
        if let Ok(mut slot) = self.slot.lock() {
            if slot
                .as_ref()
                .is_some_and(|process| process.generation == generation)
            {
                *slot = None;
            }
        }
    }
}
