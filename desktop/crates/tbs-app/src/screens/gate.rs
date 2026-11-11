//! The screens that stand between a valid session and the work: an expired
//! licence and the launch password.

use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDate, Utc};
use iced::widget::{center, column, container};
use iced::{Alignment, Fill, Subscription, Task};
use tbs_core::api::{LaunchStatus, LaunchVerify};

use crate::ctx::{Ctx, text_error};
use crate::theme::{self, Tone, space};
use crate::ui::{self, El, Icon, Kind};

const MONTHS: [&str; 12] = [
    "января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря",
];

/// `2026-07-01` or a full timestamp to "1 июля 2026 г.".
pub fn long_date(raw: &str) -> String {
    let date = DateTime::parse_from_rfc3339(raw)
        .map(|v| v.with_timezone(&Local).date_naive())
        .ok()
        .or_else(|| NaiveDate::parse_from_str(raw.get(..10).unwrap_or(raw), "%Y-%m-%d").ok());
    match date {
        Some(d) => format!("{} {} {} г.", d.day(), MONTHS[d.month0() as usize], d.year()),
        None => raw.to_owned(),
    }
}

/// Licence refused, or the client has no way to reach the server legitimately.
pub fn blocked<'a, M: Clone + 'a>(expired_at: Option<&'a str>, retry: M, busy: bool) -> El<'a, M> {
    let mut body = column![ui::icon_tone(Icon::Lock, 28.0, Tone::Danger)]
        .spacing(space::MD)
        .align_x(Alignment::Center)
        .max_width(480);
    match expired_at {
        Some(date) => {
            body = body
                .push(ui::title("Срок доступа истёк"))
                .push(ui::label(format!("Доступ к программе действовал до {}.", long_date(date))))
                .push(ui::muted(
                    "Обратитесь к администратору, чтобы продлить. После продления просто запустите программу заново — входить в аккаунт повторно не нужно.",
                ));
        }
        None => {
            body = body.push(ui::title("Доступ заблокирован")).push(ui::muted(
                "Не удалось проверить доступ. Проверьте подключение к сети и запустите программу заново; если это повторяется, обратитесь к администратору.",
            ));
        }
    }
    body = body.push(ui::btn(if busy { "Проверяем…" } else { "Проверить снова" }, Kind::Primary, (!busy).then_some(retry)));
    container(center(ui::card(body).max_width(540))).width(Fill).height(Fill).style(theme::page).into()
}

/// Password asked before the client opens, when the administrator requires it.
#[derive(Default)]
pub struct LaunchGate {
    password: String,
    busy: bool,
    error: Option<String>,
    hint: Option<String>,
    locked_until: Option<DateTime<Utc>>,
    pub unlocked: bool,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Status(Result<LaunchStatus, String>),
    Password(String),
    Submit,
    Answer(Result<LaunchVerify, String>),
    Tick,
}

fn parse_until(raw: Option<&str>) -> Option<DateTime<Utc>> {
    let raw = raw?;
    DateTime::parse_from_rfc3339(raw)
        .map(|v| v.with_timezone(&Utc))
        .ok()
        .or_else(|| chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f").ok().map(|v| v.and_utc()))
}

impl LaunchGate {
    pub fn load(ctx: &Ctx) -> Task<Msg> {
        let api = ctx.api.clone();
        Task::perform(async move { api.launch_status().await.map_err(text_error) }, Msg::Status)
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        if self.locked_until.is_some() {
            iced::time::every(Duration::from_secs(1)).map(|_| Msg::Tick)
        } else {
            Subscription::none()
        }
    }

    fn seconds_left(&self) -> i64 {
        self.locked_until.map(|until| (until - Utc::now()).num_seconds().max(0)).unwrap_or(0)
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Status(Ok(status)) => {
                if !status.password_set {
                    self.unlocked = true;
                }
                self.locked_until = parse_until(status.locked_until.as_deref());
            }
            Msg::Status(Err(reason)) => self.error = Some(reason),
            Msg::Password(value) => self.password = value,
            Msg::Submit => {
                if self.busy || self.password.is_empty() || self.seconds_left() > 0 {
                    return Task::none();
                }
                self.busy = true;
                self.error = None;
                let (api, password) = (ctx.api.clone(), self.password.clone());
                return Task::perform(async move { api.verify_launch(&password).await.map_err(text_error) }, Msg::Answer);
            }
            Msg::Answer(answer) => {
                self.busy = false;
                self.password.clear();
                match answer {
                    Ok(reply) if reply.verified => self.unlocked = true,
                    Ok(reply) => {
                        self.locked_until = parse_until(reply.locked_until.as_deref());
                        self.error = Some("Неверный пароль запуска".into());
                        self.hint = reply.attempts_left.map(|n| format!("Осталось попыток: {n}"));
                    }
                    Err(reason) => self.error = Some(reason),
                }
            }
            Msg::Tick => {
                if self.seconds_left() == 0 {
                    self.locked_until = None;
                    self.error = None;
                    self.hint = None;
                }
            }
        }
        Task::none()
    }

    pub fn view(&self) -> El<'_, Msg> {
        let left = self.seconds_left();
        let ready = !self.busy && !self.password.is_empty() && left == 0;
        let mut body = column![
            ui::icon(Icon::Lock, 24.0),
            ui::title("Пароль запуска"),
            ui::muted("Введите пароль, заданный администратором."),
            ui::secret_input("Пароль", &self.password).on_input(Msg::Password).on_submit(Msg::Submit),
        ]
        .spacing(space::MD);
        if left > 0 {
            body = body.push(ui::error_line(format!("Вход заблокирован. Повторите через {} сек", left)));
        } else if let Some(error) = &self.error {
            body = body.push(ui::error_line(error.as_str()));
        }
        if let Some(hint) = &self.hint {
            body = body.push(ui::muted(hint.as_str()));
        }
        body = body.push(ui::btn(if self.busy { "Проверка…" } else { "Продолжить" }, Kind::Primary, ready.then_some(Msg::Submit)).width(Fill));
        container(center(ui::card(body).max_width(400))).width(Fill).height(Fill).style(theme::page).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_the_way_people_say_them() {
        assert_eq!(long_date("2026-07-01"), "1 июля 2026 г.");
        assert_eq!(long_date("2026-12-31T23:59:59"), "31 декабря 2026 г.");
    }
}
