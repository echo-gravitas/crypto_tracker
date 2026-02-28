use chrono::{DateTime, Datelike, Local, Timelike, Weekday};
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
