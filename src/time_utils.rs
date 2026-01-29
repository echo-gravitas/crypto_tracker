use chrono::{DateTime, Datelike, Local, Timelike, Weekday};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn sleep_until_interval_boundary(interval_secs: u64) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0));
    let now_ms = now.as_secs() * 1000 + u64::from(now.subsec_millis());
    let interval_ms = interval_secs * 1000;
    if interval_ms == 0 {
        return;
    }
    let remainder = now_ms % interval_ms;
    if remainder == 0 {
        return;
    }
    let sleep_ms = interval_ms - remainder;
    std::thread::sleep(Duration::from_millis(sleep_ms));
}

pub fn format_timestamp_de(time: DateTime<Local>) -> String {
    let weekday = match time.weekday() {
        Weekday::Mon => "Montag",
        Weekday::Tue => "Dienstag",
        Weekday::Wed => "Mittwoch",
        Weekday::Thu => "Donnerstag",
        Weekday::Fri => "Freitag",
        Weekday::Sat => "Samstag",
        Weekday::Sun => "Sonntag",
    };
    format!(
        "{}, {:02}.{:02}.{:04} {:02}:{:02} Uhr",
        weekday,
        time.day(),
        time.month(),
        time.year(),
        time.hour(),
        time.minute()
    )
}

pub fn format_timestamp_short(time: DateTime<Local>) -> String {
    time.format("%d.%m.%Y %H:%M").to_string()
}
