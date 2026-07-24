use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessError {
    InvalidFrame(String),
    Lock(String),
    Write(String),
    Timeout(String),
    Exited(String),
}

impl ProcessError {
    pub(crate) fn invalidates_process(&self) -> bool {
        matches!(self, Self::Lock(_) | Self::Write(_) | Self::Exited(_))
    }
}

impl fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidFrame(message)
            | Self::Lock(message)
            | Self::Write(message)
            | Self::Timeout(message)
            | Self::Exited(message) => message,
        };
        formatter.write_str(message)
    }
}

#[cfg(test)]
mod tests {
    use super::ProcessError;

    #[test]
    fn timeout_does_not_invalidate_other_inflight_requests() {
        assert!(!ProcessError::Timeout("timeout".to_owned()).invalidates_process());
        assert!(!ProcessError::InvalidFrame("frame".to_owned()).invalidates_process());
        assert!(ProcessError::Exited("exit".to_owned()).invalidates_process());
        assert!(ProcessError::Write("write".to_owned()).invalidates_process());
    }
}
