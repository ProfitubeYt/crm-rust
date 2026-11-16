//! Telegram's own sign-in on the server's TDLib client, and what the section
//! shows while it has no chats to show.

use iced::widget::{button, center, column, container, row, text};
use iced::{Alignment, Fill};

use super::client::{AuthForm, Client, Msg, Step};
use crate::theme::{self, Tone, control, size, space};
use crate::ui::{self, El, Icon};

const CARD: f32 = 400.0;

fn mark<'a>(glyph: Icon, tone: Tone) -> El<'a, Msg> {
    container(ui::icon_tone(glyph, 20.0, tone)).center_x(40.0).center_y(40.0).style(theme::tinted(tone)).into()
}

fn eyebrow<'a>(tone: Tone, words: &'a str) -> El<'a, Msg> {
    row![ui::status_dot(tone), text(words).size(size::XS).font(theme::FONT_MEDIUM)].spacing(space::XS).align_y(Alignment::Center).into()
}

/// The step that needs something typed: the number, the code, the cloud password.
fn card<'a>(step: Step, form: &'a AuthForm) -> El<'a, Msg> {
    let (stage, glyph, title, hint, placeholder) = match step {
        Step::Phone => ("01 / Авторизация", Icon::Phone, "Введите номер телефона", "Отправим код в приложение Telegram. Номер в международном формате, например +70000000000", "+70000000000"),
        Step::Code => ("02 / Подтверждение", Icon::Key, "Шестизначный код", "Telegram отправил код подтверждения в приложение.", "12345"),
        Step::Password => ("03 / Защита", Icon::Lock, "Облачный пароль", "Двухфакторный пароль вашего аккаунта Telegram.", "Пароль"),
    };
    let field = if step == Step::Password { ui::secret_input(placeholder, &form.value) } else { ui::input(placeholder, &form.value) };
    let can = !form.busy && !form.value.trim().is_empty();
    let go = button(container(text(if form.busy { "Отправляем…" } else { "Продолжить" }).size(size::BASE).font(theme::FONT_MEDIUM)).center_x(Fill).center_y(Fill))
        .width(Fill)
        .height(control::LG)
        .style(theme::btn_primary)
        .on_press_maybe(can.then_some(Msg::AuthSubmit));
    let mut block = column![
        eyebrow(Tone::Accent, stage),
        mark(glyph, Tone::Accent),
        column![ui::title(title), ui::muted(hint)].spacing(space::XS),
        field.on_input(Msg::AuthInput).on_submit(Msg::AuthSubmit).size(size::MD).padding([space::SM + 2.0, space::MD]),
    ]
    .spacing(space::MD);
    if let Some(reason) = &form.error {
        block = block.push(ui::error_line(reason.as_str()));
    }
    block = block.push(go).push(row![ui::faint("Telegram · TDLib"), ui::fill(), ui::faint("Enter")]);
    center(ui::card(block).max_width(CARD)).into()
}

fn board<'a>(tone: Tone, stage: &'a str, glyph: Icon, title: &'a str, hint: &'a str, reason: Option<&'a str>) -> El<'a, Msg> {
    let mut block = column![eyebrow(tone, stage), mark(glyph, tone), column![ui::title(title), ui::muted(hint)].spacing(space::XS)].spacing(space::MD);
    if let Some(reason) = reason {
        block = block.push(ui::error_line(reason));
    }
    center(ui::card(block).max_width(CARD)).into()
}

/// What stands in place of the chats until Telegram is signed in and ready.
/// `None` once it is.
pub fn view(client: &Client) -> Option<El<'_, Msg>> {
    if client.ready() {
        return None;
    }
    let reason = client.status_error.as_deref();
    let Some(status) = &client.status else {
        return Some(match reason {
            None => board(Tone::Warning, "Подключение", Icon::Loader, "Подключаемся", "Получаем статус Telegram-сессии.", None),
            Some(_) => board(Tone::Warning, "Подключение", Icon::Loader, "Инициализация", "Telegram готовится к работе.", reason),
        });
    };
    if let Some(step) = client.step() {
        return Some(card(step, &client.auth));
    }
    Some(match status.state.as_str() {
        "tdlib_unavailable" | "misconfigured" => board(Tone::Danger, "Сервис", Icon::Alert, "Сервис недоступен", "Telegram-клиент не настроен на сервере.", None),
        _ => board(Tone::Warning, "Подключение", Icon::Loader, "Инициализация", "Telegram готовится к работе.", reason),
    })
}
