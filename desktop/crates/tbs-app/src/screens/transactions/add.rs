//! The sheet that records an operation by hand.

use std::time::Duration;

use chrono::{Local, NaiveDateTime};
use iced::widget::{column, row, text_editor};
use iced::{Fill, Task};
use tbs_core::api::NewTransaction;
use tbs_core::format;
use tbs_core::model::Transaction;

use super::edit::{CURRENCIES, TYPES};
use super::{Msg as Screen, Sheet, Transactions};
use crate::ctx::{Ctx, text_error};
use crate::theme::{self, size, space};
use crate::ui::{self, El, Kind};

const WIDTH: f32 = 520.0;
const HIGHLIGHT_FOR: Duration = Duration::from_secs(3);
const SUGGESTIONS: usize = 6;

#[derive(Clone, Debug)]
pub enum Msg {
    When(String),
    Operator(String),
    App(String),
    Amount(String),
    Balance(String),
    Card(String),
    Currency(&'static str),
    Type(&'static str),
    P2p(bool),
    Raw(text_editor::Action),
    Submit,
    Close,
    Created(Result<Transaction, String>),
}

#[derive(Default)]
pub struct Errors {
    when: Option<String>,
    operator: Option<String>,
    amount: Option<String>,
    card: Option<String>,
    form: Option<String>,
}

pub struct AddForm {
    when: String,
    operator: String,
    app: String,
    amount: String,
    balance: String,
    card: String,
    currency: &'static str,
    kind: &'static str,
    p2p: bool,
    raw: text_editor::Content,
    errors: Errors,
    saving: bool,
}

impl AddForm {
    pub fn new() -> Self {
        Self {
            when: Local::now().naive_local().format("%d.%m.%Y %H:%M").to_string(),
            operator: String::new(),
            app: String::new(),
            amount: String::new(),
            balance: String::new(),
            card: String::new(),
            currency: CURRENCIES[0],
            kind: TYPES[0].0,
            p2p: false,
            raw: text_editor::Content::new(),
            errors: Errors::default(),
            saving: false,
        }
    }
}

fn parse_when(input: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(input.trim(), "%d.%m.%Y %H:%M").ok()
}

fn positive(input: &str) -> Option<String> {
    let value: String = input.chars().filter(|c| !c.is_whitespace()).map(|c| if c == ',' { '.' } else { c }).collect();
    value.parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0).map(|_| value)
}

/// Checks the form and builds the request, or marks what is wrong.
///
/// The time is sent as typed: the server keeps the wall clock of the receipt,
/// the same way an edit in the table does.
fn validated(form: &AddForm) -> Result<NewTransaction, Errors> {
    let mut errors = Errors::default();
    let when = parse_when(&form.when);
    if when.is_none() {
        errors.when = Some("Укажите дату и время".into());
    }
    if form.operator.trim().is_empty() {
        errors.operator = Some("Укажите оператора".into());
    }
    let amount = positive(&form.amount);
    if amount.is_none() {
        errors.amount = Some("Сумма должна быть больше нуля".into());
    }
    if form.card.len() != 4 || !form.card.bytes().all(|b| b.is_ascii_digit()) {
        errors.card = Some("Введите ровно 4 цифры карты".into());
    }
    let (Some(when), Some(amount), None, None) = (when, amount, &errors.operator, &errors.card) else { return Err(errors) };
    let optional = |value: &str| Some(value.trim().to_string()).filter(|v| !v.is_empty());
    Ok(NewTransaction {
        datetime: when.format("%Y-%m-%dT%H:%M:%S").to_string(),
        operator: form.operator.trim().to_string(),
        amount,
        card_last4: form.card.clone(),
        transaction_type: form.kind.to_string(),
        currency: form.currency.to_string(),
        app: optional(&form.app),
        balance: positive(&form.balance),
        is_p2p: form.p2p,
        receiver_name: None,
        receiver_card: None,
        raw_text: optional(&form.raw.text()),
    })
}

fn matching<'a>(items: &'a [String], typed: &str) -> Vec<&'a String> {
    let needle = typed.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let hits = items.iter().filter(|item| item.to_lowercase().contains(&needle));
    let hits: Vec<&String> = hits.take(SUGGESTIONS + 1).collect();
    // An exact match needs no help.
    if hits.len() == 1 && hits[0].to_lowercase() == needle { Vec::new() } else { hits.into_iter().take(SUGGESTIONS).collect() }
}

fn suggestions<'a>(items: &'a [String], typed: &str, pick: fn(String) -> Msg) -> Option<El<'a, Screen>> {
    let hits = matching(items, typed);
    if hits.is_empty() {
        return None;
    }
    let mut list = column![].spacing(space::XXS);
    for item in hits {
        list = list.push(
            iced::widget::button(ui::label(item.as_str()))
                .width(Fill)
                .padding([space::XXS, space::SM])
                .style(theme::btn_menu(false))
                .on_press(Screen::Add(pick(item.clone()))),
        );
    }
    Some(iced::widget::container(list).padding(space::XXS).width(Fill).style(theme::inset).into())
}

fn with_error<'a>(control: impl Into<El<'a, Screen>>, error: &'a Option<String>) -> El<'a, Screen> {
    match error {
        Some(reason) => column![control.into(), ui::error_line(reason.as_str())].spacing(space::XS).into(),
        None => control.into(),
    }
}

impl Transactions {
    pub(super) fn add_update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Screen> {
        if let Msg::Created(result) = message {
            return self.created(result, ctx);
        }
        let Sheet::Add(form) = &mut self.sheet else { return Task::none() };
        match message {
            Msg::When(value) => {
                form.when = value;
                form.errors.when = None;
            }
            Msg::Operator(value) => {
                form.operator = value;
                form.errors.operator = None;
            }
            Msg::App(value) => form.app = value,
            Msg::Amount(value) => {
                form.amount = value;
                form.errors.amount = None;
            }
            Msg::Balance(value) => form.balance = value,
            Msg::Card(value) => {
                form.card = value.chars().filter(char::is_ascii_digit).take(4).collect();
                form.errors.card = None;
            }
            Msg::Currency(value) => form.currency = value,
            Msg::Type(value) => form.kind = value,
            Msg::P2p(value) => form.p2p = value,
            Msg::Raw(action) => form.raw.perform(action),
            Msg::Close => self.sheet = Sheet::None,
            Msg::Submit => {
                if form.saving {
                    return Task::none();
                }
                match validated(form) {
                    Ok(request) => {
                        form.saving = true;
                        form.errors = Errors::default();
                        let api = ctx.api.clone();
                        return Task::perform(async move { api.create_transaction(&request).await.map_err(text_error) }, |result| {
                            Screen::Add(Msg::Created(result))
                        });
                    }
                    Err(errors) => form.errors = errors,
                }
            }
            Msg::Created(_) => {}
        }
        Task::none()
    }

    fn created(&mut self, result: Result<Transaction, String>, ctx: &mut Ctx) -> Task<Screen> {
        match result {
            Ok(tx) => {
                self.sheet = Sheet::None;
                let id = tx.id;
                let saved = self.take_new_row(tx, ctx);
                self.reveal_id(id);
                self.highlight = Some(id);
                let fade = Task::perform(tokio::time::sleep(HIGHLIGHT_FOR), move |_| Screen::HighlightOff(id));
                Task::batch([saved, fade])
            }
            Err(reason) => {
                if let Sheet::Add(form) = &mut self.sheet {
                    form.saving = false;
                    form.errors.form = Some(if reason.is_empty() { "Не удалось создать транзакцию".to_string() } else { reason });
                }
                Task::none()
            }
        }
    }

    pub(super) fn add_view<'a>(&'a self, form: &'a AddForm) -> El<'a, Screen> {
        let send = |make: fn(String) -> Msg| move |value: String| Screen::Add(make(value));
        let operator = {
            let mut block = column![with_error(ui::input("Введите или выберите", &form.operator).on_input(send(Msg::Operator)), &form.errors.operator)].spacing(space::XS);
            if let Some(list) = suggestions(&self.operator_options, &form.operator, Msg::Operator) {
                block = block.push(list);
            }
            block
        };
        let app = {
            let mut block = column![ui::input("Введите или выберите", &form.app).on_input(send(Msg::App))].spacing(space::XS);
            if let Some(list) = suggestions(&self.app_options, &form.app, Msg::App) {
                block = block.push(list);
            }
            block
        };
        let kinds = TYPES.iter().fold(row![].spacing(space::XS), |line, (code, name)| line.push(ui::chip(*name, form.kind == *code, Screen::Add(Msg::Type(code)))));
        let currencies = CURRENCIES.iter().fold(row![].spacing(space::XS), |line, code| line.push(ui::chip(*code, form.currency == *code, Screen::Add(Msg::Currency(code)))));
        let raw = text_editor(&form.raw)
            .placeholder("SMS / Telegram сообщение, если есть")
            .height(96)
            .size(size::SM)
            .on_action(|action| Screen::Add(Msg::Raw(action)));

        let mut body = column![
            ui::field("Дата и время", with_error(ui::input("ДД.ММ.ГГГГ ЧЧ:ММ", &form.when).on_input(send(Msg::When)), &form.errors.when)),
            ui::field("Оператор", operator),
            ui::field("Приложение", app),
            row![
                ui::field("Сумма", with_error(ui::input("0.00", &form.amount).on_input(send(Msg::Amount)), &form.errors.amount)).width(Fill),
                ui::field("Остаток", ui::input(format::EMPTY, &form.balance).on_input(send(Msg::Balance))).width(Fill),
            ]
            .spacing(space::MD),
            ui::field("Последние 4 цифры", with_error(ui::input("1234", &form.card).on_input(send(Msg::Card)), &form.errors.card)),
            ui::field("Валюта", currencies),
            ui::field("Тип", kinds),
            ui::check("P2P-перевод", form.p2p, |value| Screen::Add(Msg::P2p(value))),
            ui::field("Исходный текст · опционально", raw),
        ]
        .spacing(space::LG);
        if let Some(reason) = &form.errors.form {
            body = body.push(ui::notice(crate::theme::Tone::Danger, reason.as_str()));
        }

        let submit = (!form.saving).then_some(Screen::Add(Msg::Submit));
        let footer = row![
            ui::fill(),
            ui::btn("Отмена", Kind::Secondary, Some(Screen::Add(Msg::Close))),
            ui::btn(if form.saving { "Сохраняем" } else { "Создать" }, Kind::Primary, submit),
        ]
        .spacing(space::SM);
        let panel = ui::sheet_panel(
            "Новая запись",
            "Добавить транзакцию",
            "Запись будет помечена как ручной источник.",
            Screen::Add(Msg::Close),
            body,
            Some(footer.into()),
        );
        ui::sheet(iced::widget::Space::new(), panel, WIDTH, Some(Screen::Add(Msg::Close)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled() -> AddForm {
        AddForm {
            when: "05.03.2026 14:30".into(),
            operator: " Korzinka ".into(),
            amount: "1 500,50".into(),
            card: "1234".into(),
            ..AddForm::new()
        }
    }

    #[test]
    fn a_complete_form_becomes_a_request() {
        let request = validated(&filled()).ok().unwrap();
        assert_eq!(request.datetime, "2026-03-05T14:30:00");
        assert_eq!(request.operator, "Korzinka");
        assert_eq!(request.amount, "1500.50");
        assert_eq!(request.transaction_type, "DEBIT");
        assert_eq!(request.currency, "UZS");
        assert_eq!(request.app, None);
        assert_eq!(request.raw_text, None);
    }

    #[test]
    fn every_required_field_is_named() {
        let form = AddForm { when: "завтра".into(), ..AddForm::new() };
        let errors = validated(&form).err().unwrap();
        assert!(errors.when.is_some());
        assert!(errors.operator.is_some());
        assert!(errors.amount.is_some());
        assert!(errors.card.is_some());
    }

    #[test]
    fn zero_is_not_an_amount() {
        let form = AddForm { amount: "0".into(), ..filled() };
        assert!(validated(&form).err().unwrap().amount.is_some());
    }

    #[test]
    fn suggestions_hide_once_the_name_is_complete() {
        let items = vec!["Korzinka".to_string(), "Korzinka Go".to_string(), "Makro".to_string()];
        assert_eq!(matching(&items, "korz").len(), 2);
        assert!(matching(&items, "makro").is_empty());
        assert!(matching(&items, "").is_empty());
    }
}
