//! Date ranges nobody may edit, with per-user exceptions.

use iced::widget::{Column, Row, Space, button, column, row};
use iced::{Alignment, Fill, FillPortion, Length, Task};
use serde_json::json;
use tbs_core::api::{AdminUser, PeriodRecord};
use tbs_core::format::parse_date_input;

use crate::ctx::{Ctx, text_error};
use crate::theme::{self, Tone, space};
use crate::ui::{self, El, Icon, Kind};

use super::{panel, plural_users};

#[derive(Clone, Debug)]
pub enum Msg {
    Periods(Result<Vec<PeriodRecord>, String>),
    Users(Result<Vec<AdminUser>, String>),
    From(String),
    To(String),
    Reason(String),
    Add,
    Added(Result<PeriodRecord, String>),
    Remove(i64),
    Removed(Result<serde_json::Value, String>),
    Expand(i64),
    Collapse,
    Pick(i64),
    Clear,
    SaveExclusions,
    Saved(Result<PeriodRecord, String>),
}

struct Draft {
    period: i64,
    excluded: Vec<i64>,
    saving: bool,
}

#[derive(Default)]
pub struct Periods {
    items: Option<Vec<PeriodRecord>>,
    users: Vec<AdminUser>,
    from: String,
    to: String,
    reason: String,
    form_error: Option<&'static str>,
    adding: bool,
    draft: Option<Draft>,
}

/// Both ends as ISO dates, earliest first, or the reason they cannot be used.
fn normalized(from: &str, to: &str) -> Result<(String, String), &'static str> {
    let (Some(start), Some(end)) = (parse_date_input(from), parse_date_input(to)) else {
        return Err("Укажите даты в формате ДД.ММ.ГГГГ или ГГГГ-ММ-ДД.");
    };
    if start > end {
        return Err("Дата начала должна быть раньше или равна дате окончания.");
    }
    Ok((start.format("%Y-%m-%d").to_string(), end.format("%Y-%m-%d").to_string()))
}

fn user_label(user: &AdminUser) -> String {
    format!("{} @{}", user.title(), user.username)
}

impl Periods {
    pub fn escape(&mut self) {
        self.draft = None;
    }

    pub fn load(&self, ctx: &Ctx) -> Task<Msg> {
        let (periods, users) = (ctx.api.clone(), ctx.api.clone());
        Task::batch([
            Task::perform(async move { periods.periods().await.map_err(text_error) }, Msg::Periods),
            Task::perform(async move { users.admin_users().await.map_err(text_error) }, Msg::Users),
        ])
    }

    fn put(&mut self, record: PeriodRecord) {
        let items = self.items.get_or_insert_default();
        match items.iter_mut().find(|item| item.id == record.id) {
            Some(item) => *item = record,
            None => items.insert(0, record),
        }
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Periods(Ok(items)) => self.items = Some(items),
            Msg::Periods(Err(reason)) => {
                self.items.get_or_insert_default();
                ctx.toast(Tone::Danger, reason);
            }
            Msg::Users(Ok(users)) => self.users = users,
            Msg::Users(Err(reason)) => tracing::warn!("users not loaded: {reason}"),
            Msg::From(value) => self.from = value,
            Msg::To(value) => self.to = value,
            Msg::Reason(value) => self.reason = value,
            Msg::Add => {
                match normalized(&self.from, &self.to) {
                    Err(reason) => self.form_error = Some(reason),
                    Ok((from, to)) => {
                        self.form_error = None;
                        self.adding = true;
                        let api = ctx.api.clone();
                        let reason = Some(self.reason.trim().to_owned()).filter(|text| !text.is_empty());
                        return Task::perform(async move { api.create_period(&from, &to, reason.as_deref()).await.map_err(text_error) }, Msg::Added);
                    }
                }
            }
            Msg::Added(reply) => {
                self.adding = false;
                match reply {
                    Ok(record) => {
                        self.from.clear();
                        self.to.clear();
                        self.reason.clear();
                        self.put(record);
                        ctx.toast(Tone::Success, "Период добавлен");
                    }
                    Err(_) => ctx.toast(Tone::Danger, "Ошибка добавления"),
                }
            }
            Msg::Remove(id) => {
                let api = ctx.api.clone();
                return Task::perform(async move { api.deactivate_period(id).await.map_err(text_error) }, Msg::Removed);
            }
            Msg::Removed(Ok(_)) => {
                ctx.toast(Tone::Success, "Период деактивирован");
                return self.load(ctx);
            }
            Msg::Removed(Err(_)) => ctx.toast(Tone::Danger, "Ошибка деактивации"),
            Msg::Expand(id) => {
                let excluded = self.items.iter().flatten().find(|item| item.id == id).map(|item| item.excluded_user_ids.clone()).unwrap_or_default();
                self.draft = Some(Draft { period: id, excluded, saving: false });
            }
            Msg::Collapse => self.draft = None,
            Msg::Pick(user) => {
                if let Some(draft) = &mut self.draft {
                    match draft.excluded.iter().position(|id| *id == user) {
                        Some(at) => {
                            draft.excluded.remove(at);
                        }
                        None => draft.excluded.push(user),
                    }
                }
            }
            Msg::Clear => {
                if let Some(draft) = &mut self.draft {
                    draft.excluded.clear();
                }
            }
            Msg::SaveExclusions => {
                if let Some(draft) = &mut self.draft {
                    draft.saving = true;
                    let (api, id) = (ctx.api.clone(), draft.period);
                    let changes = json!({ "excluded_user_ids": draft.excluded });
                    return Task::perform(async move { api.update_period(id, &changes).await.map_err(text_error) }, Msg::Saved);
                }
            }
            Msg::Saved(Ok(record)) => {
                self.draft = None;
                self.put(record);
                ctx.toast(Tone::Success, "Исключения обновлены");
            }
            Msg::Saved(Err(_)) => {
                if let Some(draft) = &mut self.draft {
                    draft.saving = false;
                }
                ctx.toast(Tone::Danger, "Ошибка сохранения исключений");
            }
        }
        Task::none()
    }

    fn form(&self) -> El<'_, Msg> {
        let line = row![
            ui::field("Дата начала", ui::input("ДД.ММ.ГГГГ", &self.from).on_input(Msg::From).on_submit(Msg::Add)).width(Length::Fixed(150.0)),
            ui::field("Дата конца", ui::input("ДД.ММ.ГГГГ", &self.to).on_input(Msg::To).on_submit(Msg::Add)).width(Length::Fixed(150.0)),
            ui::field("Причина", ui::input("Например: закрытие квартала", &self.reason).on_input(Msg::Reason).on_submit(Msg::Add)).width(Fill),
            ui::btn_icon(Icon::Plus, "Добавить", Kind::Primary, (!self.adding).then_some(Msg::Add)),
        ]
        .spacing(space::SM)
        .align_y(Alignment::End);
        let mut block = column![line].spacing(space::XS);
        if let Some(error) = self.form_error {
            block = block.push(ui::error_line(error));
        }
        block.into()
    }

    fn draft_view<'a>(&'a self, draft: &'a Draft) -> El<'a, Msg> {
        let mut chips = Row::new().spacing(space::XS);
        for user in &self.users {
            chips = chips.push(ui::chip(user_label(user), draft.excluded.contains(&user.id), Msg::Pick(user.id)));
        }
        let names: Vec<&str> = self.users.iter().filter(|user| draft.excluded.contains(&user.id)).take(4).map(|user| user.title()).collect();
        let mut summary = format!("Текущая выборка: {}", plural_users(draft.excluded.len()));
        if !names.is_empty() {
            summary = format!("{summary} · {}", names.join(", "));
        }
        let actions = row![
            ui::btn(if draft.saving { "Сохраняем" } else { "Сохранить" }, Kind::Primary, (!draft.saving).then_some(Msg::SaveExclusions)),
            ui::btn("Отмена", Kind::Secondary, Some(Msg::Collapse)),
            ui::btn("Очистить", Kind::Ghost, Some(Msg::Clear)),
        ]
        .spacing(space::SM);
        iced::widget::container(column![ui::muted("Выберите пользователей, для которых период не действует:"), chips.wrap(), ui::faint(summary), actions].spacing(space::SM))
            .padding(space::MD)
            .width(Fill)
            .style(theme::inset)
            .into()
    }

    fn line<'a>(&'a self, item: &'a PeriodRecord) -> El<'a, Msg> {
        let excluded: El<'a, Msg> = if item.excluded_user_ids.is_empty() {
            button(ui::muted("нет")).padding(0).style(theme::btn_link).on_press(Msg::Expand(item.id)).into()
        } else {
            button(ui::label(plural_users(item.excluded_user_ids.len()))).padding(0).style(theme::btn_link).on_press(Msg::Expand(item.id)).into()
        };
        let status = if item.is_active { ui::badge("активен", Tone::Warning) } else { ui::badge("снят", Tone::Neutral) };
        let action: El<'a, Msg> = if item.is_active { ui::btn("Снять", Kind::Danger, Some(Msg::Remove(item.id))).into() } else { ui::faint("—").into() };
        row![
            iced::widget::container(ui::faint(format!("#{}", item.id))).width(Length::Fixed(48.0)),
            iced::widget::container(ui::label(format!("{} … {}", item.date_from, item.date_to))).width(FillPortion(3)),
            iced::widget::container(ui::muted(item.reason.clone().unwrap_or_else(|| "—".to_owned()))).width(FillPortion(3)),
            iced::widget::container(excluded).width(FillPortion(1)),
            iced::widget::container(status).width(Length::Fixed(90.0)),
            iced::widget::container(action).width(Length::Fixed(80.0)),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center)
        .into()
    }

    pub fn view<'a>(&'a self, _ctx: &Ctx) -> El<'a, Msg> {
        let hint = "Запрет правок транзакций в указанных диапазонах. Можно исключить отдельных пользователей.";
        let Some(items) = &self.items else {
            return panel("Заблокированные периоды", hint, Space::new(), ui::loading("Загружаем периоды…"));
        };
        let active = items.iter().filter(|item| item.is_active).count();
        let chip = ui::badge(format!("активных · {active} / {}", items.len()), Tone::Neutral);
        let table: El<'a, Msg> = if items.is_empty() {
            ui::empty_state(Icon::Lock, "Периодов пока нет", "Добавьте первый период через форму выше.")
        } else {
            let mut lines = Column::new().spacing(space::SM);
            for item in items {
                lines = lines.push(self.line(item));
                if let Some(draft) = self.draft.as_ref().filter(|draft| draft.period == item.id) {
                    lines = lines.push(self.draft_view(draft));
                }
                lines = lines.push(ui::divider());
            }
            lines.into()
        };
        panel("Заблокированные периоды", hint, chip, column![self.form(), ui::divider(), table].spacing(space::MD))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_read_in_both_notations() {
        assert_eq!(normalized("01.02.2026", "2026-03-31"), Ok(("2026-02-01".to_owned(), "2026-03-31".to_owned())));
    }

    #[test]
    fn a_reversed_range_is_refused() {
        assert_eq!(normalized("2026-03-01", "2026-02-01"), Err("Дата начала должна быть раньше или равна дате окончания."));
        assert!(normalized("2026-02-01", "2026-02-01").is_ok());
    }

    #[test]
    fn nonsense_is_refused() {
        assert!(normalized("завтра", "2026-02-01").is_err());
    }
}
