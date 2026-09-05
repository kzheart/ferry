//! Cross-platform read-only SQLite opens.
//!
//! Windows `fs::canonicalize` yields `\\?\C:\...`. Passing that through a
//! `file:` URI (`file:\\?\C:\...?mode=ro`) is rejected by SQLite and can hang
//! the opener. Open by path after stripping the verbatim prefix instead.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use super::paths::strip_verbatim_prefix;

/// 只读打开，包含 WAL；不加写锁、不改库文件。
pub fn open_readonly(path: &Path) -> rusqlite::Result<Connection> {
    let resolved =
        strip_verbatim_prefix(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()));
    let connection = Connection::open_with_flags(resolved, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_readonly_reads_a_temp_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("probe.sqlite");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch("CREATE TABLE t(x INTEGER); INSERT INTO t VALUES (7);")
                .unwrap();
        }
        let connection = open_readonly(&path).unwrap();
        let value: i64 = connection
            .query_row("SELECT x FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, 7);
    }
}
