//! Display formatting. Must produce the same text as the React client, since
//! people compare the two side by side during the switch.

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};

pub const EMPTY: &str = "—";
const NBSP: char = '\u{a0}';
pub const WEEKDAYS: [&str; 7] = ["вс", "пн", "вт", "ср", "чт", "пт", "сб"];

/// The server's timestamps are naive local time, sometimes with fractions or
/// a `Z`. Only the wall-clock fields are ever shown.
pub fn parse_datetime(raw: &str) -> Option<NaiveDateTime> {
    let s = raw.trim();
    let s = s.strip_suffix('Z').unwrap_or(s);
    let head = s.split('+').next().unwrap_or(s);
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M"] {
        if let Ok(v) = NaiveDateTime::parse_from_str(head, fmt) {
            return Some(v);
        }
    }
    NaiveDate::parse_from_str(head, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0))
}

/// Reads a date typed by hand, in the local or the ISO notation.
pub fn parse_date_input(value: &str) -> Option<NaiveDate> {
    let value = value.trim();
    NaiveDate::parse_from_str(value, "%d.%m.%Y").or_else(|_| NaiveDate::parse_from_str(value, "%Y-%m-%d")).ok()
}

pub fn date(v: Option<NaiveDateTime>) -> String {
    v.map_or_else(|| EMPTY.into(), |d| format!("{:02}.{:02}.{:04}", d.day(), d.month(), d.year()))
}

pub fn time(v: Option<NaiveDateTime>) -> String {
    v.map_or_else(|| EMPTY.into(), |d| format!("{:02}:{:02}", d.hour(), d.minute()))
}

pub fn datetime(v: Option<NaiveDateTime>) -> String {
    match v {
        Some(_) => format!("{} {}", date(v), time(v)),
        None => EMPTY.into(),
    }
}

pub fn datetime_str(raw: Option<&str>) -> String {
    datetime(raw.and_then(parse_datetime))
}

pub fn weekday(v: Option<NaiveDateTime>) -> &'static str {
    v.map_or(EMPTY, |d| WEEKDAYS[d.weekday().num_days_from_sunday() as usize])
}

/// `ru-RU` grouping with two decimals of the absolute value; the sign lives in
/// the transaction type column.
pub fn amount(raw: Option<&str>) -> String {
    let Some(raw) = raw.filter(|s| !s.is_empty()) else { return EMPTY.into() };
    match raw.trim().parse::<f64>() {
        Ok(v) if v.is_finite() => number(v.abs(), 2),
        _ => raw.to_string(),
    }
}

pub fn number(v: f64, decimals: usize) -> String {
    let fixed = format!("{v:.decimals$}");
    let (sign, body) = fixed.strip_prefix('-').map_or(("", fixed.as_str()), |b| ("-", b));
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    let mut out = String::with_capacity(fixed.len() + int.len() / 3 + 1);
    out.push_str(sign);
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(NBSP);
        }
        out.push(ch);
    }
    if !frac.is_empty() {
        out.push(',');
        out.push_str(frac);
    }
    out
}

pub fn count(v: usize) -> String {
    number(v as f64, 0)
}

/// Russian plural: 1 запись, 2 записи, 5 записей.
pub fn plural<'a>(n: usize, one: &'a str, few: &'a str, many: &'a str) -> &'a str {
    let (t, u) = (n % 100, n % 10);
    if (11..=14).contains(&t) {
        many
    } else if u == 1 {
        one
    } else if (2..=4).contains(&u) {
        few
    } else {
        many
    }
}

pub fn type_label(code: Option<&str>) -> Option<&'static str> {
    match code? {
        "DEBIT" => Some("Списание"),
        "CREDIT" => Some("Пополнение"),
        "CONVERSION" => Some("Конверсия"),
        "REVERSAL" => Some("Отмена"),
        _ => None,
    }
}

/// The bookkeeping month the way the ledger writes it: the server stores
/// `2026-03-01`, people read and type `03.2026`.
pub fn period(raw: &str) -> String {
    let raw = raw.trim();
    match (raw.get(0..4), raw.get(4..5), raw.get(5..7)) {
        (Some(year), Some("-"), Some(month)) if year.bytes().chain(month.bytes()).all(|b| b.is_ascii_digit()) => format!("{month}.{year}"),
        _ => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        let d = parse_datetime("2026-01-04T07:16:00");
        assert_eq!(date(d), "04.01.2026");
        assert_eq!(time(d), "07:16");
        assert_eq!(datetime(d), "04.01.2026 07:16");
        assert_eq!(weekday(d), "вс");
        assert!(parse_datetime("2026-09-01T14:30:52.693990").is_some());
        assert!(parse_datetime("2026-11-06T19:16:10.637170Z").is_some());
        assert!(parse_datetime("2026-11-06").is_some());
        assert_eq!(date(parse_datetime("мусор")), EMPTY);
    }

    #[test]
    fn amounts_match_intl_ru() {
        assert_eq!(amount(Some("5000.00")), "5\u{a0}000,00");
        assert_eq!(amount(Some("-1234567.5")), "1\u{a0}234\u{a0}567,50");
        assert_eq!(amount(Some("0")), "0,00");
        assert_eq!(amount(Some("999.999")), "1\u{a0}000,00");
        assert_eq!(amount(None), EMPTY);
        assert_eq!(amount(Some("abc")), "abc");
    }

    #[test]
    fn plurals() {
        let p = |n| plural(n, "запись", "записи", "записей");
        assert_eq!([p(1), p(2), p(5), p(11), p(21), p(112)], ["запись", "записи", "записей", "записей", "запись", "записей"]);
    }

    #[test]
    fn a_period_reads_as_a_month() {
        assert_eq!(period("2026-07-01"), "07.2026");
        assert_eq!(period("2026-07"), "07.2026");
        assert_eq!(period("07.2026"), "07.2026");
        assert_eq!(period(" 2026-12-01 "), "12.2026");
        assert_eq!(period("июль"), "июль");
        assert_eq!(period(""), "");
    }
}
