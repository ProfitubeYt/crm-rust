//! What opens over the chats: the menus of a right press, a chat's password,
//! the period of the messages and a photo at its full size.

use iced::widget::{Column, Space, button, column, container, image, row, text};
use iced::{Alignment, ContentFit, Fill, Point, Size};

use super::client::{Client, Msg, PasswordForm, PasswordMode, PeriodForm, Photo, PhotoKey, Target};
use super::dates::{self, Preset};
use super::pane::HEADER;
use crate::ctx::Ctx;
use crate::theme::{self, size, space};
use crate::ui::{self, El, Icon, Kind};

const MENU: f32 = 220.0;
const MENU_ROW: f32 = 32.0;
const PERIOD: f32 = 280.0;

fn menu<'a>(client: &'a Client, target: Target, at: Point, ctx: &Ctx) -> Option<El<'a, Msg>> {
    let items: Vec<El<'a, Msg>> = match target {
        Target::Chat(id) => {
            let chat = client.chat(id)?;
            let hidden = client.hidden(chat);
            let hide = if hidden { ui::menu_item(Icon::Eye, "Показать", false, Msg::Hide(id, false)) } else { ui::menu_item(Icon::EyeOff, "Скрыть", false, Msg::Hide(id, true)) };
            let password = if chat.is_protected {
                ui::menu_item(Icon::Unlock, "Снять пароль", false, Msg::AskPassword(id, PasswordMode::Remove))
            } else {
                ui::menu_item(Icon::Key, "Поставить пароль", false, Msg::AskPassword(id, PasswordMode::Set))
            };
            vec![hide, password]
        }
        Target::Message(id) => {
            let mut items = vec![ui::menu_item(Icon::Scan, "В обработку", false, Msg::Process(id)), ui::menu_item(Icon::CheckSquare, "Добавить в выбор", false, Msg::Pick(id))];
            if client.messages.iter().any(|message| message.id == Some(id) && message.text.as_deref().is_some_and(|words| !words.is_empty())) {
                items.push(ui::menu_item(Icon::Copy, "Копировать текст", false, Msg::CopyText(id)));
            }
            items
        }
    };
    let room = Size::new(MENU, items.len() as f32 * MENU_ROW + 2.0 * space::SM);
    Some(ui::floating(ui::menu_panel(Column::with_children(items), MENU), at, room, ctx.window, Msg::CloseMenu))
}

fn password(form: &PasswordForm) -> El<'_, Msg> {
    let (title, about, action, placeholder) = match form.mode {
        PasswordMode::Set => ("Установить пароль", "Чат будет защищён паролем. Без него содержимое не будет показываться.", "Установить", "Минимум 4 символа"),
        PasswordMode::Remove => ("Снять защиту", "Введите текущий пароль, чтобы убрать защиту с этого чата.", "Снять защиту", "Текущий пароль"),
    };
    let setting = form.mode == PasswordMode::Set;
    let first = ui::secret_input(placeholder, &form.password).on_input(Msg::PasswordInput);
    // Enter submits from the last box of the form.
    let first = if setting { first } else { first.on_submit(Msg::PasswordSubmit) };
    let mut body = column![
        ui::dialog_title(title, (!form.busy).then_some(Msg::ClosePassword)),
        ui::muted(about),
        row![ui::faint("Чат:"), text(form.title.as_str()).size(size::SM).font(theme::FONT_MEDIUM)].spacing(space::XS),
        first,
    ]
    .spacing(space::MD);
    if setting {
        body = body.push(ui::secret_input("Повторите пароль", &form.confirm).on_input(Msg::PasswordConfirm).on_submit(Msg::PasswordSubmit));
    }
    if let Some(reason) = &form.error {
        body = body.push(ui::error_line(reason.as_str()));
    }
    let go = ui::btn(if form.busy { "Сохраняем…" } else { action }, if setting { Kind::Primary } else { Kind::Danger }, (!form.busy).then_some(Msg::PasswordSubmit));
    let actions = row![ui::fill(), ui::btn("Отмена", Kind::Secondary, (!form.busy).then_some(Msg::ClosePassword)), go].spacing(space::SM);
    ui::modal(Space::new(), body.push(actions), 420.0, (!form.busy).then_some(Msg::ClosePassword))
}

fn period<'a>(client: &'a Client, form: &'a PeriodForm, ctx: &Ctx) -> El<'a, Msg> {
    let today = dates::now().date();
    let mut lines = Column::new().spacing(space::XXS);
    for preset in Preset::ALL {
        let chosen = preset.range(today) == client.range;
        let mut words = row![text(preset.label()).size(size::BASE).width(Fill)].align_y(Alignment::Center);
        if chosen {
            words = words.push(ui::icon(Icon::Check, 14.0));
        }
        lines = lines.push(button(words).width(Fill).padding([space::XS + 2.0, space::SM]).style(theme::btn_menu(false)).on_press(Msg::Preset(preset)));
    }
    lines = lines.push(ui::divider());
    let fold = row![ui::icon(Icon::Calendar, 14.0), text("Свой диапазон").size(size::BASE).width(Fill), ui::icon(if form.custom { Icon::ChevronUp } else { Icon::ChevronDown }, 14.0)]
        .spacing(space::SM)
        .align_y(Alignment::Center);
    lines = lines.push(button(fold).width(Fill).padding([space::XS + 2.0, space::SM]).style(theme::btn_menu(false)).on_press(Msg::PeriodCustom));
    let mut rows = if form.custom { 8.0 } else { 7.0 };
    if form.custom {
        let empty = form.from.trim().is_empty() && form.to.trim().is_empty();
        let days = row![
            ui::field("От", ui::input("ДД.ММ.ГГГГ", &form.from).on_input(Msg::PeriodFrom).on_submit(Msg::PeriodApply)).width(Fill),
            ui::field("До", ui::input("ДД.ММ.ГГГГ", &form.to).on_input(Msg::PeriodTo).on_submit(Msg::PeriodApply)).width(Fill),
        ]
        .spacing(space::SM);
        let actions = row![ui::btn_small("Сброс", Kind::Ghost, Some(Msg::ClearRange)), ui::fill(), ui::btn_small("Применить", Kind::Primary, (!empty).then_some(Msg::PeriodApply))].align_y(Alignment::Center);
        let mut custom = column![days, actions].spacing(space::SM).padding([space::XS, space::SM]);
        if let Some(reason) = &form.error {
            custom = custom.push(ui::error_line(reason.as_str()));
            rows += 1.0;
        }
        lines = lines.push(custom);
        rows += 2.5;
    }
    let room = Size::new(PERIOD, rows * MENU_ROW + 2.0 * space::SM);
    // Under the period button, which stands at the right of the chat's header.
    let at = Point::new(ctx.window.width - PERIOD - space::LG, HEADER + space::XS);
    ui::floating(ui::menu_panel(lines, PERIOD), at, room, ctx.window, Msg::TogglePeriod)
}

fn viewer<'a>(client: &'a Client, key: PhotoKey, ctx: &Ctx) -> Option<El<'a, Msg>> {
    let Some(Photo::Ready(handle)) = client.photos.get(&key) else { return None };
    let width = (ctx.window.width - ctx.rail - 4.0 * space::XL).clamp(320.0, 1100.0);
    let height = (ctx.window.height - 220.0).clamp(200.0, 620.0);
    let picture = container(image(handle.clone()).content_fit(ContentFit::Contain).width(Fill).height(height)).width(Fill).center_x(Fill);
    let actions = row![ui::fill(), ui::btn_icon(Icon::Download, "Сохранить…", Kind::Secondary, Some(Msg::SavePhoto))];
    let body = column![ui::dialog_title("Фото", Some(Msg::CloseViewer)), picture, actions].spacing(space::MD);
    Some(ui::modal(Space::new(), body, width, Some(Msg::CloseViewer)))
}

/// The topmost of what is open, if anything is.
pub fn view<'a>(client: &'a Client, ctx: &Ctx) -> Option<El<'a, Msg>> {
    if let Some(key) = client.viewer
        && let Some(layer) = viewer(client, key, ctx)
    {
        return Some(layer);
    }
    if let Some(form) = &client.password {
        return Some(password(form));
    }
    if let Some(form) = &client.period {
        return Some(period(client, form, ctx));
    }
    let (target, at) = client.menu?;
    menu(client, target, at, ctx)
}
