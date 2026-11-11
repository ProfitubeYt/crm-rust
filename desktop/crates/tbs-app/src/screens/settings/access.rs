//! Who may open which folder: one checkbox per user and folder.

use iced::widget::{Column, Id, Row, Space, column, container, row};
use iced::{Alignment, Fill, Length, Task};
use serde_json::json;
use tbs_core::api::{AccessScope, AdminUser};

use crate::ctx::{Ctx, text_error};
use crate::theme::{Tone, space};
use crate::ui::{self, El, Icon};

use super::{avatar, panel};

const SEARCH_ID: &str = "access-search";
const USER_WIDTH: f32 = 260.0;
const FOLDER_WIDTH: f32 = 120.0;

pub fn focus_search<M: Send + 'static>() -> Task<M> {
    iced::widget::operation::focus(Id::new(SEARCH_ID))
}

#[derive(Clone, Debug)]
pub enum Msg {
    Users(Result<Vec<AdminUser>, String>),
    Scopes(Result<Vec<AccessScope>, String>),
    Search(String),
    Toggle(i64, i64),
    Saved(Result<AdminUser, String>),
}

#[derive(Default)]
pub struct Access {
    users: Option<Vec<AdminUser>>,
    scopes: Option<Vec<AccessScope>>,
    search: String,
}

/// The folder list after one checkbox flipped.
fn flipped(folders: &[i64], folder: i64) -> Vec<i64> {
    if folders.contains(&folder) {
        folders.iter().copied().filter(|id| *id != folder).collect()
    } else {
        folders.iter().copied().chain([folder]).collect()
    }
}

impl Access {
    pub fn load(&self, ctx: &Ctx) -> Task<Msg> {
        let (users, scopes) = (ctx.api.clone(), ctx.api.clone());
        Task::batch([
            Task::perform(async move { users.admin_users().await.map_err(text_error) }, Msg::Users),
            Task::perform(async move { scopes.access_scopes().await.map_err(text_error) }, Msg::Scopes),
        ])
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Users(Ok(users)) => self.users = Some(users),
            Msg::Scopes(Ok(scopes)) => self.scopes = Some(scopes),
            Msg::Users(Err(reason)) | Msg::Scopes(Err(reason)) => ctx.toast(Tone::Danger, reason),
            Msg::Search(value) => self.search = value,
            Msg::Toggle(user, folder) => {
                let Some(person) = self.users.iter_mut().flatten().find(|person| person.id == user) else { return Task::none() };
                if person.is_admin() {
                    return Task::none();
                }
                let folders = flipped(&person.allowed_folders, folder);
                person.allowed_folders.clone_from(&folders);
                let api = ctx.api.clone();
                let changes = json!({ "allowed_folders": folders });
                return Task::perform(async move { api.update_user(user, &changes).await.map_err(text_error) }, Msg::Saved);
            }
            Msg::Saved(Ok(user)) => {
                ctx.toast(Tone::Success, format!("Доступ к папке для @{} обновлён", user.username));
                if let Some(person) = self.users.iter_mut().flatten().find(|person| person.id == user.id) {
                    *person = user;
                }
            }
            Msg::Saved(Err(_)) => {
                ctx.toast(Tone::Danger, "Не удалось обновить доступ");
                return self.load(ctx);
            }
        }
        Task::none()
    }

    fn matrix<'a>(&'a self, users: Vec<&'a AdminUser>, scopes: &'a [AccessScope]) -> El<'a, Msg> {
        let mut head = Row::new().spacing(space::SM).push(container(ui::faint("Пользователь")).width(Length::Fixed(USER_WIDTH)));
        for scope in scopes {
            head = head.push(container(ui::faint(scope.name.as_str())).width(Length::Fixed(FOLDER_WIDTH)).clip(true));
        }
        let mut lines = Column::new().push(container(head).padding([space::SM, 0.0])).push(ui::divider());
        for user in users {
            let mut who = row![avatar(user.title(), 28.0), column![ui::label(user.title().to_owned()), ui::faint(format!("@{}", user.username))]].spacing(space::SM).align_y(Alignment::Center);
            if user.is_admin() {
                who = who.push(ui::badge("admin", Tone::Accent));
            }
            let mut line = Row::new().spacing(space::SM).align_y(Alignment::Center).push(container(who).width(Length::Fixed(USER_WIDTH)));
            for scope in scopes {
                let (id, folder) = (user.id, scope.id);
                let cell: El<'a, Msg> = if user.is_admin() {
                    ui::hint_on(ui::check("", true, |_| Msg::Search(String::new())).on_toggle_maybe(None::<fn(bool) -> Msg>), "admin: полный доступ")
                } else {
                    ui::check("", user.allowed_folders.contains(&folder), move |_| Msg::Toggle(id, folder)).into()
                };
                line = line.push(container(cell).width(Length::Fixed(FOLDER_WIDTH)));
            }
            lines = lines.push(container(line).padding([space::SM, 0.0])).push(ui::divider());
        }
        iced::widget::scrollable(lines).direction(iced::widget::scrollable::Direction::Horizontal(Default::default())).width(Fill).into()
    }

    pub fn view<'a>(&'a self, _ctx: &Ctx) -> El<'a, Msg> {
        let (Some(users), Some(scopes)) = (&self.users, &self.scopes) else {
            return panel("Матрица доступа", "Получаем список пользователей и папок.", Space::new(), ui::loading("Загружаем матрицу…"));
        };
        let needle = self.search.trim().to_lowercase();
        let shown: Vec<&AdminUser> = users.iter().filter(|user| needle.is_empty() || user.username.to_lowercase().contains(&needle) || user.display_name.to_lowercase().contains(&needle)).collect();
        let total = shown.len();
        let folders = format!("{} {}", scopes.len(), tbs_core::format::plural(scopes.len(), "папка", "папки", "папок"));
        let tools = row![
            ui::input("Поиск пользователя", &self.search).id(Id::new(SEARCH_ID)).on_input(Msg::Search).width(Length::Fixed(300.0)),
            ui::fill(),
            ui::faint(format!("{total} из {}", users.len())),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center);
        let body: El<'a, Msg> = if scopes.is_empty() {
            ui::empty_state(Icon::FolderPerson, "Нет настроенных папок", "Создайте папки в системе, чтобы управлять доступами.")
        } else if users.is_empty() {
            ui::empty_state(Icon::Users, "Нет пользователей", "Добавьте операторов в разделе «Пользователи».")
        } else if shown.is_empty() {
            ui::empty_state(Icon::Search, "Никого не найдено", "Попробуйте изменить запрос.")
        } else {
            self.matrix(shown, scopes)
        };
        let content = column![tools, body, ui::faint("Изменения сохраняются мгновенно. Admin-пользователи всегда видят все папки.")].spacing(space::MD);
        panel("Матрица доступа", "Рядом с пользователем — папки, к которым у него есть доступ.", ui::badge(folders, Tone::Neutral), content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checkbox_adds_or_removes_one_folder() {
        assert_eq!(flipped(&[1, 2], 2), vec![1]);
        assert_eq!(flipped(&[1], 3), vec![1, 3]);
    }
}
