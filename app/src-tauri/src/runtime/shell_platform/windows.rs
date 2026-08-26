use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(super) fn shell() -> (&'static str, &'static str) {
    ("cmd.exe", "/C")
}

pub(super) fn inherit_env_keys() -> &'static [&'static str] {
    &[
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "SystemRoot",
        "SYSTEMDRIVE",
        "COMSPEC",
        "USERNAME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "HOME",
        "LANG",
        "TERM",
    ]
}

pub(super) fn configure_command(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

pub(super) fn kill_process_tree(pid: u32) {
    let _ = Command::new("taskkill.exe")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub(super) fn needs_explicit_approval(command: &str) -> bool {
    const MARKERS: [&str; 8] = [
        "del ",
        "erase ",
        "rmdir ",
        "rd /s",
        "format ",
        "diskpart",
        "remove-item ",
        "clear-recyclebin",
    ];
    MARKERS.iter().any(|marker| command.contains(marker))
        || (command.contains("curl ") || command.contains("wget "))
            && (command.contains("| powershell") || command.contains("|pwsh"))
}
