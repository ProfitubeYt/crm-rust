//! The list of chats on the left: its tabs, its search and its lines.

use chrono::NaiveDateTime;
use iced::widget::{Column, button, column, container, mouse_area, row, text};
use iced::{Alignment, Fill};
use tbs_core::api::TgChat;

use super::client::{Client, Filter, Msg, Target};
use super::{dates, style};
use crate::theme::{self, Tone, control, size, space};
use crate::ui::{self, El, Icon};

pub const WIDTH: f32 = 320.0;

pub fn type_icon(kind: &str) -> Option<Icon> {
    match kind {
        "bot" => Some(Icon::Bot),
        "user" => Some(Icon::User),
        "channel" => Some(Icon::Megaphone),
        "group" | "supergroup" => Some(Icon::Users),
        _ => None,
    }
}

/// Initials on a colour of the chat's own; nobody's picture is downloaded.
pub fn avatar<'a, M: 'a>(name: &str, seed: i64, px: f32) -> El<'a, M> {
    container(text(dates::initials(name)).size(px * 0.36).font(theme::FONT_MEDIUM)).center_x(px).center_y(px).style(style::avatar(seed)).into()
}

/// One line of what the chat said last.
fn preview(chat: &TgChat) -> String {
    let Some(last) = &chat.last_message else { return String::new() };
    if let Some(words) = last.text.as_deref().filter(|words| !words.is_empty()) {
        return words.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect();
    }
    if last.photo.is_some() {
        return "Фото".to_owned();
    }
    match &last.document {
        Some(file) => file.file_name.clone().filter(|name| !name.is_empty()).unwrap_or_else(|| "Документ".to_owned()),
        None => String::new(),
    }
}

fn line<'a>(client: &'a Client, chat: &'a TgChat, now: NaiveDateTime) -> El<'a, Msg> {
    let hidden = client.hidden(chat);
    let mut top = row![].spacing(space::XS).align_y(Alignment::Center);
    if let Some(glyph) = type_icon(&chat.chat_type) {
        top = top.push(ui::icon(glyph, 12.0));
    }
    top = top.push(container(text(chat.title.as_str()).size(size::BASE).font(theme::FONT_MEDIUM).wrapping(text::Wrapping::None)).width(Fill).clip(true));
    if hidden {
        top = top.push(ui::icon(Icon::EyeOff, 12.0));
    }
    if chat.is_protected {
        top = top.push(ui::icon(Icon::Lock, 12.0));
    }
    if chat.is_monitored {
        top = top.push(ui::icon_tone(Icon::Live, 12.0, if chat.monitor_enabled { Tone::Success } else { Tone::Neutral }));
    }
    let seen = chat.last_message.as_ref().and_then(|last| last.date.as_deref()).or(chat.latest_message_date.as_deref());
    let time = dates::list_time(seen, now);
    if !time.is_empty() {
        top = top.push(ui::faint(time));
    }

    let mut bottom = row![container(ui::muted(preview(chat)).wrapping(text::Wrapping::None)).width(Fill).height(size::SM * 1.4).clip(true)].spacing(space::XS).align_y(Alignment::Center);
    // How far the worker is behind is shown where hidden chats are listed.
    let lag = chat.backlog_lag_messages.unwrap_or(0);
    if lag > 0 && hidden && client.filter != Filter::Active {
        bottom = bottom.push(ui::badge(if lag > 99 { "99+".to_owned() } else { lag.to_string() }, Tone::Accent));
    }

    let body = row![avatar(&chat.title, chat.chat_id, 40.0), column![top, bottom].spacing(space::XXS).width(Fill)].spacing(space::SM).align_y(Alignment::Center);
    let open = button(body).width(Fill).padding(space::SM).style(style::chat(client.active == Some(chat.chat_id))).on_press(Msg::Open(chat.chat_id));
    mouse_area(open).on_right_press(Msg::Context(Target::Chat(chat.chat_id))).into()
}

fn nothing<'a>(glyph: Option<Icon>, head: &'a str, hint: &'a str) -> El<'a, Msg> {
    let mut block = Column::new().spacing(space::XS).align_x(Alignment::Center);
    if let Some(glyph) = glyph {
        block = block.push(ui::icon(glyph, 28.0));
    }
    block = block.push(ui::strong(head));
    if !hint.is_empty() {
        block = block.push(ui::muted(hint).center());
    }
    container(block).width(Fill).padding([space::XL, space::LG]).center_x(Fill).into()
}

pub fn view(client: &Client, now: NaiveDateTime) -> El<'_, Msg> {
    let (tone, word) = if client.ready() { (Tone::Success, "подключено") } else { (Tone::Warning, "подключение…") };
    let state = container(row![ui::status_dot(tone), text(word).size(size::XS).font(theme::FONT_MEDIUM)].spacing(space::XS).align_y(Alignment::Center))
        .padding([2.0, space::SM])
        .style(theme::pill(tone));
    let head = row![ui::heading("Чаты"), ui::fill(), state].align_y(Alignment::Center);

    let (all, active, hidden) = client.counts();
    let tab = |filter: Filter, caption: &'static str, count: usize| {
        let mut words = row![text(caption).size(size::SM).font(theme::FONT_MEDIUM)].spacing(space::XS).align_y(Alignment::Center);
        if count > 0 {
            words = words.push(text(count.to_string()).size(size::XS));
        }
        button(container(words).center_x(Fill).center_y(Fill))
            .width(Fill)
            .height(control::SM - 4.0)
            .padding(0)
            .style(theme::btn_segment(client.filter == filter))
            .on_press(Msg::Filter(filter))
    };
    let tabs = container(row![tab(Filter::Active, "Активные", active), tab(Filter::All, "Все", all), tab(Filter::Hidden, "Скрытые", hidden)].spacing(space::XXS))
        .padding(space::XXS)
        .style(theme::inset);

    let top = column![head, ui::input("Поиск по чатам...", &client.query).on_input(Msg::Query), tabs].spacing(space::SM).padding([space::MD, space::MD]);

    let listed = client.listed();
    let body: El<'_, Msg> = if !client.chats_loaded {
        match &client.chats_error {
            Some(reason) => nothing(Some(Icon::Alert), "Не удалось загрузить чаты", reason),
            None => nothing(None, "Загружаем чаты…", ""),
        }
    } else if !listed.is_empty() {
        let lines = listed.into_iter().fold(Column::new().spacing(space::XXS).padding([0.0, space::SM]), |lines, chat| lines.push(line(client, chat, now)));
        ui::scroll(lines).height(Fill).into()
    } else if !client.query.trim().is_empty() {
        nothing(None, "Ничего не найдено", "Попробуйте другой запрос.")
    } else if client.filter == Filter::Hidden {
        nothing(None, "Скрытых чатов нет", "Используйте контекстное меню, чтобы скрыть чат.")
    } else {
        nothing(Some(Icon::Chat), "Чатов нет", "Подождите, идёт загрузка из Telegram.")
    };

    container(column![top, body]).width(Fill).height(Fill).style(style::list_pane).into()
}
