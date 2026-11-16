//! Times and periods as the chats show them.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime};
use tbs_core::format::{parse_date_input, parse_datetime};

const WEEKDAYS: [&str; 7] = ["пн", "вт", "ср", "чт", "пт", "сб", "вс"];
const MONTHS: [&str; 12] = ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];

/// The server sends UTC with its zone. A time without one is taken as local.
pub fn local(raw: &str) -> Option<NaiveDateTime> {
    match DateTime::parse_from_rfc3339(raw.trim()) {
        Ok(moment) => Some(moment.with_timezone(&Local).naive_local()),
        Err(_) => parse_datetime(raw),
    }
}

pub fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

pub fn clock(raw: Option<&str>) -> String {
    raw.and_then(local).map(|moment| moment.format("%H:%M").to_string()).unwrap_or_default()
}

/// The time beside a chat: the hour today, the weekday within a week, the date before that.
pub fn list_time(raw: Option<&str>, now: NaiveDateTime) -> String {
    let Some(moment) = raw.and_then(local) else { return String::new() };
    if moment.date() == now.date() {
        return moment.format("%H:%M").to_string();
    }
    if now - moment < Duration::days(7) {
        return WEEKDAYS[moment.weekday().num_days_from_monday() as usize].to_owned();
    }
    moment.format("%d.%m").to_string()
}

/// The line between the messages of two days.
pub fn day_label(day: NaiveDate, today: NaiveDate) -> String {
    if day == today {
        return "Сегодня".to_owned();
    }
    if Some(day) == today.pred_opt() {
        return "Вчера".to_owned();
    }
    let month = MONTHS[day.month0() as usize];
    if day.year() == today.year() { format!("{} {month}", day.day()) } else { format!("{} {month} {} г.", day.day(), day.year()) }
}

pub fn size_label(bytes: Option<i64>) -> String {
    let Some(bytes) = bytes.filter(|bytes| *bytes > 0) else { return String::new() };
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let (mut value, mut unit) = (bytes as f64, 0);
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{value:.0} {}", UNITS[unit]) } else { format!("{value:.1} {}", UNITS[unit]) }
}

/// Two letters for a picture nobody sent.
pub fn initials(name: &str) -> String {
    let mut words = name.split_whitespace();
    match (words.next(), words.next()) {
        (Some(first), Some(second)) => first.chars().take(1).chain(second.chars().take(1)).collect::<String>().to_uppercase(),
        (Some(only), None) => only.chars().take(2).collect::<String>().to_uppercase(),
        _ => "?".to_owned(),
    }
}

/// Whole days, both ends included; an open end reaches as far as there is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Range {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
}

impl Range {
    pub fn is_set(&self) -> bool {
        self.from.is_some() || self.to.is_some()
    }

    /// A message without a date belongs to every period.
    pub fn holds(&self, moment: Option<NaiveDateTime>) -> bool {
        let Some(day) = moment.map(|moment| moment.date()) else { return true };
        self.from.is_none_or(|from| day >= from) && self.to.is_none_or(|to| day <= to)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Today,
    Yesterday,
    Last7,
    Last30,
    ThisMonth,
    PrevMonth,
}

impl Preset {
    pub const ALL: [Preset; 6] = [Preset::Today, Preset::Yesterday, Preset::Last7, Preset::Last30, Preset::ThisMonth, Preset::PrevMonth];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Today => "Сегодня",
            Preset::Yesterday => "Вчера",
            Preset::Last7 => "Последние 7 дней",
            Preset::Last30 => "Последние 30 дней",
            Preset::ThisMonth => "Этот месяц",
            Preset::PrevMonth => "Прошлый месяц",
        }
    }

    pub fn range(self, today: NaiveDate) -> Range {
        let back = |days: i64| today - Duration::days(days);
        let first = today.with_day(1).unwrap_or(today);
        let (from, to) = match self {
            Preset::Today => (today, today),
            Preset::Yesterday => (back(1), back(1)),
            Preset::Last7 => (back(6), today),
            Preset::Last30 => (back(29), today),
            Preset::ThisMonth => (first, today),
            Preset::PrevMonth => {
                let last = first.pred_opt().unwrap_or(first);
                (last.with_day(1).unwrap_or(last), last)
            }
        };
        Range { from: Some(from), to: Some(to) }
    }
}

/// What the period button says: a preset by its name, anything else by its days.
pub fn range_label(range: Range, today: NaiveDate) -> String {
    if !range.is_set() {
        return String::new();
    }
    if let Some(preset) = Preset::ALL.into_iter().find(|preset| preset.range(today) == range) {
        return preset.label().to_owned();
    }
    let day = |day: Option<NaiveDate>| day.map_or_else(|| "…".to_owned(), |day| day.format("%d.%m").to_string());
    format!("{} — {}", day(range.from), day(range.to))
}

pub fn typed(day: Option<NaiveDate>) -> String {
    day.map(|day| day.format("%d.%m.%Y").to_string()).unwrap_or_default()
}

/// A day typed by hand. An empty box is an open end, not a mistake.
pub fn read_day(value: &str) -> Result<Option<NaiveDate>, ()> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    parse_date_input(value).map(Some).ok_or(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn a_chat_shows_the_hour_then_the_weekday_then_the_date() {
        let now = day(2026, 11, 11).and_hms_opt(18, 0, 0).unwrap();
        assert_eq!(list_time(Some("2026-11-11T09:05:00"), now), "09:05");
        // 9 November 2026 is a Monday.
        assert_eq!(list_time(Some("2026-11-09T23:00:00"), now), "пн");
        assert_eq!(list_time(Some("2026-11-01T10:00:00"), now), "01.11");
        assert_eq!(list_time(Some("not a date"), now), "");
        assert_eq!(list_time(None, now), "");
        assert_eq!(clock(Some("2026-11-11T09:05:00")), "09:05");
    }

    #[test]
    fn a_zoned_time_is_shown_in_local_time() {
        let utc = DateTime::parse_from_rfc3339("2026-11-11T10:00:00Z").unwrap();
        assert_eq!(local("2026-11-11T10:00:00Z"), Some(utc.with_timezone(&Local).naive_local()));
        assert_eq!(local("2026-11-11T12:00:00+02:00"), local("2026-11-11T10:00:00Z"));
    }

    #[test]
    fn days_are_named_as_people_name_them() {
        let today = day(2026, 11, 11);
        assert_eq!(day_label(today, today), "Сегодня");
        assert_eq!(day_label(day(2026, 11, 10), today), "Вчера");
        assert_eq!(day_label(day(2026, 3, 5), today), "5 марта");
        assert_eq!(day_label(day(2025, 12, 31), today), "31 декабря 2025 г.");
    }

    #[test]
    fn presets_cover_the_days_they_name() {
        let today = day(2026, 3, 11);
        assert_eq!(Preset::Today.range(today), Range { from: Some(today), to: Some(today) });
        assert_eq!(Preset::Last7.range(today).from, Some(day(2026, 3, 5)));
        assert_eq!(Preset::Last30.range(today).from, Some(day(2026, 2, 10)));
        assert_eq!(Preset::ThisMonth.range(today), Range { from: Some(day(2026, 3, 1)), to: Some(today) });
        assert_eq!(Preset::PrevMonth.range(today), Range { from: Some(day(2026, 2, 1)), to: Some(day(2026, 2, 28)) });
        assert_eq!(Preset::PrevMonth.range(day(2026, 1, 15)), Range { from: Some(day(2025, 12, 1)), to: Some(day(2025, 12, 31)) });
    }

    #[test]
    fn a_period_is_named_by_its_preset_or_its_days() {
        let today = day(2026, 11, 11);
        assert_eq!(range_label(Range::default(), today), "");
        assert_eq!(range_label(Preset::Yesterday.range(today), today), "Вчера");
        assert_eq!(range_label(Range { from: Some(day(2026, 10, 1)), to: Some(day(2026, 10, 7)) }, today), "01.10 — 07.10");
        assert_eq!(range_label(Range { from: Some(day(2026, 10, 1)), to: None }, today), "01.10 — …");
    }

    #[test]
    fn a_period_holds_its_days_and_messages_without_a_date() {
        let range = Range { from: Some(day(2026, 11, 10)), to: Some(day(2026, 11, 11)) };
        let at = |d: u32, h: u32| day(2026, 11, d).and_hms_opt(h, 0, 0);
        assert!(range.holds(at(10, 0)) && range.holds(at(11, 23)));
        assert!(!range.holds(at(9, 23)) && !range.holds(at(12, 0)));
        assert!(range.holds(None));
        assert!(Range { from: None, to: Some(day(2026, 11, 10)) }.holds(at(1, 0)));
    }

    #[test]
    fn typed_days_read_back() {
        assert_eq!(read_day(" 05.11.2026 "), Ok(Some(day(2026, 11, 5))));
        assert_eq!(read_day(""), Ok(None));
        assert_eq!(read_day("позавчера"), Err(()));
        assert_eq!(typed(Some(day(2026, 11, 5))), "05.11.2026");
    }

    #[test]
    fn sizes_and_initials() {
        assert_eq!(size_label(Some(512)), "512 B");
        assert_eq!(size_label(Some(10_240)), "10.0 KB");
        assert_eq!(size_label(Some(5 * 1024 * 1024 + 1024 * 512)), "5.5 MB");
        assert_eq!(size_label(None), "");
        assert_eq!(initials("касса чеки"), "КЧ");
        assert_eq!(initials("бот"), "БО");
        assert_eq!(initials("  "), "?");
    }
}
