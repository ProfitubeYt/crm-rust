//! Who is signed in, the way out, and the theme.

use iced::widget::{Space, column, row};
use iced::{Alignment, Task};

use crate::ctx::{Ctx, Request};
use crate::theme::{Tone, space};
use crate::ui::{self, El, Icon, Kind};

use super::{avatar, panel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    Light,
    Dark,
    System,
}

impl ThemeChoice {
    pub(crate) fn saved(self) -> Option<String> {
        match self {
            ThemeChoice::Light => Some("light".into()),
            ThemeChoice::Dark => Some("dark".into()),
            ThemeChoice::System => Some("system".into()),
        }
    }

    /// Nothing saved means the light theme, the one the client is designed in.
    pub(crate) fn from_saved(saved: Option<&str>) -> Self {
        match saved {
            Some("dark") => ThemeChoice::Dark,
            Some("system") => ThemeChoice::System,
            _ => ThemeChoice::Light,
        }
    }

    fn name(self) -> &'static str {
        match self {
            ThemeChoice::Light => "светлая",
            ThemeChoice::Dark => "тёмная",
            ThemeChoice::System => "системная",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Msg {
    AskLogout,
    StayIn,
    Logout,
    Theme(ThemeChoice),
}

#[derive(Default)]
pub struct Profile {
    confirming: bool,
}

impl Profile {
    pub fn escape(&mut self) {
        self.confirming = false;
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::AskLogout => self.confirming = true,
            Msg::StayIn => self.confirming = false,
            Msg::Logout => {
                self.confirming = false;
                ctx.request(Request::SignOut);
            }
            Msg::Theme(choice) => {
                ctx.prefs.theme = choice.saved();
                ctx.request(Request::ThemeChanged);
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, ctx: &Ctx) -> El<'a, Msg> {
        let Some(user) = &ctx.user else { return ui::loading("Загрузка…") };
        let name = user.title().to_owned();
        let role_tone = if user.is_admin() { Tone::Accent } else { Tone::Neutral };
        let two_factor = if user.totp_enabled { ui::badge("2FA", Tone::Success) } else { ui::badge("2FA отключена", Tone::Warning) };
        let badges = row![ui::badge(user.role.clone(), role_tone), ui::badge(format!("@{}", user.username), Tone::Neutral), two_factor].spacing(space::XS).align_y(Alignment::Center);
        let who = column![ui::heading(name.clone()), badges].spacing(space::XS).width(iced::Fill);
        let hero = ui::card(
            row![avatar(&name, 56.0), who, ui::btn_icon(Icon::Logout, "Выйти", Kind::Secondary, Some(Msg::AskLogout))]
                .spacing(space::LG)
                .align_y(Alignment::Center),
        );
        let locked = |value: String| ui::input("", &value).font(crate::theme::FONT_MONO);
        let fields = column![
            ui::field("Логин", locked(user.username.clone())),
            ui::faint("Используется для входа в систему."),
            ui::field("Отображаемое имя", ui::input("", &name)),
            ui::faint("Видно в шапке и в логах действий."),
            ui::field("Роль", ui::input("", &user.role)),
            ui::faint("Определяет, какие разделы доступны."),
        ]
        .spacing(space::XS);
        column![hero, panel("Учётная запись", "Эти поля управляются администратором.", Space::new(), fields)].spacing(space::MD).into()
    }

    pub fn appearance<'a>(&'a self, ctx: &Ctx) -> El<'a, Msg> {
        let choice = ThemeChoice::from_saved(ctx.prefs.theme.as_deref());
        let actual = if ctx.dark { "тёмная" } else { "светлая" };
        let pick = |glyph: Icon, caption: &'static str, value: ThemeChoice| {
            let kind = if choice == value { Kind::Primary } else { Kind::Secondary };
            ui::btn_icon(glyph, caption, kind, Some(Msg::Theme(value)))
        };
        let options = row![pick(Icon::Sun, "Светлая", ThemeChoice::Light), pick(Icon::Moon, "Тёмная", ThemeChoice::Dark), pick(Icon::Monitor, "Системная", ThemeChoice::System)].spacing(space::SM);
        panel("Тема", format!("Сейчас активна {} · фактически {actual}", choice.name()), Space::new(), options)
    }

    pub fn overlay(&self) -> Option<El<'_, Msg>> {
        self.confirming.then(|| {
            let actions = row![ui::fill(), ui::btn("Остаться", Kind::Secondary, Some(Msg::StayIn)), ui::btn("Выйти", Kind::Danger, Some(Msg::Logout))].spacing(space::SM);
            let body = column![ui::dialog_title("Выйти из аккаунта?", Some(Msg::StayIn)), ui::muted("Активная сессия будет завершена, на других устройствах вход сохранится."), actions].spacing(space::MD);
            ui::modal(Space::new(), body, 420.0, Some(Msg::StayIn))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_theme_choice_round_trips_through_preferences() {
        for choice in [ThemeChoice::Light, ThemeChoice::Dark, ThemeChoice::System] {
            assert_eq!(ThemeChoice::from_saved(choice.saved().as_deref()), choice);
        }
    }
}
