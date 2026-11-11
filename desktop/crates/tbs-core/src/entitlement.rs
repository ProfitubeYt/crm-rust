//! Remembers how long this account may use the app.
//!
//! The server's answer is kept on disk so a launch without network is not a
//! locked door, but only for a grace period: long enough to survive an outage,
//! short enough that unplugging the cable is not a way to keep working. The
//! server remains the one that actually refuses requests.

use std::path::Path;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::api::Entitlement;
use crate::config::{load_json, save_json};

pub const STATE_FILE: &str = "entitlement.json";
pub const OFFLINE_GRACE_DAYS: i64 = 7;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CachedEntitlement {
    pub ok: bool,
    pub expired: bool,
    pub allowed_until: Option<String>,
    pub note: Option<String>,
    pub username: Option<String>,
    pub checked_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing worth believing is stored; the server has to decide.
    Unknown,
    Allowed,
    Blocked { allowed_until: Option<String> },
}

fn parse_moment(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(value) = DateTime::parse_from_rfc3339(raw) {
        return Some(value.with_timezone(&Utc));
    }
    if let Ok(value) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(value.and_utc());
    }
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()?.and_hms_opt(0, 0, 0).map(|value| value.and_utc())
}

impl CachedEntitlement {
    pub fn from_server(answer: &Entitlement, now: DateTime<Utc>) -> Self {
        Self {
            ok: answer.ok,
            expired: answer.expired,
            allowed_until: answer.allowed_until.clone(),
            note: answer.note.clone(),
            username: answer.username.clone(),
            checked_at: Some(now),
        }
    }

    /// What this record is worth at `now`. A cache cannot outlive the
    /// entitlement it describes, and a clock rolled far backwards is not a free
    /// extension.
    pub fn evaluate(&self, now: DateTime<Utc>) -> Verdict {
        let Some(checked_at) = self.checked_at else { return Verdict::Unknown };
        let age = now - checked_at;
        if age > Duration::days(OFFLINE_GRACE_DAYS) || age < Duration::days(-1) {
            return Verdict::Unknown;
        }
        if let Some(until) = self.allowed_until.as_deref().and_then(parse_moment) {
            if until <= now {
                return Verdict::Blocked { allowed_until: self.allowed_until.clone() };
            }
        }
        if self.ok { Verdict::Allowed } else { Verdict::Blocked { allowed_until: self.allowed_until.clone() } }
    }

    pub fn load(dir: &Path) -> Self {
        load_json(&dir.join(STATE_FILE))
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        save_json(&dir.join(STATE_FILE), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(age_days: i64, until: Option<&str>, now: DateTime<Utc>) -> CachedEntitlement {
        CachedEntitlement {
            ok: true,
            allowed_until: until.map(str::to_owned),
            checked_at: Some(now - Duration::days(age_days)),
            ..Default::default()
        }
    }

    fn now() -> DateTime<Utc> {
        parse_moment("2026-06-15T12:00:00Z").unwrap()
    }

    #[test]
    fn nothing_stored_decides_nothing() {
        assert_eq!(CachedEntitlement::default().evaluate(now()), Verdict::Unknown);
    }

    #[test]
    fn fresh_answer_is_believed() {
        assert_eq!(record(3, Some("2026-07-01"), now()).evaluate(now()), Verdict::Allowed);
    }

    #[test]
    fn grace_period_closes() {
        assert_eq!(record(8, None, now()).evaluate(now()), Verdict::Unknown);
    }

    #[test]
    fn rolled_back_clock_is_not_trusted() {
        assert_eq!(record(-3, None, now()).evaluate(now()), Verdict::Unknown);
    }

    #[test]
    fn stored_date_in_the_past_blocks_while_offline() {
        let verdict = record(1, Some("2026-06-10"), now()).evaluate(now());
        assert_eq!(verdict, Verdict::Blocked { allowed_until: Some("2026-06-10".into()) });
    }

    #[test]
    fn server_refusal_is_remembered() {
        let mut value = record(1, None, now());
        value.ok = false;
        assert!(matches!(value.evaluate(now()), Verdict::Blocked { .. }));
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        record(1, Some("2026-07-01"), now()).save(dir.path()).unwrap();
        assert_eq!(CachedEntitlement::load(dir.path()).evaluate(now()), Verdict::Allowed);
    }
}
