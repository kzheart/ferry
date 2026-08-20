//! Cursor 写入路径要用到的时间与时区表达。
//!
//! Cursor 的三处时间形态互不相同，只能各写各的：
//! - bubble 的 `createdAt` 是 ISO-8601 毫秒 UTC 字符串；
//! - header 行与 `conversationState` 的 f26 是 epoch 毫秒整数；
//! - 上下文层 user 消息里的 `<timestamp>` 包装是**人读**的本地时间文案。
//!
//! 标准库没有本地时区，unix 上走 `libc::localtime_r`，其他平台退化成 UTC——退化
//! 只影响那句人读文案与 f27，两者 Cursor 都不校验。

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const WEEKDAYS: [&str; 7] = [
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
];

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// 当前 epoch 毫秒。
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

/// Howard Hinnant 的 `civil_from_days`：纪元天数 → 民用日期。
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (year + i64::from(month <= 2), month, day)
}

/// epoch 毫秒 → `2026-05-28T11:22:22.424Z`（bubble 的 `createdAt` 形态）。
pub fn iso_utc_millis(epoch_ms: i64) -> String {
    let seconds = epoch_ms.div_euclid(1000);
    let millis = epoch_ms.rem_euclid(1000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// 本机时区相对 UTC 的秒偏移。
fn local_offset_seconds(epoch_seconds: i64) -> i64 {
    #[cfg(unix)]
    {
        // SAFETY：`localtime_r` 把结果写进调用方提供的 `tm`，不碰全局状态。
        unsafe {
            let time = epoch_seconds as libc::time_t;
            let mut parts: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&time, &mut parts).is_null() {
                return 0;
            }
            parts.tm_gmtoff as i64
        }
    }
    #[cfg(not(unix))]
    {
        let _ = epoch_seconds;
        0
    }
}

/// `<timestamp>` 包装里的人读文案：`Wednesday, Aug 19, 2026, 11:10 PM (UTC+8)`。
pub fn local_timestamp_label(epoch_ms: i64) -> String {
    let offset = local_offset_seconds(epoch_ms.div_euclid(1000));
    timestamp_label(epoch_ms, offset)
}

/// [`local_timestamp_label`] 的纯函数内核；偏移显式传入，便于逐字节断言。
fn timestamp_label(epoch_ms: i64, offset: i64) -> String {
    let utc_seconds = epoch_ms.div_euclid(1000);
    let local_seconds = utc_seconds + offset;
    let days = local_seconds.div_euclid(86_400);
    let rest = local_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let weekday = WEEKDAYS[days.rem_euclid(7) as usize];
    let month_name = MONTHS[(month - 1).clamp(0, 11) as usize];
    let hour24 = rest / 3600;
    let hour12 = match hour24 % 12 {
        0 => 12,
        other => other,
    };
    let meridiem = if hour24 < 12 { "AM" } else { "PM" };
    let minute = (rest % 3600) / 60;
    let hours = offset / 3600;
    let sign = if hours < 0 { '-' } else { '+' };
    let remainder = (offset.abs() % 3600) / 60;
    let zone = if remainder == 0 {
        format!("UTC{sign}{}", hours.abs())
    } else {
        format!("UTC{sign}{}:{remainder:02}", hours.abs())
    };
    format!("{weekday}, {month_name} {day}, {year}, {hour12}:{minute:02} {meridiem} ({zone})")
}

/// 本机 IANA 时区名；解析不出来时退回 `UTC`。
///
/// `TZ` 优先（容器与 CI 常只设它），否则读 `/etc/localtime` 这条指向 zoneinfo 的
/// 符号链接。取不到就是 `UTC`——f27 只是给模型看的环境信息，Cursor 不校验。
pub fn timezone_name() -> String {
    if let Ok(explicit) = std::env::var("TZ") {
        let trimmed = explicit.trim().trim_start_matches(':');
        if !trimmed.is_empty() && !trimmed.contains('\0') {
            return trimmed.to_string();
        }
    }
    if let Ok(target) = std::fs::read_link(Path::new("/etc/localtime")) {
        let text = target.to_string_lossy().into_owned();
        if let Some(index) = text.find("zoneinfo/") {
            let name = &text[index + "zoneinfo/".len()..];
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    "UTC".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_millis_matches_the_native_bubble_shape() {
        assert_eq!(iso_utc_millis(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            iso_utc_millis(1_780_312_942_424),
            "2026-06-01T11:22:22.424Z"
        );
        // 负毫秒（纪元前）不能 panic。
        assert_eq!(iso_utc_millis(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn the_timestamp_label_reads_like_the_native_wrapper() {
        // 偏移显式给定，用例不随运行机器的时区漂移。
        assert_eq!(
            timestamp_label(1_780_312_942_424, 8 * 3600),
            "Monday, Jun 1, 2026, 7:22 PM (UTC+8)"
        );
        assert_eq!(
            timestamp_label(1_780_312_942_424, -5 * 3600),
            "Monday, Jun 1, 2026, 6:22 AM (UTC-5)"
        );
        // 半小时时区与午夜/正午的 12 小时制边界。
        assert!(timestamp_label(0, 5 * 3600 + 1800).contains("(UTC+5:30)"));
        assert!(timestamp_label(0, 0).contains("12:00 AM"));
        assert!(timestamp_label(12 * 3_600_000, 0).contains("12:00 PM"));
    }

    #[test]
    fn weekdays_line_up_with_the_epoch() {
        // 1970-01-01 是星期四；数组下标 0 必须是 Thursday。
        assert!(timestamp_label(0, 0).starts_with("Thursday, Jan 1, 1970"));
        assert!(local_timestamp_label(now_ms()).contains(" (UTC"));
    }

    #[test]
    fn the_timezone_falls_back_to_utc_instead_of_failing() {
        let name = timezone_name();
        assert!(!name.is_empty());
        assert!(!name.contains('\0'));
    }
}
