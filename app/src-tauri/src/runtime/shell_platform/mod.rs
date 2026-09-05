//! Runtime shell platform boundary.
//!
//! Approval and output handling stay platform-neutral in `bash`; command
//! syntax, inherited environment, and process-tree cleanup live here.

use std::process::Command;

#[cfg(unix)]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(target_os = "windows")]
use windows as imp;

pub(super) fn shell() -> (&'static str, &'static str) {
    imp::shell()
}

pub(super) fn inherit_env_keys() -> &'static [&'static str] {
    imp::inherit_env_keys()
}

pub(super) fn configure_command(command: &mut Command) {
    imp::configure_command(command);
}

pub(super) fn kill_process_tree(pid: u32) {
    imp::kill_process_tree(pid);
}

pub(super) fn needs_explicit_approval(lowercase_command: &str) -> bool {
    imp::needs_explicit_approval(lowercase_command)
}
