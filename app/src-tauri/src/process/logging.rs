//! 诊断日志:宿主与两个 sidecar 的日志统一落在 `~/.ferry/logs/`。
//!
//! 宿主进程写 `host.log`;engine/runtime 的 stderr 分别接到 `engine.log`
//! 与 `runtime.log`。日志失败必须静默——诊断设施不能反过来影响主流程。

use std::fs::{create_dir_all, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

fn log_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let dir = PathBuf::from(home).join(".ferry").join("logs");
    create_dir_all(&dir).ok()?;
    Some(dir)
}

/// UTC 时间戳 `YYYY-MM-DDTHH:MM:SS.mmmZ`;不引依赖,手工从 epoch 换算。
fn format_timestamp(secs: u64, millis: u32) -> String {
    let days = (secs / 86_400) as i64;
    let (hour, minute, second) = (secs / 3600 % 24, secs / 60 % 60, secs % 60);
    // civil_from_days(Howard Hinnant 算法),epoch 为 1970-01-01
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format_timestamp(now.as_secs(), now.subsec_millis())
}

/// 宿主日志:`host.log` 追加一行 `时间 [组件] 内容`。
pub(crate) fn host_log(component: &str, message: &str) {
    let Some(dir) = log_dir() else { return };
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("host.log"))
    else {
        return;
    };
    let _ = writeln!(file, "{} [{component}] {message}", timestamp());
}

/// sidecar 的 stderr 目的地:打不开日志文件时退回丢弃,行为与之前一致。
pub(crate) fn sidecar_stderr(file_name: &str) -> Stdio {
    let target: Option<File> = log_dir().and_then(|dir| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(file_name))
            .ok()
    });
    match target {
        Some(file) => Stdio::from(file),
        None => Stdio::null(),
    }
}

#[cfg(test)]
mod tests {
    use super::format_timestamp;

    #[test]
    fn timestamp_shape_is_iso_utc() {
        assert_eq!(format_timestamp(0, 0), "1970-01-01T00:00:00.000Z");
        // 闰日
        assert_eq!(
            format_timestamp(1_709_214_307, 12),
            "2024-02-29T13:45:07.012Z"
        );
        assert_eq!(
            format_timestamp(1_787_469_154, 999),
            "2026-08-23T07:12:34.999Z"
        );
    }
}
