use std::path::Path;
use std::process::Command;

pub(super) fn reveal_path(path: &Path) -> Result<(), String> {
    let output = Command::new("open")
        .arg("-R")
        .arg(path)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(())
}
