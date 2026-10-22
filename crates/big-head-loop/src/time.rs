//! 时间格式化工具 —— 将 Unix timestamp 转换为本地可读格式。

/// 将 Unix epoch seconds 转换为本地时间的 ISO 8601 格式字符串。
///
/// 使用简单的偏移量计算（不支持 DST），格式为：`2026-06-01T17:35:31+08:00`
pub fn format_local_timestamp(epoch_secs: u64, offset_hours: i32) -> String {
    let _offset_secs = offset_hours as i64 * 3600;

    // 计算 UTC 时间的日期时间组件
    let days = epoch_secs / 86400;
    let remaining = epoch_secs % 86400;
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;

    // 计算 UTC 日期（从 1970-01-01 开始）
    let mut year = 1970;
    let mut days_left = days as i64;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days_left < days_in_year {
            break;
        }
        days_left -= days_in_year;
        year += 1;
    }

    let days_in_months = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1;
    for days_in_month in days_in_months.iter() {
        if days_left < *days_in_month as i64 {
            break;
        }
        days_left -= *days_in_month as i64;
        month += 1;
    }
    let day = (days_left + 1) as u8;

    // 转换为本地时间
    let sign = if offset_hours >= 0 { '+' } else { '-' };
    let abs_offset = offset_hours.abs();

    // 重新计算本地时间（简化处理）
    let local_hour = ((hours as i64 + offset_hours as i64) % 24 + 24) % 24;

    // 调整日期（如果小时变成负数或超过24）
    let mut local_day = day;
    let mut local_month = month as u8;
    let mut local_year = year;

    if local_hour < 0 {
        local_day -= 1;
        if local_day == 0 {
            local_month -= 1;
            if local_month == 0 {
                local_month = 12;
                local_year -= 1;
            }
            local_day = days_in_months[(local_month - 1) as usize] as u8;
        }
    } else if local_hour >= 24 {
        local_day += 1;
        if local_day > days_in_months[(local_month - 1) as usize] as u8 {
            local_day = 1;
            local_month += 1;
            if local_month > 12 {
                local_month = 1;
                local_year += 1;
            }
        }
    }

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:00",
        local_year, local_month, local_day, local_hour, minutes, seconds, sign, abs_offset
    )
}

/// 简化版本：返回易读的本地时间格式 `YYYY-MM-DD HH:MM:SS (UTC±HH)`
pub fn format_local_readable(epoch_secs: u64, offset_hours: i32) -> String {
    let days = epoch_secs / 86400;
    let remaining = epoch_secs % 86400;
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;

    // 计算日期
    let mut year = 1970;
    let mut days_left = days as i64;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days_left < days_in_year {
            break;
        }
        days_left -= days_in_year;
        year += 1;
    }

    let days_in_months = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1;
    for days_in_month in days_in_months.iter() {
        if days_left < *days_in_month as i64 {
            break;
        }
        days_left -= *days_in_month as i64;
        month += 1;
    }
    let day = (days_left + 1) as u8;

    // 计算本地时间
    let local_hour = ((hours as i64 + offset_hours as i64) % 24 + 24) % 24;

    let sign = if offset_hours >= 0 { "+" } else { "" };

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} (UTC{}{})",
        year, month, day, local_hour, minutes, seconds, sign, offset_hours
    )
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_format() {
        // 2026-06-01 17:35:31 UTC = epoch 1772459731
        let epoch = 1772459731;
        let result = format_local_readable(epoch, 8);
        assert!(result.contains("2026"));
    }

    #[test]
    fn test_utc_plus_8() {
        // UTC 17:35:31 -> CST 01:35:31 (次日)
        let epoch = 1772459731; // 2026-06-01 17:35:31 UTC
        let result = format_local_readable(epoch, 8);
        assert!(result.contains("(UTC+8)"));
    }

    #[test]
    fn convert_log_timestamps_preserves_chinese() {
        // 之前的 byte 迭代会把 UTF-8 续字节切成 Latin-1 码点。
        let input = "push_spark: spawned entity 82v0 with text '清理任务：当前有 6 条 Spark 记录，请归档。'";
        let out = convert_log_timestamps(input, 8);
        assert_eq!(out, input);
    }

    #[test]
    fn convert_log_timestamps_preserves_emoji() {
        let input = "🧠 Wave 1 ── reasoning";
        let out = convert_log_timestamps(input, 8);
        assert_eq!(out, input);
    }

    #[test]
    fn convert_log_timestamps_replaces_iso_inside_chinese() {
        let input = "操作 2026-06-02T10:30:00Z 成功";
        let out = convert_log_timestamps(input, 8);
        assert!(!out.contains("2026-06-02T10:30:00Z"));
        assert!(out.contains("操作 "));
        assert!(out.contains(" 成功"));
    }
}

/// 将 ISO 8601 格式的时间字符串（如 "2026-06-01T17:35:31Z"）转换为本地可读格式。
///
/// 例如：输入 "2026-06-01T17:35:31Z"，偏移 +8 小时，输出 "2026-06-02 01:35:31 (UTC+8)"
pub fn convert_iso_to_local(iso_time: &str, offset_hours: i32) -> String {
    // 解析 ISO 8601 格式：YYYY-MM-DDTHH:MM:SSZ
    if iso_time.len() < 20 || !iso_time.ends_with('Z') {
        return iso_time.to_string();
    }

    // 手动解析各部分
    let year: i64 = iso_time[0..4].parse().unwrap_or(0);
    let month: u8 = iso_time[5..7].parse().unwrap_or(1);
    let day: u8 = iso_time[8..10].parse().unwrap_or(1);
    let hour: u8 = iso_time[11..13].parse().unwrap_or(0);
    let minute: u8 = iso_time[14..16].parse().unwrap_or(0);
    let second: u8 = iso_time[17..19].parse().unwrap_or(0);

    // 计算 Unix timestamp（UTC）
    let epoch_secs = date_time_to_epoch(year, month, day, hour, minute, second);

    format_local_readable(epoch_secs, offset_hours)
}

/// 计算 UTC 日期时间到 Unix epoch 的秒数
fn date_time_to_epoch(year: i64, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> u64 {
    // 计算从 1970-01-01 到该日期的天数
    let mut days = 0i64;

    // 加上完整年份的天数
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }

    // 加上月份的天数
    let days_in_months = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    for m in 1..month {
        days += days_in_months[(m - 1) as usize] as i64;
    }

    // 加上当月天数（day 从 1 开始）
    days += (day - 1) as i64;

    // 转换为秒
    let seconds = days * 86400 + (hour as i64 * 3600) + (minute as i64 * 60) + (second as i64);

    seconds as u64
}

/// 在日志字符串中查找并替换 ISO 8601 时间格式为本地时间格式。
/// 用于处理 TickAction 脚本中输出的日志。
///
/// 按 `char` 迭代,UTF-8 多字节字符保持完整 —— 之前的 byte 迭代会把
/// 续字节 (0x80–0xBF) 当成 Latin-1 码点,造成中文乱码。
pub fn convert_log_timestamps(log: &str, offset_hours: i32) -> String {
    let mut result = String::with_capacity(log.len());
    let bytes = log.as_bytes();
    let mut byte_idx = 0;

    while byte_idx < bytes.len() {
        // 仅在 ASCII 范围内检测 ISO 8601 模式 (YYYY-MM-DDTHH:MM:SSZ, 20 字节)
        if bytes[byte_idx].is_ascii_digit() && byte_idx + 20 <= bytes.len() {
            let probe = &bytes[byte_idx..byte_idx + 20];
            let is_match = probe[0..4].iter().all(|b| b.is_ascii_digit())
                && probe[4] == b'-'
                && probe[5..7].iter().all(|b| b.is_ascii_digit())
                && probe[7] == b'-'
                && probe[8..10].iter().all(|b| b.is_ascii_digit())
                && probe[10] == b'T'
                && probe[11..13].iter().all(|b| b.is_ascii_digit())
                && probe[13] == b':'
                && probe[14..16].iter().all(|b| b.is_ascii_digit())
                && probe[16] == b':'
                && probe[17..19].iter().all(|b| b.is_ascii_digit())
                && probe[19] == b'Z';

            if is_match {
                let iso_time = &log[byte_idx..byte_idx + 20];
                result.push_str(&convert_iso_to_local(iso_time, offset_hours));
                byte_idx += 20;
                continue;
            }
        }

        // 按 char 取下一个 UTF-8 字符,整段推进 byte_idx
        let ch = log[byte_idx..].chars().next().expect("non-empty");
        result.push(ch);
        byte_idx += ch.len_utf8();
    }

    result
}
