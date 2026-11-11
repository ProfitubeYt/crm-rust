//! Which years the account may open, and the code that opens a locked one.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, NaiveDateTime};
use iced::widget::{column, container, row};
use iced::{Alignment, Fill, Subscription, Task};
use serde::{Deserialize, Serialize};
use tbs_core::api::{AccessScope, AccessStatus, InitReply, LockedPeriod, ScopeCode, UnlockedScope};
use tbs_core::format;
use tbs_core::query::Scope;

use super::{Msg as Screen, Transactions, filters};
use crate::ctx::{Ctx, text_error};
use crate::theme::{self, Tone, space};
use crate::ui::{self, El, Icon, Kind};

/// The widest span of years a scope period is unrolled into.
const LONGEST_RANGE: i32 = 50;
/// Absent from the list this many times in a row, the chosen year gives way.
const MISSING_LIMIT: u8 = 3;
const YEAR_STATE: &str = "transactions-year";

#[derive(Clone, Debug)]
pub enum Msg {
    Status(Result<AccessStatus, String>),
    Scopes(Result<Vec<AccessScope>, String>),
    Locked(Result<Vec<LockedPeriod>, String>),
    Init(Result<InitReply, String>),
    Code(String),
    Request,
    Requested(i64, Result<ScopeCode, String>),
    Verify,
    Verified(Result<UnlockedScope, String>),
    Tick,
    Expired,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedYear {
    pub year: Option<i32>,
}

#[derive(Default)]
pub struct Access {
    pub status: Option<AccessStatus>,
    status_failed: bool,
    /// A year was just changed; the status on hand speaks for the previous one.
    status_pending: bool,
    scopes: Vec<AccessScope>,
    scopes_asked: bool,
    locked: Vec<LockedPeriod>,
    init_loaded: bool,
    init_years: Vec<i32>,
    init_available: Vec<i32>,
    code: String,
    requesting: bool,
    verifying: bool,
    auto_key: Option<String>,
    otp_scope: Option<i64>,
    expires_at: Option<Instant>,
    seconds_left: i64,
    missing: u8,
    tokens: HashMap<i32, String>,
}

fn year_of(raw: Option<&str>) -> Option<i32> {
    raw.and_then(format::parse_datetime).map(|d| d.year())
}

fn moment(raw: Option<&str>) -> Option<NaiveDateTime> {
    raw.and_then(format::parse_datetime)
}

fn year_range(start: Option<i32>, end: Option<i32>) -> Vec<i32> {
    let (Some(start), Some(end)) = (start.or(end), end.or(start)) else { return Vec::new() };
    let (low, high) = (start.min(end), start.max(end));
    if high - low > LONGEST_RANGE {
        return Vec::new();
    }
    (low..=high).collect()
}

fn years_in(value: &serde_json::Value) -> Vec<i32> {
    value["available_years"].as_array().map_or_else(Vec::new, |items| items.iter().filter_map(|y| y.as_i64()).map(|y| y as i32).collect())
}

impl Access {
    fn enabled(&self) -> bool {
        self.status.as_ref().is_some_and(|s| s.scopes_enabled)
    }

    fn current(&self) -> Option<&AccessScope> {
        self.status.as_ref().and_then(|s| s.current_scope.as_ref())
    }

    pub fn year_options(&self, data_years: &[i32], this_year: i32) -> Vec<i32> {
        let mut years: BTreeSet<i32> = self.init_years.iter().copied().collect();
        if let Some(status) = &self.status {
            years.extend(&status.available_years);
        }
        if self.enabled() {
            years.extend(&self.init_available);
            years.extend(self.scopes.iter().flat_map(|s| s.years.iter()));
            if let Some(current) = self.current() {
                years.extend(&current.years);
                years.extend(year_range(year_of(current.period_start.as_deref()), year_of(current.period_end.as_deref())));
            }
        } else {
            years.extend(data_years);
        }
        if years.is_empty() {
            years.insert(this_year);
        }
        years.into_iter().rev().collect()
    }

    fn needs_initial_unlock(&self, admin: bool) -> bool {
        !admin && self.enabled() && self.current().is_none()
    }

    fn year_requires_unlock(&self, admin: bool, year: Option<i32>) -> bool {
        let (false, true, Some(year), Some(current)) = (admin, self.enabled(), year, self.current()) else { return false };
        let outside_years = !current.years.is_empty() && !current.years.contains(&year);
        let before = year_of(current.period_start.as_deref()).is_some_and(|start| year < start);
        let after = year_of(current.period_end.as_deref()).is_some_and(|end| year > end);
        outside_years || before || after
    }

    pub fn must_unlock(&self, admin: bool, year: Option<i32>) -> bool {
        self.needs_initial_unlock(admin) || self.year_requires_unlock(admin, year)
    }

    pub fn locked_view(&self, admin: bool, year: Option<i32>) -> bool {
        let has_snapshot = admin || self.status.is_some();
        !has_snapshot || self.must_unlock(admin, year)
    }

    fn target_scope(&self, year: Option<i32>) -> Option<&AccessScope> {
        let mut usable = self.scopes.iter().filter(|s| s.is_active && s.allow_transactions);
        let first = usable.clone().next()?;
        let Some(year) = year else { return Some(first) };
        let fits = |s: &&AccessScope| {
            let in_years = s.years.is_empty() || s.years.contains(&year);
            let from = year_of(s.period_start.as_deref()).is_none_or(|start| year >= start);
            let to = year_of(s.period_end.as_deref()).is_none_or(|end| year <= end);
            in_years && from && to
        };
        Some(usable.find(fits).unwrap_or(first))
    }

    pub fn data_scope(&self, year: Option<i32>) -> Scope {
        let mut scope = Scope { year, ..Default::default() };
        if self.enabled()
            && let Some(current) = self.current()
        {
            scope.allowed_years = current.years.clone();
            scope.start = moment(current.period_start.as_deref());
            scope.end = moment(current.period_end.as_deref());
        }
        scope
    }

    fn banner(&self, year: Option<i32>) -> Option<String> {
        let overlaps: Vec<&LockedPeriod> = self
            .locked
            .iter()
            .filter(|p| {
                let Some(year) = year else { return true };
                let (from, to) = (year_of(Some(&p.date_from)), year_of(Some(&p.date_to)));
                from.is_some_and(|f| f <= year) && to.is_some_and(|t| year <= t)
            })
            .collect();
        match overlaps.as_slice() {
            [] => None,
            [one] => {
                let reason = one.reason.as_deref().filter(|r| !r.is_empty()).map_or(String::new(), |r| format!(" ({r})"));
                Some(format!("Есть заблокированный период: {} — {}{reason}.", one.date_from, one.date_to))
            }
            many => Some(format!("Есть {} заблокированных периодов. Экспорт и доступ к этим датам ограничены.", many.len())),
        }
    }

    fn forget_code(&mut self) {
        self.code.clear();
        self.otp_scope = None;
        self.seconds_left = 0;
        self.expires_at = None;
    }

    pub fn timer(&self) -> Subscription<Screen> {
        if self.expires_at.is_none() {
            return Subscription::none();
        }
        iced::time::every(Duration::from_secs(1)).map(|_| Screen::Access(Msg::Tick))
    }

    /// The next time the screen is opened starts from a fresh code.
    pub fn close_door(&mut self) {
        self.tokens.clear();
        self.status = None;
        self.auto_key = None;
        self.forget_code();
    }

    /// Brings back the code already earned for this year, if any.
    pub fn token_for(&self, year: i32) -> Option<String> {
        self.tokens.get(&year).cloned()
    }

    pub fn expect_status(&mut self) {
        self.status_pending = true;
    }
}

fn send(message: Msg) -> Screen {
    Screen::Access(message)
}

pub(super) fn fetch_status(ctx: &Ctx) -> Task<Screen> {
    let api = ctx.api.clone();
    Task::perform(async move { api.access_status().await.map_err(text_error) }, |r| send(Msg::Status(r)))
}

fn fetch_init(ctx: &Ctx) -> Task<Screen> {
    let api = ctx.api.clone();
    Task::perform(async move { api.transactions_init().await.map_err(text_error) }, |r| send(Msg::Init(r)))
}

fn fetch_scopes(ctx: &Ctx) -> Task<Screen> {
    let api = ctx.api.clone();
    Task::perform(async move { api.access_scopes().await.map_err(text_error) }, |r| send(Msg::Scopes(r)))
}

pub(super) fn fetch_all(ctx: &Ctx) -> Task<Screen> {
    let api = ctx.api.clone();
    let locked = Task::perform(async move { api.locked_periods().await.map_err(text_error) }, |r| send(Msg::Locked(r)));
    Task::batch([fetch_status(ctx), fetch_init(ctx), locked])
}

fn banner<'a>(tone: Tone, name: &'a str, hint: &'a str, action: Option<El<'a, Screen>>) -> El<'a, Screen> {
    // The words take what the button leaves and wrap there: in a narrow page a long hint pushed the button out.
    let mut line = row![column![ui::strong(name), ui::muted(hint)].spacing(space::XXS).width(Fill)].spacing(space::MD).align_y(Alignment::Center);
    if let Some(action) = action {
        line = line.push(action);
    }
    container(line).padding([space::SM, space::MD]).width(Fill).style(theme::tinted(tone)).into()
}

impl Transactions {
    /// Whatever arrived, the year list, the table and the code follow from it.
    fn settle_access(&mut self, ctx: &mut Ctx) -> Task<Screen> {
        self.settle_year(ctx);
        self.refresh_keeping_selection();
        self.auto_request(ctx)
    }

    pub(super) fn settle_year(&mut self, ctx: &Ctx) {
        self.years = self.access.year_options(&self.data.years, Local::now().year());
        let first = self.years.first().copied();
        let known = self.access.status.is_some() && self.access.init_loaded;
        match self.year {
            None => self.remember_year(first, ctx),
            Some(year) if self.years.contains(&year) => self.access.missing = 0,
            Some(_) if known => {
                self.access.missing += 1;
                if self.access.missing >= MISSING_LIMIT {
                    self.access.missing = 0;
                    self.remember_year(first, ctx);
                }
            }
            Some(_) => {}
        }
    }

    pub(super) fn remember_year(&mut self, year: Option<i32>, ctx: &Ctx) {
        self.year = year;
        if let Err(error) = tbs_core::config::save_json(&ctx.state_path(YEAR_STATE), &SavedYear { year }) {
            tracing::warn!("year not saved: {error}");
        }
    }

    pub(super) fn saved_year(ctx: &Ctx) -> Option<i32> {
        tbs_core::config::load_json::<SavedYear>(&ctx.state_path(YEAR_STATE)).year
    }

    fn auto_request(&mut self, ctx: &mut Ctx) -> Task<Screen> {
        let year = self.year;
        let target = self.access.target_scope(year).map(|s| s.id);
        let (true, Some(target)) = (self.access.must_unlock(self.admin, year), target) else {
            self.access.auto_key = None;
            return Task::none();
        };
        if self.access.status_pending || self.access.requesting {
            return Task::none();
        }
        if self.access.otp_scope == Some(target) && self.access.seconds_left > 0 {
            return Task::none();
        }
        let key = format!("{}:{target}", year.map_or("all".to_string(), |y| y.to_string()));
        if self.access.auto_key.as_deref() == Some(key.as_str()) {
            return Task::none();
        }
        self.access.auto_key = Some(key);
        self.request_code(target, ctx)
    }

    fn request_code(&mut self, scope: i64, ctx: &Ctx) -> Task<Screen> {
        self.access.requesting = true;
        let api = ctx.api.clone();
        Task::perform(async move { api.request_scope_code(scope).await.map_err(text_error) }, move |r| send(Msg::Requested(scope, r)))
    }

    fn opened(&mut self, token: String, ctx: &Ctx) -> Task<Screen> {
        if let Some(year) = self.year {
            self.access.tokens.insert(year, token);
        }
        self.access.forget_code();
        Task::batch([fetch_status(ctx), fetch_init(ctx), Task::done(Screen::Sync(true))])
    }

    pub(super) fn access_update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Screen> {
        match message {
            Msg::Status(Ok(status)) => {
                let enabled = status.scopes_enabled;
                self.access.status = Some(status);
                self.access.status_failed = false;
                self.access.status_pending = false;
                let scopes = if enabled && !self.access.scopes_asked {
                    self.access.scopes_asked = true;
                    fetch_scopes(ctx)
                } else {
                    Task::none()
                };
                let settled = self.settle_access(ctx);
                let wanted = match self.want_sync {
                    Some(force) if !self.locked_view() => {
                        self.want_sync = None;
                        Task::done(Screen::Sync(force))
                    }
                    _ => Task::none(),
                };
                return Task::batch([scopes, settled, wanted]);
            }
            Msg::Status(Err(reason)) => {
                tracing::warn!("access status: {reason}");
                self.access.status_failed = true;
                self.access.status_pending = false;
            }
            Msg::Scopes(Ok(scopes)) => {
                self.access.scopes = scopes;
                return self.settle_access(ctx);
            }
            Msg::Scopes(Err(reason)) => tracing::warn!("access scopes: {reason}"),
            Msg::Locked(Ok(periods)) => self.access.locked = periods,
            Msg::Locked(Err(reason)) => tracing::warn!("locked periods: {reason}"),
            Msg::Init(Ok(init)) => {
                self.access.init_loaded = true;
                self.access.init_years = init.years.iter().map(|y| y.year).collect();
                self.access.init_available = years_in(&init.security_status);
                self.add_options(init.operators, init.applications);
                return self.settle_access(ctx);
            }
            Msg::Init(Err(reason)) => tracing::warn!("transactions init: {reason}"),
            Msg::Code(value) => self.access.code = value,
            Msg::Request => match self.access.target_scope(self.year).map(|s| s.id) {
                Some(scope) => return self.request_code(scope, ctx),
                None => ctx.toast(Tone::Danger, "Не найден активный scope для OTP"),
            },
            Msg::Requested(scope, result) => {
                self.access.requesting = false;
                match result {
                    Ok(reply) => {
                        if let Some(token) = reply.token {
                            return self.opened(token, ctx);
                        }
                        let seconds = reply.expires_in.max(0);
                        self.access.otp_scope = Some(scope);
                        self.access.code.clear();
                        self.access.seconds_left = seconds;
                        self.access.expires_at = Some(Instant::now() + Duration::from_secs(seconds as u64));
                        ctx.toast(Tone::Success, "Код подтверждения отправлен в AuthBot");
                    }
                    Err(reason) => ctx.toast(Tone::Danger, if reason.is_empty() { "Не удалось запросить OTP-код".to_string() } else { reason }),
                }
            }
            Msg::Verify => {
                let Some(scope) = self.access.otp_scope else {
                    ctx.toast(Tone::Danger, "Сначала запросите OTP-код");
                    return Task::none();
                };
                let code = self.access.code.trim().to_string();
                if code.is_empty() {
                    ctx.toast(Tone::Danger, "Введите OTP-код");
                    return Task::none();
                }
                self.access.verifying = true;
                let api = ctx.api.clone();
                return Task::perform(async move { api.verify_scope_code(scope, &code).await.map_err(text_error) }, |r| send(Msg::Verified(r)));
            }
            Msg::Verified(result) => {
                self.access.verifying = false;
                match result {
                    Ok(unlocked) => {
                        ctx.toast(Tone::Success, "Доступ к периоду открыт по OTP");
                        return self.opened(unlocked.token, ctx);
                    }
                    Err(reason) => ctx.toast(Tone::Danger, if reason.is_empty() { "Не удалось подтвердить OTP-код".to_string() } else { reason }),
                }
            }
            Msg::Tick => {
                if let Some(expires) = self.access.expires_at {
                    let left = expires.saturating_duration_since(Instant::now()).as_secs_f64().ceil() as i64;
                    self.access.seconds_left = left;
                    if left <= 0 {
                        self.access.expires_at = None;
                    }
                }
            }
            Msg::Expired => {
                if let Some(year) = self.year {
                    self.access.tokens.remove(&year);
                }
                return fetch_status(ctx);
            }
        }
        Task::none()
    }

    fn otp_view(&self) -> El<'_, Screen> {
        let access = &self.access;
        let mut head = row![ui::icon_tone(Icon::Lock, 14.0, Tone::Warning), ui::strong("Требуется OTP-код")].spacing(space::SM).align_y(Alignment::Center);
        if let Some(year) = self.year {
            head = head.push(ui::muted(format!("· папка {year}")));
        }
        let mut form = row![
            ui::input("OTP-код", &access.code).on_input(|v| send(Msg::Code(v))).on_submit(send(Msg::Verify)).width(180),
            ui::btn(if access.requesting { "Запрос…" } else { "Запросить код" }, Kind::Secondary, (!access.requesting).then_some(send(Msg::Request))),
            ui::btn(if access.verifying { "Проверка…" } else { "Подтвердить" }, Kind::Primary, (!access.verifying).then_some(send(Msg::Verify))),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center);
        if access.seconds_left > 0 {
            form = form.push(ui::muted(format!("код действует {}с", access.seconds_left)));
        }
        let body = column![head, ui::muted("Код отправляется через AuthBot администраторам. После запроса введите код ниже."), form].spacing(space::SM);
        container(body).padding(space::MD).width(Fill).style(theme::tinted(Tone::Warning)).into()
    }

    pub(super) fn banners(&self, ctx: &Ctx) -> Vec<El<'_, Screen>> {
        let mut items: Vec<El<'_, Screen>> = Vec::new();
        if !self.admin && self.access.status.is_none() {
            let hint = if self.access.status_failed { "Статус безопасности недоступен" } else { "Подождите несколько секунд." };
            items.push(banner(Tone::Neutral, "Проверяем доступ…", hint, None));
        }
        if self.access.must_unlock(self.admin, self.year) {
            items.push(self.otp_view());
        }
        if let Some(text) = self.access.banner(self.year) {
            items.push(ui::notice(Tone::Warning, text));
        }
        if !ctx.online {
            items.push(banner(Tone::Danger, "Сервер недоступен", "Включён read-only fallback из локального кеша.", None));
        }
        let filtered = self.table.filters.active_count() > 0 || !self.search.trim().is_empty();
        if !self.locked_view() && filtered && self.rows.is_empty() && !self.data.is_empty() {
            // The filters and the search are what hides the rows. The look of the table is not at fault and stays.
            let action = ui::btn("Сбросить фильтры и поиск", Kind::Secondary, Some(Screen::Filters(filters::Msg::ShowAll)));
            items.push(banner(Tone::Warning, "Фильтры и поиск скрыли все строки", "Под эти условия не подходит ни одна транзакция. Вид таблицы останется прежним.", Some(action.into())));
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(id: i64, years: &[i32]) -> AccessScope {
        AccessScope { id, years: years.to_vec(), is_active: true, allow_transactions: true, ..Default::default() }
    }

    fn with(status: AccessStatus) -> Access {
        Access { status: Some(status), ..Default::default() }
    }

    fn scoped(current: Option<AccessScope>) -> Access {
        with(AccessStatus { scopes_enabled: true, current_scope: current, ..Default::default() })
    }

    #[test]
    fn without_scopes_the_years_come_from_the_data() {
        let access = with(AccessStatus { available_years: vec![2024], ..Default::default() });
        assert_eq!(access.year_options(&[2026, 2025], 2030), [2026, 2025, 2024]);
    }

    #[test]
    fn with_scopes_the_data_does_not_add_years() {
        let mut access = scoped(Some(AccessScope { period_start: Some("2021-03-01T00:00:00".into()), period_end: Some("2022-02-01T00:00:00".into()), ..Default::default() }));
        access.scopes = vec![scope(1, &[2019])];
        assert_eq!(access.year_options(&[2026], 2030), [2022, 2021, 2019]);
    }

    #[test]
    fn nothing_known_falls_back_to_this_year() {
        assert_eq!(Access::default().year_options(&[], 2030), [2030]);
    }

    #[test]
    fn range_of_a_lifetime_is_refused() {
        assert_eq!(year_range(Some(1900), Some(2026)), Vec::<i32>::new());
        assert_eq!(year_range(Some(2025), None), [2025]);
        assert_eq!(year_range(Some(2026), Some(2024)), [2024, 2025, 2026]);
    }

    #[test]
    fn a_stranger_sees_nothing_before_the_status_arrives() {
        let access = Access::default();
        assert!(access.locked_view(false, Some(2026)));
        assert!(!access.locked_view(true, Some(2026)));
    }

    #[test]
    fn enabled_scopes_lock_until_a_period_is_open() {
        let access = scoped(None);
        assert!(access.locked_view(false, Some(2026)));
        assert!(!access.locked_view(true, Some(2026)));
    }

    #[test]
    fn open_scope_covers_only_its_years() {
        let access = scoped(Some(scope(1, &[2025])));
        assert!(!access.locked_view(false, Some(2025)));
        assert!(access.locked_view(false, Some(2026)));
    }

    #[test]
    fn open_period_covers_only_its_span() {
        let access = scoped(Some(AccessScope { period_start: Some("2024-01-01T00:00:00".into()), period_end: Some("2025-12-31T00:00:00".into()), ..Default::default() }));
        assert!(access.locked_view(false, Some(2023)));
        assert!(!access.locked_view(false, Some(2024)));
        assert!(access.locked_view(false, Some(2026)));
    }

    #[test]
    fn the_scope_that_fits_the_year_is_asked_first() {
        let mut access = Access::default();
        access.scopes = vec![
            AccessScope { is_active: false, allow_transactions: true, ..scope(1, &[2026]) },
            scope(2, &[2024]),
            scope(3, &[2026]),
        ];
        assert_eq!(access.target_scope(Some(2026)).map(|s| s.id), Some(3));
        assert_eq!(access.target_scope(Some(2000)).map(|s| s.id), Some(2));
        assert_eq!(access.target_scope(None).map(|s| s.id), Some(2));
        access.scopes.clear();
        assert!(access.target_scope(Some(2026)).is_none());
    }

    #[test]
    fn data_is_cut_to_the_open_scope() {
        let access = scoped(Some(AccessScope { years: vec![2025], period_start: Some("2025-02-01T00:00:00".into()), ..Default::default() }));
        let scope = access.data_scope(Some(2025));
        assert_eq!(scope.allowed_years, [2025]);
        assert_eq!(scope.start.map(|d| d.month()), Some(2));
        assert!(scope.end.is_none());
        assert_eq!(Access::default().data_scope(Some(2025)).allowed_years, Vec::<i32>::new());
    }

    #[test]
    fn locked_periods_read_as_one_or_as_a_count() {
        let period = |from: &str, to: &str, reason: Option<&str>| LockedPeriod { date_from: from.into(), date_to: to.into(), reason: reason.map(Into::into), ..Default::default() };
        let mut access = Access { locked: vec![period("2025-01-01", "2025-03-31", Some("сверка"))], ..Default::default() };
        assert_eq!(access.banner(Some(2025)).unwrap(), "Есть заблокированный период: 2025-01-01 — 2025-03-31 (сверка).");
        assert_eq!(access.banner(Some(2026)), None);
        access.locked.push(period("2025-06-01", "2026-01-31", None));
        assert_eq!(access.banner(Some(2025)).unwrap(), "Есть 2 заблокированных периодов. Экспорт и доступ к этим датам ограничены.");
    }
}
