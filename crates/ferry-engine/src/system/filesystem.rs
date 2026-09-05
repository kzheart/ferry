//! Platform-specific filesystem inspection.

use std::path::Path;

/// SQLite WAL is unsafe on common network filesystems. This probe deliberately
/// lives outside adapters so macOS, Linux, and Windows do not grow command
/// branches inside format-specific code.
pub fn is_network_filesystem(path: &Path) -> bool {
    imp::is_network_filesystem(path)
}

#[cfg(any(unix, test))]
fn filesystem_name_is_network(filesystem: &str) -> bool {
    ["nfs", "smb", "cifs", "afp", "webdav", "sshfs"]
        .iter()
        .any(|marker| filesystem.to_ascii_lowercase().contains(marker))
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{filesystem_name_is_network, Path};
    use std::time::Duration;

    pub(super) fn is_network_filesystem(path: &Path) -> bool {
        let command = vec![
            "/usr/bin/stat".into(),
            "-f".into(),
            "%T".into(),
            path.to_string_lossy().into_owned(),
        ];
        crate::system::probes::run(&command, None, Duration::from_secs(3), None)
            .map(|result| filesystem_name_is_network(result.stdout.trim()))
            .unwrap_or(false)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::{filesystem_name_is_network, Path};
    use std::time::Duration;

    pub(super) fn is_network_filesystem(path: &Path) -> bool {
        let command = vec![
            "stat".into(),
            "-f".into(),
            "-c".into(),
            "%T".into(),
            path.to_string_lossy().into_owned(),
        ];
        crate::system::probes::run(&command, None, Duration::from_secs(3), None)
            .map(|result| filesystem_name_is_network(result.stdout.trim()))
            .unwrap_or(false)
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::Path;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Prefix};
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;

    const DRIVE_REMOTE: u32 = 4;

    pub(super) fn is_network_filesystem(path: &Path) -> bool {
        let Some(root) = volume_root(path) else {
            return false;
        };
        let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe { GetDriveTypeW(wide.as_ptr()) == DRIVE_REMOTE }
    }

    fn volume_root(path: &Path) -> Option<std::path::PathBuf> {
        let prefix = match path.components().next()? {
            Component::Prefix(prefix) => prefix.kind(),
            _ => return None,
        };
        let root = match prefix {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                format!("{}:\\", letter as char)
            }
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => format!(
                r"\\{}\{}\",
                server.to_string_lossy(),
                share.to_string_lossy()
            ),
            _ => return None,
        };
        Some(root.into())
    }

    #[cfg(test)]
    mod tests {
        use super::volume_root;
        use std::path::{Path, PathBuf};

        #[test]
        fn extracts_windows_volume_roots() {
            assert_eq!(
                volume_root(Path::new(r"C:\work\x")),
                Some(PathBuf::from(r"C:\"))
            );
            assert_eq!(
                volume_root(Path::new(r"\\server\share\work")),
                Some(PathBuf::from(r"\\server\share\"))
            );
        }
    }
}

#[cfg(not(any(unix, target_os = "windows")))]
mod imp {
    use super::Path;

    pub(super) fn is_network_filesystem(_path: &Path) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::filesystem_name_is_network;

    #[test]
    fn recognizes_network_filesystem_names() {
        assert!(filesystem_name_is_network("smbfs"));
        assert!(filesystem_name_is_network("NFS"));
        assert!(!filesystem_name_is_network("apfs"));
        assert!(!filesystem_name_is_network("ntfs"));
    }
}
