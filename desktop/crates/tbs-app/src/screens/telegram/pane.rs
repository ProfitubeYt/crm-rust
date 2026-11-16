//! The open chat: its header, its messages and the line a message is typed in.

use chrono::NaiveDate;
use iced::widget::text_editor::{Binding, KeyPress};
use iced::widget::{Column, Row, Space, button, center, column, container, image, mouse_area, progress_bar, row, stack, svg, text, text_editor};
use iced::keyboard::{Key, key::Named};
use iced::{Alignment, ContentFit, Fill, Length, Padding, Shrink};
use tbs_core::api::{TgChat, TgFile, TgMarkup, TgMessage};
use tbs_core::format::plural;

use super::client::{Client, Msg, Photo, STREAM, Target};
use super::list::{avatar, type_icon};
use super::style::{self, Work};
use super::dates;
use crate::theme::{self, Tone, control, size, space};
use crate::ui::{self, El, Icon, Kind};

/// Wider than this a bubble is harder to read than to scroll.
const BUBBLE: f32 = 520.0;
const PHOTO: f32 = 320.0;
pub const HEADER: f32 = 56.0;

fn type_label(chat: &TgChat) -> String {
    let members = chat.member_count.filter(|count| *count > 0);
    match (chat.chat_type.as_str(), members) {
        ("bot", _) => "бот".to_owned(),
        ("user", _) => "пользователь".to_owned(),
        ("channel", Some(count)) => format!("канал · {count} подписчиков"),
        ("channel", None) => "канал".to_owned(),
        ("group" | "supergroup", Some(count)) => format!("группа · {count} участников"),
        ("group" | "supergroup", None) => "группа".to_owned(),
        (other, _) => other.to_owned(),
    }
}

/// A square button of the header that stays lit while what it stands for is on.
fn tool<'a>(glyph: Icon, hint: &'a str, lit: bool, on_press: Msg) -> El<'a, Msg> {
    let image: El<'a, Msg> = if lit { ui::icon_tone(glyph, 16.0, Tone::Accent).into() } else { ui::icon(glyph, 16.0).into() };
    let pressed = button(center(image)).width(control::MD).height(control::MD).padding(0).style(theme::btn_chip(lit)).on_press(on_press);
    ui::hint_before(pressed, hint)
}

fn header<'a>(client: &'a Client, chat: &'a TgChat, narrow: bool, today: NaiveDate) -> El<'a, Msg> {
    let mut name = row![text(chat.title.as_str()).size(size::MD).font(theme::FONT_BOLD).wrapping(text::Wrapping::None)].spacing(space::XS).align_y(Alignment::Center);
    if chat.is_protected {
        name = name.push(ui::icon(Icon::Lock, 12.0));
    }
    let mut about = row![].spacing(space::XS).align_y(Alignment::Center);
    if let Some(glyph) = type_icon(&chat.chat_type) {
        about = about.push(ui::icon(glyph, 12.0));
    }
    let handle = chat.username.as_deref().filter(|name| !name.is_empty()).map(|name| format!(" · @{name}")).unwrap_or_default();
    about = about.push(ui::muted(format!("{}{handle}", type_label(chat))).wrapping(text::Wrapping::None));

    let mut line = Row::new().spacing(space::SM).align_y(Alignment::Center);
    if narrow {
        line = line.push(ui::icon_btn(Icon::ArrowLeft, "Назад к списку чатов", Some(Msg::Back)));
    }
    line = line.push(avatar(&chat.title, chat.chat_id, 32.0));
    line = line.push(container(column![name, about].spacing(space::XXS)).width(Fill).clip(true));

    if let Some(app) = client.menu_app(chat.chat_id) {
        let caption = app.text.clone().filter(|caption| !caption.is_empty()).unwrap_or_else(|| "Открыть приложение".to_owned());
        line = line.push(ui::btn_icon(Icon::AppWindow, caption, Kind::Secondary, Some(Msg::MenuApp)));
    }
    if client.range.is_set() {
        line = line.push(ui::btn_icon(Icon::Calendar, dates::range_label(client.range, today), Kind::Secondary, Some(Msg::TogglePeriod)));
        line = line.push(ui::icon_btn(Icon::Close, "Сбросить период", Some(Msg::ClearRange)));
    } else {
        line = line.push(tool(Icon::Calendar, "Фильтр по дате", client.period.is_some(), Msg::TogglePeriod));
    }
    let (glyph, hint) = if client.selecting { (Icon::Close, "Выйти из режима выбора") } else { (Icon::CheckSquare, "Выбрать несколько") };
    line = line.push(tool(glyph, hint, client.selecting, Msg::ToggleSelecting));
    let watched = chat.monitor_enabled;
    let (glyph, hint) = if watched { (Icon::Eye, "Мониторинг включён") } else { (Icon::EyeOff, "Мониторинг выключен") };
    line = line.push(tool(glyph, hint, watched, Msg::Monitor(chat.chat_id, !watched)));

    container(line).height(HEADER).padding([0.0, space::LG]).center_y(HEADER).width(Fill).style(style::bar).into()
}

fn bulk_bar(client: &Client) -> El<'_, Msg> {
    let (selected, total) = (client.selected.len(), client.selectable());
    let all = selected > 0 && selected == total;
    let busy = client.bulk_busy();
    let (processed, failed) = client.bulk.as_ref().map_or((0, 0), |bulk| (bulk.processed, bulk.failed));
    let of = if busy { (processed + failed + client.in_flight()).max(selected) } else { selected };
    let share = if of > 0 { (processed + failed) as f32 / of as f32 } else { 0.0 };

    let count = row![ui::strong(selected.to_string()), ui::label(plural(selected, "сообщение", "сообщения", "сообщений"))].spacing(space::XS).align_y(Alignment::Center);
    let (glyph, caption, pick) = if all { (Icon::Square, "Снять".to_owned(), Msg::SelectNone) } else { (Icon::Multiselect, format!("Выбрать все · {total}"), Msg::SelectAll) };
    let mut line = row![
        ui::icon_btn(Icon::Close, "Выйти из режима выбора (Esc)", (!busy).then_some(Msg::CancelSelection)),
        count,
        ui::hint_on(ui::btn_icon(glyph, caption, Kind::Ghost, (total > 0).then_some(pick)), if all { "Снять выделение (Ctrl+D)" } else { "Выбрать все (Ctrl+A)" }),
        ui::fill(),
    ]
    .spacing(space::SM)
    .align_y(Alignment::Center);
    if busy {
        let mut words = format!("{processed} из {of}");
        if failed > 0 {
            words.push_str(&format!(" · ошибок {failed}"));
        }
        line = line.push(ui::muted(words));
    }
    let action = if busy { "Обработка…".to_owned() } else { format!("В обработку · {selected}") };
    line = line.push(ui::btn_icon(Icon::Scan, action, Kind::Primary, (!busy && selected > 0).then_some(Msg::BulkStart)));

    let bar = container(line).padding([space::SM, space::LG]).width(Fill).style(style::bar);
    column![bar, progress_bar(0.0..=1.0, share).girth(2.0).style(theme::progress)].into()
}

fn tick<'a>(checked: bool) -> El<'a, Msg> {
    let mark: El<'a, Msg> = if checked { ui::icon(Icon::Check, 12.0).style(|theme: &iced::Theme, _| svg::Style { color: Some(theme::p(theme).on_accent) }).into() } else { Space::new().into() };
    container(mark).center_x(20.0).center_y(20.0).style(style::tick(checked)).into()
}

/// The room a photo takes: its own shape, scaled into a square of [`PHOTO`].
fn photo_room(file: &TgFile) -> (f32, f32) {
    match (file.width.filter(|w| *w > 0), file.height.filter(|h| *h > 0)) {
        (Some(width), Some(height)) => {
            let (width, height) = (width as f32, height as f32);
            let scale = (PHOTO / width).min(PHOTO / height).min(1.0);
            ((width * scale).round().max(48.0), (height * scale).round().max(48.0))
        }
        _ => (240.0, 160.0),
    }
}

fn photo<'a>(client: &'a Client, chat: i64, message: i64, file: &'a TgFile) -> El<'a, Msg> {
    let (width, height) = photo_room(file);
    let key = file.file_id.map(|file| (chat, message, file));
    match key.and_then(|key| client.photos.get(&key).map(|photo| (key, photo))) {
        Some((key, Photo::Ready(handle))) => {
            let picture = image(handle.clone()).width(width).height(height).content_fit(ContentFit::Contain);
            ui::hint_on(button(picture).padding(0).style(style::plain).on_press(Msg::ViewPhoto(key)), "Открыть фото")
        }
        found => {
            let words = if matches!(found, Some((_, Photo::Failed))) { "Фото не загружено" } else { "Фото" };
            container(column![ui::icon(Icon::Image, 20.0), ui::faint(words)].spacing(space::XS).align_x(Alignment::Center)).center_x(width).center_y(height).style(style::photo_wait).into()
        }
    }
}

fn document<'a>(client: &'a Client, message: i64, file: &'a TgFile) -> El<'a, Msg> {
    let name = file.file_name.as_deref().filter(|name| !name.is_empty()).unwrap_or("Документ");
    let mut about = dates::size_label(file.size);
    if let Some(kind) = file.mime_type.as_deref().filter(|kind| !kind.is_empty()) {
        if !about.is_empty() {
            about.push_str(" · ");
        }
        about.push_str(kind);
    }
    let saving = client.saving == Some(message);
    let body = row![
        container(ui::icon(Icon::Document, 16.0)).center_x(32.0).center_y(32.0).style(style::file_mark),
        column![text(name).size(size::BASE).font(theme::FONT_MEDIUM), ui::faint(about)].spacing(space::XXS).width(Fill),
        ui::icon(if saving { Icon::Loader } else { Icon::Download }, 14.0),
    ]
    .spacing(space::SM)
    .align_y(Alignment::Center);
    let ready = message != 0 && file.file_id.is_some() && !saving;
    ui::hint_on(button(body).width(Fill).padding(space::XS).style(style::plain).on_press_maybe(ready.then_some(Msg::Document(message))), "Скачать файл")
}

fn keyboard<'a>(client: &'a Client, message: i64, markup: &'a TgMarkup) -> El<'a, Msg> {
    let mut lines = Column::new().spacing(space::XS);
    for (at_row, keys) in markup.rows.iter().enumerate() {
        let mut line = Row::new().spacing(space::XS);
        for (at_col, key) in keys.iter().enumerate() {
            let has_url = key.url.as_deref().is_some_and(|url| !url.is_empty());
            let caption = text(key.text.as_str()).size(size::SM).font(theme::FONT_MEDIUM);
            let press = Msg::Key(message, at_row, at_col);
            let (body, on_press, app, hint): (El<'a, Msg>, Option<Msg>, bool, &'a str) = match key.kind.as_str() {
                "web_app" => (
                    row![ui::icon_tone(Icon::AppWindow, 12.0, Tone::Accent), caption].spacing(space::XS).align_y(Alignment::Center).into(),
                    has_url.then_some(press),
                    true,
                    "Открыть мини-приложение",
                ),
                "url" | "login_url" => (row![caption, ui::icon(Icon::Open, 12.0)].spacing(space::XS).align_y(Alignment::Center).into(), has_url.then_some(press), false, "Открыть ссылку"),
                "copy_text" => {
                    let copied = client.copied == Some((message, at_row, at_col));
                    (row![caption, ui::icon(if copied { Icon::Check } else { Icon::Copy }, 12.0)].spacing(space::XS).align_y(Alignment::Center).into(), Some(press), false, "Скопировать")
                }
                // The bot would have to answer these; the server has no way to ask it yet.
                _ => (caption.into(), None, false, "Эта кнопка требует ответа от бота и пока не поддерживается"),
            };
            let pressed = button(container(body).center_x(Fill)).width(Fill).padding([space::XS + 1.0, space::SM]).style(style::key(app)).on_press_maybe(on_press);
            line = line.push(container(ui::hint_on(pressed, hint)).width(Fill));
        }
        lines = lines.push(line);
    }
    lines.into()
}

fn work_button<'a>(state: Work, message: i64) -> El<'a, Msg> {
    let (glyph, caption, hint) = match state {
        Work::Busy => (Icon::Loader, "…", "Обработка…"),
        Work::Failed => (Icon::Refresh, "Повторить", "Ошибка обработки — нажмите чтобы повторить"),
        Work::Done => (Icon::Check, "Готово", "Уже обработано"),
        Work::Idle => (Icon::Scan, "Обработать", "Отправить в обработку"),
    };
    let tone = match state {
        Work::Done => Tone::Success,
        Work::Failed => Tone::Danger,
        _ => Tone::Neutral,
    };
    let body = row![ui::icon_tone(glyph, 14.0, tone), text(caption).size(size::XS).font(theme::FONT_MEDIUM)].spacing(space::XS).align_y(Alignment::Center);
    let pressed = button(container(body).center_y(Fill)).height(control::SM - 2.0).padding([0.0, space::SM]).style(style::work(state)).on_press_maybe((state != Work::Busy).then_some(Msg::Process(message)));
    ui::hint_on(pressed, hint)
}

fn bubble<'a>(client: &'a Client, chat: i64, message: &'a TgMessage) -> El<'a, Msg> {
    let id = message.id.unwrap_or(0);
    let state = client.work_of(chat, message);
    // The time sits at the right edge of a bubble that is only as wide as
    // what it holds. A strut of the width of that line keeps a one-word
    // message from being narrower than its own time.
    let room = 36.0 + if state == Work::Done { 86.0 } else { 0.0 } + if message.is_outgoing { 16.0 } else { 0.0 };
    let mut inside = Column::new().spacing(space::XS).push(Space::new().width(room));
    if let Some(file) = &message.photo {
        inside = inside.push(photo(client, chat, id, file));
    }
    if let Some(file) = &message.document {
        inside = inside.push(document(client, id, file));
    }
    if let Some(words) = message.text.as_deref().filter(|words| !words.is_empty()) {
        // A long address has no spaces to break at.
        inside = inside.push(text(words).size(size::BASE).wrapping(text::Wrapping::WordOrGlyph));
    }
    if let Some(markup) = message.reply_markup.as_ref().filter(|markup| !markup.rows.is_empty()) {
        inside = inside.push(keyboard(client, id, markup));
    }
    let mut meta = Row::new().spacing(space::XS).align_y(Alignment::Center);
    if state == Work::Done {
        meta = meta.push(ui::badge("обработано", Tone::Success));
    }
    meta = meta.push(ui::faint(dates::clock(message.date.as_deref())));
    if message.is_outgoing {
        meta = meta.push(ui::hint_on(ui::icon(Icon::Check, 12.0), "Отправлено"));
    }
    inside = inside.push(container(meta).align_right(Fill));

    let selected = client.selected.contains(&id);
    let has_file = message.photo.is_some() || message.document.is_some() || message.reply_markup.is_some();
    let width = if has_file { Length::Fixed(PHOTO + 2.0 * space::MD) } else { Shrink };
    // The strut is a line of no height: the gap after it is the top padding.
    let padding = Padding { top: space::SM - space::XS, right: space::MD, bottom: space::SM, left: space::MD };
    container(inside).padding(padding).width(width).max_width(BUBBLE).style(style::bubble(message.is_outgoing, selected)).into()
}

fn message_row<'a>(client: &'a Client, chat: i64, message: &'a TgMessage) -> El<'a, Msg> {
    let id = message.id.unwrap_or(0);
    let state = client.work_of(chat, message);
    let mut line = Row::new().spacing(space::SM).align_y(Alignment::End);
    if client.selecting {
        line = line.push(tick(client.selected.contains(&id)));
    }
    let process = (!client.selecting && id != 0).then(|| work_button(state, id));
    if message.is_outgoing {
        line = line.push(ui::fill());
        line = line.extend(process);
        line = line.push(bubble(client, chat, message));
    } else {
        line = line.push(bubble(client, chat, message));
        line = line.extend(process);
        line = line.push(ui::fill());
    }
    let marked = client.selecting && client.selected.contains(&id);
    let body = container(line).width(Fill).padding([space::XXS, space::XS]).style(style::row_mark(marked));
    if id == 0 {
        return body.into();
    }
    mouse_area(body).on_press(Msg::RowPress(id)).on_right_press(Msg::Context(Target::Message(id))).into()
}

fn note<'a>(glyph: Option<Icon>, head: &'a str, hint: &'a str) -> El<'a, Msg> {
    let mut block = Column::new().spacing(space::SM).align_x(Alignment::Center);
    if let Some(glyph) = glyph {
        block = block.push(ui::icon(glyph, 28.0));
    }
    block = block.push(ui::strong(head));
    if !hint.is_empty() {
        block = block.push(ui::muted(hint).center());
    }
    center(block).into()
}

fn lock_form(client: &Client) -> El<'_, Msg> {
    let Some(form) = &client.lock else { return note(None, "", "") };
    let can = !form.busy && !form.password.trim().is_empty();
    let mut block = column![
        row![ui::icon_tone(Icon::Lock, 16.0, Tone::Warning), ui::strong("Введите пароль")].spacing(space::SM).align_y(Alignment::Center),
        ui::muted("Этот чат защищён. Введите пароль, чтобы открыть его содержимое."),
        row![
            ui::secret_input("Пароль", &form.password).on_input(Msg::LockInput).on_submit(Msg::Unlock).width(Fill),
            ui::btn(if form.busy { "Проверка..." } else { "Открыть" }, Kind::Primary, can.then_some(Msg::Unlock)),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center),
    ]
    .spacing(space::MD);
    if let Some(reason) = &form.error {
        block = block.push(ui::error_line(reason.as_str()));
    }
    center(ui::card(block).max_width(440)).padding(space::LG).into()
}

fn stream<'a>(client: &'a Client, chat: i64, today: NaiveDate) -> El<'a, Msg> {
    if client.lock.is_some() {
        return lock_form(client);
    }
    if !client.messages_loaded {
        return match &client.messages_error {
            Some(reason) => note(Some(Icon::Alert), "Не удалось загрузить сообщения", reason),
            None => note(None, "Загружаем сообщения…", ""),
        };
    }
    let shown = client.shown();
    if shown.is_empty() {
        return if client.range.is_set() {
            note(Some(Icon::Calendar), "В выбранном периоде сообщений нет", "Попробуйте расширить диапазон дат.")
        } else {
            note(Some(Icon::Chat), "Сообщений ещё нет", "Напишите первое сообщение в этом чате.")
        };
    }
    let mut list = Column::new().spacing(space::XXS).padding([space::MD, space::LG]).width(Fill);
    let mut day_before: Option<NaiveDate> = None;
    for message in shown {
        let day = message.date.as_deref().and_then(dates::local).map(|moment| moment.date());
        if let Some(day) = day.filter(|day| day_before != Some(*day)) {
            let mark = container(text(dates::day_label(day, today)).size(size::XS).font(theme::FONT_MEDIUM)).padding([2.0, space::MD]).style(style::day_mark);
            list = list.push(container(mark).center_x(Fill).padding([space::SM, 0.0]));
            day_before = Some(day);
        }
        list = list.push(message_row(client, chat, message));
    }
    // Anchored at the bottom: a chat opens at its newest message and stays there.
    let scroller = ui::scroll(list).id(STREAM).anchor_bottom().on_scroll(|view| Msg::Scrolled(view.absolute_offset().y)).width(Fill).height(Fill);
    if !client.away {
        return scroller.into();
    }
    let down = button(center(ui::icon(Icon::ChevronDown, 18.0))).width(40.0).height(40.0).padding(0).style(style::fab).on_press(Msg::ToBottom);
    stack![scroller, container(ui::hint_before(down, "Прокрутить вниз")).width(Fill).height(Fill).align_right(Fill).align_bottom(Fill).padding(space::LG)].into()
}

fn composer<'a>(client: &'a Client, chat: &'a TgChat) -> El<'a, Msg> {
    let sending = client.sending();
    let can_send = !sending && !client.draft.text().trim().is_empty();
    let editor = text_editor(&client.draft)
        .placeholder(format!("Сообщение в «{}»…", chat.title))
        .size(size::BASE)
        .padding([6.5, 10.0])
        // One line of text; with the padding it is as tall as the buttons beside it.
        .min_height(control::MD - 13.0)
        .max_height(160.0)
        .style(style::composer)
        .on_action(Msg::Draft)
        // Enter sends; with Shift it starts a new line.
        .key_binding(|press: KeyPress| {
            let sends = matches!(press.key, Key::Named(Named::Enter)) && !press.modifiers.shift();
            if sends { Some(Binding::Custom(Msg::Send)) } else { Binding::from_key_press(press) }
        });
    let send_glyph = ui::icon(if sending { Icon::Loader } else { Icon::Send }, 16.0)
        .style(move |theme: &iced::Theme, _| svg::Style { color: Some(if can_send { theme::p(theme).on_accent } else { theme::p(theme).text3 }) });
    let send = button(center(send_glyph)).width(control::MD).height(control::MD).padding(0).style(theme::btn_primary).on_press_maybe(can_send.then_some(Msg::Send));
    let line = row![
        ui::icon_btn(Icon::Attach, "Прикрепить файл", (!sending).then_some(Msg::Attach)),
        container(editor).width(Fill),
        ui::hint_before(send, "Отправить сообщение"),
    ]
    .spacing(space::SM)
    .align_y(Alignment::End);
    container(line).padding([space::SM, space::LG]).width(Fill).style(style::bar).into()
}

/// The right side while no chat is open.
pub fn empty<'a>() -> El<'a, Msg> {
    note(Some(Icon::Chat), "Выберите чат", "Нажмите на чат в списке слева, чтобы посмотреть сообщения.")
}

pub fn view<'a>(client: &'a Client, chat: &'a TgChat, narrow: bool, today: NaiveDate) -> El<'a, Msg> {
    let mut pane = Column::new().push(header(client, chat, narrow, today)).push(ui::divider());
    if client.selecting {
        pane = pane.push(bulk_bar(client));
    }
    pane = pane.push(container(stream(client, chat.chat_id, today)).width(Fill).height(Fill));
    if !client.selecting && client.lock.is_none() {
        pane = pane.push(ui::divider()).push(composer(client, chat));
    }
    pane.width(Fill).height(Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_photo_keeps_its_shape_inside_its_room() {
        let room = |width, height| photo_room(&TgFile { width, height, ..TgFile::default() });
        assert_eq!(room(Some(1280), Some(640)), (320.0, 160.0));
        assert_eq!(room(Some(600), Some(1200)), (160.0, 320.0));
        // A small picture is not blown up, and one without a size gets a plain room.
        assert_eq!(room(Some(100), Some(80)), (100.0, 80.0));
        assert_eq!(room(None, Some(80)), (240.0, 160.0));
        assert_eq!(room(Some(0), Some(0)), (240.0, 160.0));
    }

    #[test]
    fn a_chat_is_described_by_its_kind() {
        let chat = |kind: &str, members| TgChat { chat_type: kind.into(), member_count: members, ..TgChat::default() };
        assert_eq!(type_label(&chat("bot", None)), "бот");
        assert_eq!(type_label(&chat("supergroup", Some(12))), "группа · 12 участников");
        assert_eq!(type_label(&chat("channel", Some(0))), "канал");
        assert_eq!(type_label(&chat("secret", None)), "secret");
    }
}
