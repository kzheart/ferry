use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

pub(super) fn shell() -> (&'static str, &'static str) {
    ("sh", "-c")
}

pub(super) fn inherit_env_keys() -> &'static [&'static str] {
    &["PATH", "HOME", "LANG", "TERM"]
}

pub(super) fn configure_command(command: &mut Command) {
    // A separate process group lets timeout cleanup include grandchildren.
    command.process_group(0);
}

pub(super) fn kill_process_tree(pid: u32) {
    // A negative pid targets the process group. Use the shell builtin to avoid
    // another unsafe libc call at this boundary.
    let _ = Command::new("sh")
        .arg("-c")
        .arg(format!("kill -KILL -{pid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub(super) fn needs_explicit_approval(_command: &str) -> bool {
    false
}
