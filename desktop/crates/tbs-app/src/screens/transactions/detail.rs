//! The sheet with one operation: the original message and what was read from it.

use iced::widget::{column, container, grid, row, text};
use iced::{Fill, Task};
use tbs_core::format;
use tbs_core::model::Transaction;
use tbs_core::query::Row;

use super::{Msg as Screen, Sheet, Transactions};
use crate::ctx::{Ctx, text_error};
use crate::theme::{self, size, space};
use crate::ui::{self, El};

const WIDTH: f32 = 520.0;

#[derive(Clone, Debug)]
pub enum Msg {
    Close,
    Loaded(i64, Result<Transaction, String>),
}

pub struct Detail {
    pub id: i64,
    pub raw: Option<String>,
    pub loading: bool,
}

fn method_line(tx: &Transaction) -> String {
    let method = tx.parsing_method.as_deref().filter(|m| !m.is_empty()).unwrap_or(format::EMPTY);
    match tx.parsing_confidence {
        Some(confidence) => format!("Метод · {method} · Уверенность · {}%", (confidence * 100.0).round()),
        None => format!("Метод · {method}"),
    }
}

fn card_line(tx: &Transaction) -> String {
    tx.card_last_4.as_deref().filter(|c| !c.is_empty()).map_or(format::EMPTY.to_string(), |c| format!("•••• {c}"))
}

fn value(raw: Option<&str>) -> String {
    raw.filter(|v| !v.is_empty()).unwrap_or(format::EMPTY).to_string()
}

fn pair<'a>(name: &'a str, content: String) -> El<'a, Screen> {
    column![ui::faint(name), ui::label(content)].spacing(space::XXS).into()
}

fn line<'a>(name: &'a str, content: String) -> El<'a, Screen> {
    row![container(ui::muted(name)).width(120), ui::label(content)].spacing(space::SM).into()
}

impl Transactions {
    pub(super) fn open_detail(&mut self, id: i64, ctx: &Ctx) -> Task<Screen> {
        let Some(row) = self.data.by_id(id) else { return Task::none() };
        let raw = row.tx.raw_message.clone().filter(|m| !m.is_empty());
        let loading = raw.is_none();
        self.sheet = Sheet::Detail(Detail { id, raw, loading });
        if !loading {
            return Task::none();
        }
        let api = ctx.api.clone();
        Task::perform(async move { api.transaction(id).await.map_err(text_error) }, move |result| Screen::Detail(Msg::Loaded(id, result)))
    }

    pub(super) fn detail_update(&mut self, message: Msg) {
        match message {
            Msg::Close => self.sheet = Sheet::None,
            Msg::Loaded(id, result) => {
                if let Sheet::Detail(detail) = &mut self.sheet
                    && detail.id == id
                {
                    detail.loading = false;
                    detail.raw = result.ok().and_then(|tx| tx.raw_message).filter(|m| !m.is_empty());
                }
            }
        }
    }

    pub(super) fn detail_view<'a>(&'a self, detail: &'a Detail, row: &'a Row) -> El<'a, Screen> {
        let tx = &row.tx;
        let money = format!("{} {}", format::amount(tx.amount.as_deref()), value(tx.currency.as_deref()));
        let source = tbs_core::columns::source_label(tx);
        let headline = tx.application_mapped.as_deref().or(tx.operator_raw.as_deref());

        let summary = column![
            ui::faint("Сумма"),
            text(money).size(size::XXL).font(theme::FONT_BOLD),
            ui::muted(value(headline)),
        ]
        .spacing(space::XS);

        let meta = grid![
            pair("Дата", format::datetime(row.when)),
            pair("Тип", value(tx.transaction_type_display.as_deref().or(tx.transaction_type.as_deref()))),
            pair("Карта", card_line(tx)),
            pair("Источник", value(Some(source.as_str()))),
        ]
        .columns(2)
        .spacing(space::MD)
        .height(iced::Shrink);

        let raw_text = if detail.loading { "Загрузка…" } else { detail.raw.as_deref().unwrap_or(format::EMPTY) };
        let raw = container(text(raw_text).size(size::SM).font(theme::FONT_MONO)).padding(space::MD).width(Fill).style(theme::inset);

        let parsed = column![
            line("Оператор", value(tx.operator_raw.as_deref())),
            line("Приложение", value(tx.application_mapped.as_deref())),
            line("Валюта", value(tx.currency.as_deref())),
            line("Остаток", tx.balance_after.as_deref().map_or(format::EMPTY.to_string(), |b| format::amount(Some(b)))),
            line("P2P", if tx.is_p2p { "да" } else { "нет" }.to_string()),
        ]
        .spacing(space::SM);

        let body = column![summary, meta, ui::section("Исходный текст", raw), ui::section("Распарсенные поля", parsed)].spacing(space::XL);
        let panel = ui::sheet_panel(format!("Транзакция · #{}", tx.id), "Детали операции", method_line(tx), Screen::Detail(Msg::Close), body, None);
        ui::sheet(iced::widget::Space::new(), panel, WIDTH, Some(Screen::Detail(Msg::Close)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_line_reads_without_a_score() {
        let tx = Transaction { parsing_method: Some("regex".into()), parsing_confidence: Some(0.87), ..Default::default() };
        assert_eq!(method_line(&tx), "Метод · regex · Уверенность · 87%");
        assert_eq!(method_line(&Transaction::default()), format!("Метод · {}", format::EMPTY));
    }

    #[test]
    fn card_is_masked() {
        let tx = Transaction { card_last_4: Some("1234".into()), ..Default::default() };
        assert_eq!(card_line(&tx), "•••• 1234");
        assert_eq!(card_line(&Transaction::default()), format::EMPTY);
    }
}
