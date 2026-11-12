//! Personal preferences, team administration and system switches.

mod access;
mod editor;
mod periods;
mod profile;
mod security;
mod sessions;
mod system;
mod updates;
mod users;
mod workbook;

pub use profile::ThemeChoice;

use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Column, Stack, button, column, container, opaque, row, text};
use iced::{Alignment, Border, Color, Event, Fill, Length, Subscription, Task, Theme, event, window};

use crate::ctx::Ctx;
use crate::theme::{self, p, size, space};
use crate::ui::{self, El, Icon};

use access::Access;
use periods::Periods;
use profile::Profile;
use security::Security;
use sessions::Sessions;
use system::System;
use updates::Updates;
use users::Users;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Profile,
    Security,
    Appearance,
    Users,
    Access,
    Periods,
    System,
    Updates,
    Sessions,
}

/// Who a section of the settings is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum For {
    Everyone,
    /// The Checks administrator: the senior role inside this workspace.
    Checks,
    /// Whoever manages the accounts of the whole CRM. On a server that keeps
    /// that right apart it is the CRM administrator, and the Checks role does
    /// not stand in for it.
    Accounts,
    /// Both: the accounts are the CRM's, the folders are Checks'.
    Both,
}

impl For {
    fn open_to(self, ctx: &Ctx) -> bool {
        match self {
            For::Everyone => true,
            For::Checks => ctx.is_admin(),
            For::Accounts => ctx.manages_accounts(),
            For::Both => ctx.is_admin() && ctx.manages_accounts(),
        }
    }
}

struct Group {
    name: &'static str,
    items: &'static [(Section, Icon, &'static str, For)],
}

const NAV: [Group; 3] = [
    Group {
        name: "Личное",
        items: &[
            (Section::Profile, Icon::User, "Профиль", For::Everyone),
            (Section::Security, Icon::Key, "Безопасность", For::Everyone),
            (Section::Appearance, Icon::Palette, "Внешний вид", For::Everyone),
        ],
    },
    Group {
        name: "Команда",
        items: &[
            (Section::Users, Icon::Users, "Пользователи", For::Accounts),
            (Section::Access, Icon::FolderPerson, "Доступы и папки", For::Both),
            (Section::Periods, Icon::Lock, "Заблокир. периоды", For::Checks),
        ],
    },
    Group {
        name: "Система",
        items: &[
            (Section::System, Icon::Server, "Системные", For::Checks),
            (Section::Updates, Icon::Download, "Обновления", For::Everyone),
            (Section::Sessions, Icon::Monitor, "Сессии", For::Accounts),
        ],
    },
];

const NAV_WIDTH: f32 = 224.0;

#[derive(Clone, Debug)]
pub enum Msg {
    Section(Section),
    Escape,
    FocusSearch,
    Add,
    Profile(profile::Msg),
    Security(security::Msg),
    Users(users::Msg),
    Access(access::Msg),
    Periods(periods::Msg),
    System(system::Msg),
    Sessions(sessions::Msg),
    Updates(updates::Msg),
}

pub struct Settings {
    section: Section,
    profile: Profile,
    security: Security,
    users: Users,
    access: Access,
    periods: Periods,
    system: System,
    sessions: Sessions,
    updates: Updates,
}

fn hotkey(event: Event, _status: event::Status, _window: window::Id) -> Option<Msg> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, physical_key, .. }) = event else { return None };
    if key == Key::Named(Named::Escape) {
        return Some(Msg::Escape);
    }
    if !modifiers.command() {
        return None;
    }
    match key.to_latin(physical_key) {
        Some('k') => Some(Msg::FocusSearch),
        Some('n') => Some(Msg::Add),
        _ => None,
    }
}

/// Up to two letters that stand in for a picture of the person.
pub(crate) fn initials(name: &str) -> String {
    let mut words = name.split_whitespace();
    let picked: String = match (words.next(), words.next()) {
        (Some(first), Some(second)) => first.chars().take(1).chain(second.chars().take(1)).collect(),
        (Some(only), None) => only.chars().take(2).collect(),
        _ => "?".to_owned(),
    };
    picked.to_uppercase()
}

pub(crate) fn avatar<'a, M: 'a>(name: &str, px: f32) -> El<'a, M> {
    let letters = text(initials(name)).size(px * 0.38).font(theme::FONT_MEDIUM);
    container(letters)
        .center_x(Length::Fixed(px))
        .center_y(Length::Fixed(px))
        .style(move |t: &Theme| {
            let c = p(t);
            container::Style {
                background: Some(c.accent_soft.into()),
                text_color: Some(c.accent),
                border: Border { radius: (px / 2.0).into(), width: 0.0, color: Color::TRANSPARENT },
                ..Default::default()
            }
        })
        .into()
}

/// A titled block of a settings page.
pub(crate) fn panel<'a, M: 'a>(name: &'a str, hint: impl text::IntoFragment<'a>, aside: impl Into<El<'a, M>>, body: impl Into<El<'a, M>>) -> El<'a, M> {
    let head = row![column![ui::heading(name), ui::muted(hint)].spacing(space::XXS).width(Fill), aside.into()].spacing(space::MD).align_y(Alignment::Center);
    ui::card(column![head, body.into()].spacing(space::MD)).into()
}

/// A label on the left and its control on the right.
pub(crate) fn setting_row<'a, M: 'a>(name: impl text::IntoFragment<'a>, hint: impl text::IntoFragment<'a>, control: impl Into<El<'a, M>>) -> El<'a, M> {
    row![column![ui::label(name), ui::faint(hint)].spacing(space::XXS).width(Fill), control.into()].spacing(space::LG).align_y(Alignment::Center).into()
}

pub(crate) fn plural_users(count: usize) -> String {
    format!("{count} {}", tbs_core::format::plural(count, "юзер", "юзера", "юзеров"))
}

fn page_text(section: Section) -> (&'static str, &'static str) {
    match section {
        Section::Profile => ("Профиль", "Информация о вашей учётной записи и кнопка выхода. Логин и пароль может изменить только администратор."),
        Section::Security => ("Безопасность", "Двухфакторная аутентификация текущего пользователя. Устройства, где выполнен вход, — в разделе «Сессии»."),
        Section::Appearance => ("Внешний вид", "Тема интерфейса. Системный режим следует за настройкой ОС."),
        Section::Users => ("Пользователи", "Создание учётных записей, изменение ролей, доступов и принудительной 2FA."),
        Section::Access => ("Доступы и папки", "Матрица «пользователь × папка». Изменения применяются сразу."),
        Section::Periods => ("Заблокированные периоды", "Запрет на любые правки транзакций в указанных диапазонах. Применяется ко всем пользователям."),
        Section::System => ("Системные настройки", "Глобальные тогглы, влияющие на все учётные записи."),
        Section::Updates => ("Обновления", "Новые версии приложения скачиваются и ставятся отсюда."),
        Section::Sessions => ("Активные сессии", "Все активные токены и устройства команды."),
    }
}

impl Settings {
    pub fn new(ctx: &Ctx) -> Self {
        Self {
            section: Section::Profile,
            profile: Profile::default(),
            security: Security::default(),
            users: Users::new(ctx),
            access: Access::default(),
            periods: Periods::default(),
            system: System::new(ctx),
            sessions: Sessions::new(ctx),
            updates: Updates::default(),
        }
    }

    /// The watched workbook keeps being imported from any screen.
    pub fn background(&self) -> Subscription<Msg> {
        self.system.watch().map(Msg::System)
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        let mut all = vec![event::listen_with(hotkey)];
        if self.section == Section::Sessions {
            all.push(self.sessions.subscription().map(Msg::Sessions));
        }
        Subscription::batch(all)
    }

    /// The section the account may see: its own choice while it is still for
    /// it, the profile otherwise.
    fn shown(&self, ctx: &Ctx) -> Section {
        let open = NAV.iter().flat_map(|group| group.items).find(|(listed, ..)| *listed == self.section).is_none_or(|(.., audience)| audience.open_to(ctx));
        if open { self.section } else { Section::Profile }
    }

    /// What the account may manage changed while the settings were open. A
    /// section it no longer has is left, and the accounts loaded for it are
    /// not kept: they are not this account's to look at any more.
    pub fn access_changed(&mut self, ctx: &Ctx) {
        self.section = self.shown(ctx);
        if !For::Accounts.open_to(ctx) {
            self.users = Users::new(ctx);
            self.sessions = Sessions::new(ctx);
        }
        if !For::Both.open_to(ctx) {
            self.access = Access::default();
        }
    }

    /// Every visit reloads the open section: the team changes without us.
    pub fn enter(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        self.load(ctx)
    }

    fn load(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        match self.section {
            Section::Profile | Section::Appearance => Task::none(),
            Section::Security => self.security.load(ctx).map(Msg::Security),
            Section::Users => self.users.load(ctx).map(Msg::Users),
            Section::Access => self.access.load(ctx).map(Msg::Access),
            Section::Periods => self.periods.load(ctx).map(Msg::Periods),
            Section::System => self.system.load(ctx).map(Msg::System),
            Section::Sessions => self.sessions.load(ctx).map(Msg::Sessions),
            Section::Updates => Task::none(),
        }
    }

    #[cfg(feature = "devtools")]
    pub fn probe(name: &str) -> Option<Msg> {
        Some(match name {
            "s-profile" => Msg::Section(Section::Profile),
            "s-security" => Msg::Section(Section::Security),
            "s-appearance" => Msg::Section(Section::Appearance),
            "s-users" => Msg::Section(Section::Users),
            "s-access" => Msg::Section(Section::Access),
            "s-periods" => Msg::Section(Section::Periods),
            "s-system" => Msg::Section(Section::System),
            "s-updates" => Msg::Section(Section::Updates),
            "s-sessions" => Msg::Section(Section::Sessions),
            "s-stepup" => Msg::Security(security::Msg::Ask(security::Purpose::Disable)),
            _ => return None,
        })
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Section(section) => {
                self.section = section;
                return self.load(ctx);
            }
            Msg::Escape => match self.section {
                Section::Profile => self.profile.escape(),
                Section::Security => self.security.escape(),
                Section::Users => self.users.escape(),
                Section::Periods => self.periods.escape(),
                Section::Sessions => self.sessions.escape(),
                _ => {}
            },
            Msg::FocusSearch => {
                return match self.section {
                    Section::Users => users::focus_search(),
                    Section::Access => access::focus_search(),
                    _ => Task::none(),
                };
            }
            Msg::Add => {
                if self.section == Section::Users {
                    return self.users.update(users::Msg::OpenAdd, ctx).map(Msg::Users);
                }
            }
            Msg::Profile(inner) => return self.profile.update(inner, ctx).map(Msg::Profile),
            Msg::Security(inner) => return self.security.update(inner, ctx).map(Msg::Security),
            Msg::Users(inner) => return self.users.update(inner, ctx).map(Msg::Users),
            Msg::Access(inner) => return self.access.update(inner, ctx).map(Msg::Access),
            Msg::Periods(inner) => return self.periods.update(inner, ctx).map(Msg::Periods),
            Msg::System(inner) => return self.system.update(inner, ctx).map(Msg::System),
            Msg::Sessions(inner) => return self.sessions.update(inner, ctx).map(Msg::Sessions),
            Msg::Updates(inner) => return self.updates.update(inner, ctx).map(Msg::Updates),
        }
        Task::none()
    }

    fn nav_item(&self, section: Section, glyph: Icon, caption: &'static str) -> El<'_, Msg> {
        let mut line = row![ui::icon(glyph, 16.0), text(caption).size(size::BASE).width(Fill)].spacing(space::SM).align_y(Alignment::Center);
        if section == Section::Users
            && let Some(count) = self.users.count()
        {
            line = line.push(ui::faint(count.to_string()));
        }
        button(line).width(Fill).padding([6.0, 10.0]).style(theme::btn_nav(self.section == section)).on_press(Msg::Section(section)).into()
    }

    fn nav(&self, ctx: &Ctx) -> El<'_, Msg> {
        let mut groups = Column::new().spacing(space::LG);
        for group in &NAV {
            let items: Vec<El<'_, Msg>> =
                group.items.iter().filter(|(_, _, _, audience)| audience.open_to(ctx)).map(|(section, glyph, caption, _)| self.nav_item(*section, *glyph, caption)).collect();
            if !items.is_empty() {
                groups = groups.push(column![container(ui::faint(group.name)).padding([0.0, 10.0]), Column::with_children(items).spacing(space::XXS)].spacing(space::XS));
            }
        }
        container(ui::scroll(groups).height(Fill)).width(Length::Fixed(NAV_WIDTH)).height(Fill).padding([space::LG, space::MD]).style(theme::surface).into()
    }

    pub fn view(&self, ctx: &Ctx) -> El<'_, Msg> {
        // Nothing of a section is drawn for an account that has just lost it.
        let section = self.shown(ctx);
        let (name, hint) = page_text(section);
        let aside: El<'_, Msg> = match section {
            Section::Users => ui::btn_icon(Icon::Plus, "Добавить", ui::Kind::Primary, Some(Msg::Add)).into(),
            _ => iced::widget::Space::new().into(),
        };
        let body: El<'_, Msg> = match section {
            Section::Profile => tall(self.profile.view(ctx).map(Msg::Profile)),
            Section::Security => tall(self.security.view(ctx).map(Msg::Security)),
            Section::Appearance => tall(self.profile.appearance(ctx).map(Msg::Profile)),
            Section::Users => self.users.view(ctx).map(Msg::Users),
            Section::Access => tall(self.access.view(ctx).map(Msg::Access)),
            Section::Periods => tall(self.periods.view(ctx).map(Msg::Periods)),
            Section::System => tall(self.system.view(ctx).map(Msg::System)),
            Section::Updates => tall(self.updates.view(ctx).map(Msg::Updates)),
            Section::Sessions => self.sessions.view(ctx).map(Msg::Sessions),
        };
        let page = column![ui::page_header(name, Some(hint), aside), body].spacing(space::MD);
        row![self.nav(ctx), ui::divider_v(), container(page).padding(ui::PAGE_PADDING).width(Fill).height(Fill).style(theme::page)].into()
    }

    pub fn overlay(&self, ctx: &Ctx) -> Option<El<'_, Msg>> {
        let layers: Vec<El<'_, Msg>> = match self.section {
            Section::Profile => self.profile.overlay().into_iter().map(|e| e.map(Msg::Profile)).collect(),
            Section::Security => self.security.overlay().into_iter().map(|e| e.map(Msg::Security)).collect(),
            Section::Users => self.users.overlay(ctx).into_iter().map(|e| e.map(Msg::Users)).collect(),
            Section::Sessions => self.sessions.overlay(ctx).into_iter().map(|e| e.map(Msg::Sessions)).collect(),
            _ => Vec::new(),
        };
        if layers.is_empty() {
            return None;
        }
        Some(opaque(Stack::with_children(layers).width(Fill).height(Fill)))
    }
}

/// A page taller than the window scrolls inside the content area.
fn tall<'a>(content: El<'a, Msg>) -> El<'a, Msg> {
    ui::scroll(container(content).padding(iced::Padding { right: space::MD, ..Default::default() })).height(Fill).into()
}


#[cfg(test)]
mod tests {
    use iced_test::Simulator;
    use iced_test::core::Settings as SimSettings;
    use tbs_core::model::UserInfo;

    use super::*;

    #[test]
    fn initials_come_from_the_first_two_words() {
        assert_eq!(initials("Андрей Бачинин Иванович"), "АБ");
        assert_eq!(initials("anna"), "AN");
        assert_eq!(initials("  "), "?");
    }

    /// The sections the settings offer to an account the server described so.
    fn offered(role: &str, global_admin: Option<bool>) -> Vec<&'static str> {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = Ctx::for_tests(dir.path());
        ctx.user = Some(UserInfo { id: 3, username: "u".into(), role: role.into(), global_admin, ..Default::default() });
        let screen = Settings::new(&ctx);
        let mut sim = Simulator::with_size(SimSettings::default(), iced::Size::new(1360.0, 860.0), screen.view(&ctx));
        ["Пользователи", "Доступы и папки", "Заблокир. периоды", "Системные", "Сессии", "Обновления", "Профиль"].into_iter().filter(|name| sim.find(*name).is_ok()).collect()
    }

    #[test]
    fn a_server_from_before_crm_administration_keeps_the_settings_as_they_were() {
        assert_eq!(offered("admin", None), ["Пользователи", "Доступы и папки", "Заблокир. периоды", "Системные", "Сессии", "Обновления", "Профиль"]);
        assert_eq!(offered("operator", None), ["Обновления", "Профиль"]);
    }

    #[test]
    fn accounts_are_the_crm_administrators_and_checks_stays_the_checks_administrators() {
        // Both rights, as every administrator has right after the migration.
        assert_eq!(offered("admin", Some(true)), ["Пользователи", "Доступы и папки", "Заблокир. периоды", "Системные", "Сессии", "Обновления", "Профиль"]);
        // Runs Checks, does not manage accounts: no users, no sessions of everybody.
        assert_eq!(offered("admin", Some(false)), ["Заблокир. периоды", "Системные", "Обновления", "Профиль"]);
        // Manages accounts and is an operator in Checks: the accounts, not Checks' own administration.
        assert_eq!(offered("operator", Some(true)), ["Пользователи", "Сессии", "Обновления", "Профиль"]);
        assert_eq!(offered("operator", Some(false)), ["Обновления", "Профиль"]);
    }

    #[test]
    fn an_account_that_stops_managing_accounts_is_taken_off_their_pages() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = Ctx::for_tests(dir.path());
        ctx.user = Some(UserInfo { id: 3, username: "u".into(), role: "operator".into(), global_admin: Some(true), ..Default::default() });
        let mut screen = Settings::new(&ctx);
        screen.section = Section::Users;
        let sees = |screen: &Settings, ctx: &Ctx, wanted: &str| Simulator::with_size(SimSettings::default(), iced::Size::new(1360.0, 860.0), screen.view(ctx)).find(wanted).is_ok();
        assert!(sees(&screen, &ctx, "Добавить"));

        // The right is taken away. Before the screen is told, nothing of the section is drawn.
        ctx.user.as_mut().unwrap().global_admin = Some(false);
        assert!(!sees(&screen, &ctx, "Добавить"));
        screen.access_changed(&ctx);
        assert!(screen.section == Section::Profile);
        // A section the account still has is left where it is.
        screen.section = Section::Updates;
        screen.access_changed(&ctx);
        assert!(screen.section == Section::Updates);
    }
}
