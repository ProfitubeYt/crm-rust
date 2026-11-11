//! The sheet that makes an Excel workbook and shows where it is open.
//!
//! The server builds the workbook's file with its own address and the book's
//! number inside, so in Excel a person only types the password. Nothing secret
//! is in the file: the server checks the password and gives every computer a
//! connection of its own, which is listed here and can be withdrawn.

use std::collections::HashMap;

use chrono::Utc;
use iced::widget::{column, container, row};
use iced::{Alignment, Fill, Task};
use tbs_core::api::{ApiError, ExcelActivation, ExcelGrant};
use tbs_core::format;

use super::{Msg as Screen, Parcel, Sheet, Transactions};
use crate::ctx::Ctx;
use crate::theme::{Tone, space};
use crate::ui::{self, El, Icon, Kind};

const WIDTH: f32 = 620.0;
const DEFAULT_NAME: &str = "Чеки";
const UNNAMED: &str = "Книга Excel";
const MAX_NAME: usize = 80;
const MIN_PASSWORD: usize = 8;
const MAX_PASSWORD: usize = 128;

#[derive(Clone, Debug)]
pub enum Msg {
    Load,
    Loaded(Result<Vec<ExcelGrant>, String>, Result<Vec<ExcelActivation>, String>),
    Name(String),
    NewPassword(String),
    Create,
    Created(Result<ExcelActivation, String>),
    Download(String),
    Downloaded(String, Result<(String, Parcel<Vec<u8>>), String>),
    Saved(String, Result<Option<String>, String>),
    Password(String, String),
    SavePassword(String),
    PasswordSaved(String, Result<ExcelActivation, String>),
    RemovePassword(String),
    PasswordRemoved(String, Result<(), String>),
    Revoke(String),
    Revoked(String, Result<(), String>),
    Close,
}

/// A workbook as the panel lists it: its password, and the computers it is
/// open on.
#[derive(Debug, PartialEq)]
pub struct Book<'a> {
    pub id: &'a str,
    pub name: String,
    pub activation: Option<&'a ExcelActivation>,
    pub computers: Vec<&'a ExcelGrant>,
}

pub struct ExcelPanel {
    grants: Vec<ExcelGrant>,
    activations: Vec<ExcelActivation>,
    loading: bool,
    list_error: Option<String>,
    activations_error: Option<String>,
    name: String,
    password: String,
    creating: bool,
    error: Option<String>,
    /// The workbook made in this panel, whose next steps are spelled out.
    created: Option<String>,
    downloading: Option<String>,
    download_error: Option<(String, String)>,
    revoking: Option<String>,
    passwords: HashMap<String, String>,
    password_book: Option<String>,
    password_saved: Option<String>,
    password_error: Option<(String, String)>,
}

impl ExcelPanel {
    pub fn new() -> Self {
        Self {
            grants: Vec::new(),
            activations: Vec::new(),
            loading: true,
            list_error: None,
            activations_error: None,
            name: DEFAULT_NAME.to_string(),
            password: String::new(),
            creating: false,
            error: None,
            created: None,
            downloading: None,
            download_error: None,
            revoking: None,
            passwords: HashMap::new(),
            password_book: None,
            password_saved: None,
            password_error: None,
        }
    }

    fn busy(&self) -> bool {
        self.creating || self.downloading.is_some() || self.revoking.is_some() || self.password_book.is_some()
    }

    fn remember(&mut self, activation: ExcelActivation) {
        self.activations.retain(|known| known.workbook_id != activation.workbook_id);
        self.activations.insert(0, activation);
    }
}

/// The server's own word for what went wrong, wherever its reply put it.
fn reason(error: &ApiError) -> &str {
    match error {
        ApiError::Http { code: Some(code), .. } => code,
        ApiError::Http { message, .. } => message,
        _ => "",
    }
}

/// What a person should read when a request fails.
fn request_error(error: &ApiError, fallback: &str) -> String {
    match error.status() {
        Some(401) => "Войдите в приложение заново.".to_string(),
        Some(403) => "Доступ к книгам Excel закрыт. Проверьте доступ к транзакциям.".to_string(),
        Some(429) => "Слишком много запросов. Попробуйте немного позже.".to_string(),
        _ if error.is_network() => "Нет связи с сервером. Проверьте подключение и повторите попытку.".to_string(),
        _ => fallback.to_string(),
    }
}

const NO_ADDRESS: &str = "Сервер не знает адреса, по которому книга сможет к нему обращаться. Сообщите администратору: нужен адрес https.";
const NO_TEMPLATE: &str = "На сервере нет шаблона книги. Сообщите администратору.";
const OLD_SERVER: &str = "Сервер ещё не умеет готовить книги. Попросите администратора обновить сервер и повторите.";

fn create_error(error: &ApiError) -> String {
    match (error.status(), reason(error)) {
        (Some(404 | 405), _) => OLD_SERVER.to_string(),
        (Some(409), _) => "Книг слишком много. Уберите пароль у тех, что больше не нужны, и повторите.".to_string(),
        (Some(422), "invalid_server_url") | (Some(503), "excel_public_url_not_configured") => NO_ADDRESS.to_string(),
        (Some(503), _) => NO_TEMPLATE.to_string(),
        (Some(422), _) => new_book_rules(),
        _ => request_error(error, "Не удалось создать книгу. Попробуйте ещё раз."),
    }
}

fn download_error(error: &ApiError) -> String {
    match (error.status(), reason(error)) {
        (Some(404), "workbook_not_found") => "Книга не найдена. Обновите список.".to_string(),
        (Some(404 | 405), _) => OLD_SERVER.to_string(),
        (Some(422), _) | (Some(503), "excel_public_url_not_configured") => NO_ADDRESS.to_string(),
        (Some(503), _) => NO_TEMPLATE.to_string(),
        _ => request_error(error, "Не удалось скачать книгу. Попробуйте ещё раз."),
    }
}

fn password_error(error: &ApiError) -> String {
    match error.status() {
        Some(404) => "Книга не найдена. Обновите список.".to_string(),
        Some(422) => password_length_error(),
        _ => request_error(error, "Не удалось сохранить пароль."),
    }
}

fn password_length_error() -> String {
    format!("Пароль должен содержать от {MIN_PASSWORD} до {MAX_PASSWORD} символов.")
}

fn new_book_rules() -> String {
    format!("Название — от 1 до {MAX_NAME} символов, пароль — от {MIN_PASSWORD} до {MAX_PASSWORD}.")
}

/// Why a new workbook cannot be asked for yet, if anything stands in the way.
fn new_book_problem(name: &str, password: &str) -> Option<String> {
    let name = name.trim().chars().count();
    if name == 0 || name > MAX_NAME {
        return Some(format!("Введите название книги: от 1 до {MAX_NAME} символов."));
    }
    if password.trim().is_empty() || !(MIN_PASSWORD..=MAX_PASSWORD).contains(&password.chars().count()) {
        return Some(password_length_error());
    }
    None
}

/// The name the save dialog offers: the workbook's own, minus what a file name
/// cannot hold.
pub fn file_name(name: &str) -> String {
    let stem = name.strip_suffix(".xlsm").unwrap_or(name);
    let clean: String = stem.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c }).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean = clean.trim_matches('.').trim();
    format!("{}.xlsm", if clean.is_empty() { DEFAULT_NAME } else { clean })
}

fn expired(grant: &ExcelGrant) -> bool {
    grant.expires_at.as_deref().and_then(format::parse_datetime).is_some_and(|at| at <= Utc::now().naive_utc())
}

fn live(grant: &ExcelGrant) -> bool {
    grant.revoked_at.is_none() && !expired(grant)
}

fn book_of(grant: &ExcelGrant) -> Option<&str> {
    grant.workbook_id.as_deref().filter(|id| !id.is_empty())
}

/// Every workbook worth showing, newest first: the ones the server made, then
/// the ones connected by hand that are still open somewhere. The name a person
/// gave wins over a computer's.
pub fn workbooks<'a>(activations: &'a [ExcelActivation], grants: &'a [ExcelGrant]) -> Vec<Book<'a>> {
    let computers = |id: &str| grants.iter().filter(|grant| live(grant) && book_of(grant) == Some(id)).collect::<Vec<_>>();
    let given = |id: &str| {
        let own = grants.iter().filter(|grant| book_of(grant) == Some(id) && !grant.device_bound && !grant.name.trim().is_empty());
        own.clone().find(|grant| live(grant)).or_else(|| own.clone().next()).map(|grant| grant.name.trim().to_string())
    };
    let mut made: Vec<&ExcelActivation> = activations.iter().filter(|activation| !activation.workbook_id.is_empty()).collect();
    made.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let mut books: Vec<Book<'a>> = made
        .into_iter()
        .map(|activation| {
            let id = activation.workbook_id.as_str();
            let name = activation.name.as_deref().map(str::trim).filter(|name| !name.is_empty()).map(str::to_string);
            Book { id, name: name.or_else(|| given(id)).unwrap_or_else(|| UNNAMED.to_string()), activation: Some(activation), computers: computers(id) }
        })
        .collect();
    for grant in grants.iter().filter(|grant| live(grant)) {
        let Some(id) = book_of(grant) else { continue };
        if books.iter().all(|book| book.id != id) {
            books.push(Book { id, name: given(id).unwrap_or_else(|| UNNAMED.to_string()), activation: None, computers: computers(id) });
        }
    }
    books
}

/// Connections that never reached a workbook: made by hand in an earlier
/// version and still waiting for their token to be typed in.
pub fn loose_grants(grants: &[ExcelGrant]) -> Vec<&ExcelGrant> {
    grants.iter().filter(|grant| live(grant) && book_of(grant).is_none()).collect()
}

fn when(raw: Option<&str>) -> String {
    match raw {
        None => "ещё не было".to_string(),
        Some(raw) => format::datetime_str(Some(raw)),
    }
}

fn send(message: Msg) -> Screen {
    Screen::Excel(message)
}

fn hint<'a>(value: impl iced::widget::text::IntoFragment<'a>) -> El<'a, Screen> {
    ui::muted(value).into()
}

fn fail<'a>(reason: &'a Option<String>) -> Option<El<'a, Screen>> {
    reason.as_deref().map(ui::error_line)
}

fn fail_for<'a>(reason: &'a Option<(String, String)>, book: &str) -> Option<El<'a, Screen>> {
    reason.as_ref().filter(|(id, _)| id == book).map(|(_, text)| ui::error_line(text.as_str()))
}

impl Transactions {
    pub(super) fn open_excel(&mut self, ctx: &Ctx) -> Task<Screen> {
        self.sheet = Sheet::Excel(Box::new(ExcelPanel::new()));
        load(ctx)
    }

    pub(super) fn excel_update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Screen> {
        let Sheet::Excel(panel) = &mut self.sheet else {
            // A reply to a panel that was closed: nothing typed there may linger.
            return Task::none();
        };
        match message {
            Msg::Load => {
                panel.loading = true;
                panel.list_error = None;
                return load(ctx);
            }
            Msg::Loaded(grants, activations) => {
                panel.loading = false;
                match grants {
                    Ok(items) => panel.grants = items,
                    Err(reason) => panel.list_error = Some(reason),
                }
                match activations {
                    Ok(items) => {
                        panel.activations = items;
                        panel.activations_error = None;
                    }
                    Err(reason) => panel.activations_error = Some(reason),
                }
            }
            Msg::Name(value) => panel.name = value,
            Msg::NewPassword(value) => panel.password = value,
            Msg::Create => {
                if panel.busy() || panel.loading {
                    return Task::none();
                }
                if let Some(problem) = new_book_problem(&panel.name, &panel.password) {
                    panel.error = Some(problem);
                    return Task::none();
                }
                panel.creating = true;
                panel.error = None;
                panel.created = None;
                let (api, name, password) = (ctx.api.clone(), panel.name.trim().to_string(), panel.password.clone());
                return Task::perform(async move { api.excel_create_workbook(&name, &password).await.map_err(|e| create_error(&e)) }, |result| send(Msg::Created(result)));
            }
            Msg::Created(result) => {
                panel.creating = false;
                match result {
                    Ok(activation) => {
                        // The password has done its work here; Excel asks for it next.
                        panel.password.clear();
                        let id = activation.workbook_id.clone();
                        panel.created = Some(id.clone());
                        panel.remember(activation);
                        return Task::done(send(Msg::Download(id)));
                    }
                    Err(reason) => panel.error = Some(reason),
                }
            }
            Msg::Download(book) => {
                if panel.busy() {
                    return Task::none();
                }
                let known = workbooks(&panel.activations, &panel.grants).into_iter().find(|b| b.id == book).map(|b| b.name);
                let fallback = file_name(known.as_deref().unwrap_or(DEFAULT_NAME));
                panel.downloading = Some(book.clone());
                panel.download_error = None;
                let api = ctx.api.clone();
                return Task::perform(
                    async move {
                        let result = api.excel_workbook_file(&book).await.map_err(|e| download_error(&e));
                        (book, result.map(|(name, bytes)| (name.as_deref().map_or(fallback, file_name), Parcel::new(bytes))))
                    },
                    |(book, result)| send(Msg::Downloaded(book, result)),
                );
            }
            Msg::Downloaded(book, Err(reason)) => {
                panel.downloading = None;
                panel.download_error = Some((book, reason));
            }
            Msg::Downloaded(book, Ok((name, parcel))) => {
                let Some(bytes) = parcel.take() else { return Task::none() };
                return Task::perform(crate::files::save_as(name, bytes), move |result| send(Msg::Saved(book.clone(), result)));
            }
            Msg::Saved(book, result) => {
                panel.downloading = None;
                match result {
                    Ok(Some(path)) => ctx.toast(Tone::Success, format!("Книга сохранена: {path}")),
                    Ok(None) => {}
                    Err(reason) => panel.download_error = Some((book, format!("Не удалось записать файл: {reason}"))),
                }
            }
            Msg::Password(book, value) => {
                if panel.password_saved.as_ref() == Some(&book) {
                    panel.password_saved = None;
                }
                panel.passwords.insert(book, value);
            }
            Msg::SavePassword(book) => {
                let password = panel.passwords.get(&book).cloned().unwrap_or_default();
                let length = password.chars().count();
                panel.password_saved = None;
                if !(MIN_PASSWORD..=MAX_PASSWORD).contains(&length) {
                    panel.password_error = Some((book, password_length_error()));
                    return Task::none();
                }
                if panel.busy() {
                    return Task::none();
                }
                panel.password_book = Some(book.clone());
                panel.password_error = None;
                let api = ctx.api.clone();
                return Task::perform(
                    async move {
                        let result = api.excel_set_password(&book, &password).await.map_err(|e| password_error(&e));
                        (book, result)
                    },
                    |(book, result)| send(Msg::PasswordSaved(book, result)),
                );
            }
            Msg::PasswordSaved(book, result) => {
                panel.password_book = None;
                match result {
                    Ok(mut activation) => {
                        // A server that answers without the name must not cost the book its name.
                        if let Some(known) = panel.activations.iter().find(|a| a.workbook_id == book) {
                            activation.name = activation.name.or_else(|| known.name.clone());
                            activation.created_at = activation.created_at.or_else(|| known.created_at.clone());
                        }
                        panel.remember(activation);
                        panel.passwords.remove(&book);
                        panel.password_saved = Some(book);
                    }
                    Err(reason) => panel.password_error = Some((book, reason)),
                }
            }
            Msg::RemovePassword(book) => {
                if panel.busy() {
                    return Task::none();
                }
                panel.password_book = Some(book.clone());
                panel.password_error = None;
                panel.password_saved = None;
                let api = ctx.api.clone();
                return Task::perform(
                    async move {
                        let result = api.excel_remove_password(&book).await.map_err(|e| request_error(&e, "Не удалось убрать пароль."));
                        (book, result)
                    },
                    |(book, result)| send(Msg::PasswordRemoved(book, result)),
                );
            }
            Msg::PasswordRemoved(book, result) => {
                panel.password_book = None;
                match result {
                    Ok(()) => {
                        panel.activations.retain(|a| a.workbook_id != book);
                        if panel.created.as_ref() == Some(&book) {
                            panel.created = None;
                        }
                    }
                    Err(reason) => panel.password_error = Some((book, reason)),
                }
            }
            Msg::Revoke(id) => {
                if panel.busy() {
                    return Task::none();
                }
                panel.revoking = Some(id.clone());
                panel.list_error = None;
                let api = ctx.api.clone();
                return Task::perform(
                    async move {
                        let result = api.excel_revoke_grant(&id).await.map_err(|e| request_error(&e, "Не удалось отключить компьютер."));
                        (id, result)
                    },
                    |(id, result)| send(Msg::Revoked(id, result)),
                );
            }
            Msg::Revoked(id, result) => {
                panel.revoking = None;
                match result {
                    Ok(()) => {
                        if let Some(grant) = panel.grants.iter_mut().find(|g| g.id == id) {
                            grant.revoked_at = Some(Utc::now().naive_utc().format("%Y-%m-%dT%H:%M:%S").to_string());
                        }
                    }
                    Err(reason) => panel.list_error = Some(reason),
                }
            }
            Msg::Close => {
                if !panel.busy() {
                    self.sheet = Sheet::None;
                }
            }
        }
        Task::none()
    }

    pub(super) fn excel_view<'a>(&'a self, panel: &'a ExcelPanel) -> El<'a, Screen> {
        let busy = panel.busy();
        let open = !busy && !panel.loading;
        let books = workbooks(&panel.activations, &panel.grants);

        let mut create = column![
            hint("Сервер соберёт файл, в котором уже записаны его адрес и номер книги. В Excel останется ввести пароль — один раз на каждом компьютере."),
            ui::field("Название книги", ui::input("Например, «Чеки — бухгалтерия»", &panel.name).on_input(|v| send(Msg::Name(v)))),
            ui::field("Пароль книги", ui::secret_input("Не короче 8 символов", &panel.password).on_input(|v| send(Msg::NewPassword(v))).on_submit(send(Msg::Create))),
            ui::btn_icon(Icon::Download, if panel.creating { "Создаём…" } else { "Создать и скачать книгу" }, Kind::Primary, open.then_some(send(Msg::Create))),
        ]
        .spacing(space::MD);
        if let Some(line) = fail(&panel.error) {
            create = create.push(line);
        }
        if let Some(book) = panel.created.as_deref().and_then(|id| books.iter().find(|book| book.id == id)) {
            create = create.push(next_steps(&book.name));
        }

        let refresh = open.then_some(send(Msg::Load));
        let mut list = column![row![ui::strong("Ваши книги"), ui::fill(), ui::btn_icon(Icon::Refresh, "Обновить", Kind::Ghost, refresh)].align_y(Alignment::Center)].spacing(space::MD);
        if panel.loading {
            list = list.push(hint("Загружаем книги…"));
        }
        if let Some(line) = fail(&panel.list_error) {
            list = list.push(line);
        }
        if let Some(line) = fail(&panel.activations_error) {
            list = list.push(line);
        }
        if !panel.loading && panel.list_error.is_none() && panel.activations_error.is_none() && books.is_empty() {
            list = list.push(hint("Книг пока нет. Создайте первую выше."));
        }
        for book in &books {
            list = list.push(ui::divider()).push(book_block(panel, book));
        }

        let mut body = column![ui::section("Новая книга", create), list].spacing(space::XL);
        let loose = loose_grants(&panel.grants);
        if !loose.is_empty() {
            let mut other = column![hint("Созданы вручную в прежней версии и ещё не подключены ни к одной книге. Если токен не используется, подключение лучше отозвать.")].spacing(space::MD);
            for grant in loose {
                other = other.push(ui::divider()).push(grant_line(panel, grant, "Ожидает подключения", "Отозвать", "Отзываем…"));
            }
            body = body.push(ui::section("Подключения без книги", other));
        }

        let done = (!busy).then_some(send(Msg::Close));
        let footer = row![ui::fill(), ui::btn("Готово", Kind::Secondary, done.clone())];
        let panel_view = ui::sheet_panel(
            "Excel",
            "Синхронизация с Excel",
            "Книга сама получает новые чеки и отправляет ваши правки, пока она открыта. Доступ ограничен вашими правами на транзакции.",
            send(Msg::Close),
            body,
            Some(footer.into()),
        );
        ui::sheet(iced::widget::Space::new(), panel_view, WIDTH, done)
    }
}

/// What to do with the file that was just saved.
fn next_steps<'a>(name: &str) -> El<'a, Screen> {
    let steps = column![
        ui::strong(format!("Книга «{name}» создана. Что дальше")),
        hint("1. Откройте сохранённый файл в Excel для Windows."),
        hint("2. Если Excel спросит о макросах, нажмите «Включить содержимое»."),
        hint("3. Введите пароль книги, когда она его попросит. Чеки загрузятся сами и будут обновляться, пока книга открыта."),
        hint("Тот же файл можно открыть на другом компьютере: там он тоже попросит пароль. Скачать его снова можно ниже, в списке книг."),
    ]
    .spacing(space::XS);
    ui::card(steps).into()
}

fn book_block<'a>(panel: &'a ExcelPanel, book: &Book<'a>) -> El<'a, Screen> {
    let id = book.id;
    let open = !panel.busy() && !panel.loading;
    let saving = panel.password_book.as_deref() == Some(id);
    let downloading = panel.downloading.as_deref() == Some(id);
    let draft = panel.passwords.get(id).map_or("", String::as_str);
    let state = match book.activation {
        Some(item) => match item.updated_at.as_deref() {
            Some(at) => format!("Пароль задан {}.", format::datetime_str(Some(at))),
            None => "Пароль задан.".to_string(),
        },
        None => "Пароль не задан: новый компьютер подключить нельзя.".to_string(),
    };
    let mut block = column![ui::strong(book.name.clone()), hint(state)].spacing(space::SM);
    if let Some(until) = book.activation.and_then(|a| a.locked_until.as_deref()) {
        block = block.push(hint(format!(
            "Слишком много неверных попыток: ввод пароля заблокирован до {}. Новый пароль снимает блокировку.",
            format::datetime_str(Some(until))
        )));
    }

    if book.computers.is_empty() {
        block = block.push(hint("Книга ещё не открыта ни на одном компьютере."));
    } else {
        block = block.push(hint(format!("Компьютеры, на которых книга открыта: {}", book.computers.len())));
        for grant in &book.computers {
            block = block.push(grant_line(panel, grant, "Подключён", "Отключить", "Отключаем…"));
        }
    }

    let typed = id.to_string();
    let input = ui::secret_input("Не короче 8 символов", draft).on_input(move |value| send(Msg::Password(typed.clone(), value)));
    let caption = match (saving, book.activation.is_some()) {
        (true, _) => "Сохраняем…",
        (false, true) => "Сменить пароль",
        (false, false) => "Задать пароль",
    };
    let can_save = open && !draft.trim().is_empty();
    let mut actions = row![ui::btn_icon(Icon::Key, caption, Kind::Primary, can_save.then(|| send(Msg::SavePassword(id.to_string()))))].spacing(space::SM);
    if book.activation.is_some() {
        actions = actions.push(ui::btn("Убрать пароль", Kind::Secondary, open.then(|| send(Msg::RemovePassword(id.to_string())))));
    }
    actions = actions.push(ui::btn_icon(Icon::Download, if downloading { "Скачиваем…" } else { "Скачать файл" }, Kind::Secondary, open.then(|| send(Msg::Download(id.to_string())))));
    block = block.push(ui::field(if book.activation.is_some() { "Новый пароль" } else { "Пароль" }, input)).push(actions);
    if panel.password_saved.as_deref() == Some(id) {
        block = block.push(hint("Пароль сохранён. Компьютеры, где книга уже открыта, продолжают работать."));
    }
    if let Some(line) = fail_for(&panel.password_error, id) {
        block = block.push(line);
    }
    if let Some(line) = fail_for(&panel.download_error, id) {
        block = block.push(line);
    }
    block.into()
}

/// One connection: a computer under its workbook, or a token nobody used yet.
fn grant_line<'a>(panel: &'a ExcelPanel, grant: &'a ExcelGrant, status: &'a str, withdraw: &'a str, withdrawing: &'a str) -> El<'a, Screen> {
    let name = match grant.device_name.as_deref().filter(|device| grant.device_bound && !device.trim().is_empty()) {
        Some(device) => device,
        None if grant.name.trim().is_empty() => "Без названия",
        None => grant.name.as_str(),
    };
    let term = match grant.expires_at.as_deref() {
        None => String::new(),
        Some(at) => format!(" · до {}", format::datetime_str(Some(at))),
    };
    let info = column![ui::label(name), hint(format!("{status}{term} · последняя связь: {}", when(grant.last_used_at.as_deref())))].spacing(space::XXS);
    let busy_here = panel.revoking.as_deref() == Some(grant.id.as_str());
    let open = !panel.busy() && !panel.loading;
    let button = ui::btn_icon(Icon::Unlink, if busy_here { withdrawing } else { withdraw }, Kind::Secondary, open.then(|| send(Msg::Revoke(grant.id.clone()))));
    row![container(info).width(Fill), button].spacing(space::MD).align_y(Alignment::Center).into()
}

fn load(ctx: &Ctx) -> Task<Screen> {
    let api = ctx.api.clone();
    Task::perform(
        async move {
            let grants = api.excel_grants().await.map_err(|e| request_error(&e, "Не удалось загрузить подключения."));
            let activations = api.excel_activations().await.map_err(|e| request_error(&e, "Не удалось загрузить книги."));
            (grants, activations)
        },
        |(grants, activations)| send(Msg::Loaded(grants, activations)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(name: &str, book: Option<&str>, device_bound: bool) -> ExcelGrant {
        ExcelGrant { id: format!("{name}-id"), name: name.into(), workbook_id: book.map(str::to_string), device_bound, ..Default::default() }
    }

    fn made(id: &str, name: Option<&str>, created_at: &str) -> ExcelActivation {
        ExcelActivation { workbook_id: id.into(), name: name.map(str::to_string), created_at: Some(created_at.into()), ..Default::default() }
    }

    fn http(status: u16, detail: &str) -> ApiError {
        ApiError::Http { status, code: None, message: detail.into() }
    }

    #[test]
    fn a_made_workbook_is_listed_before_any_computer_opened_it() {
        let activations = [made("w1", Some("Касса"), "2026-11-10T10:00:00"), made("w2", Some("Склад"), "2026-11-11T10:00:00")];
        let books = workbooks(&activations, &[]);
        assert_eq!(books.iter().map(|b| (b.id, b.name.as_str(), b.computers.len())).collect::<Vec<_>>(), [("w2", "Склад", 0), ("w1", "Касса", 0)]);
        assert!(books.iter().all(|b| b.activation.is_some()));
    }

    #[test]
    fn a_book_keeps_the_name_a_person_gave_it() {
        let grants = [grant("ПК Анвара", Some("w1"), true), grant("Касса", Some("w1"), false), grant("Новая", None, false), grant("ПК склада", Some("w2"), true)];
        // An activation without a name is one set on a hand-connected workbook.
        let activations = [made("w1", None, "2026-01-01T00:00:00")];
        let books = workbooks(&activations, &grants);
        assert_eq!(books.iter().map(|b| (b.id, b.name.as_str())).collect::<Vec<_>>(), [("w1", "Касса"), ("w2", UNNAMED)]);
        assert_eq!(books[0].computers.len(), 2);
        assert!(books[1].activation.is_none());
        assert_eq!(loose_grants(&grants).iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Новая"]);
    }

    #[test]
    fn a_withdrawn_or_expired_computer_is_not_listed() {
        let gone = ExcelGrant { revoked_at: Some("2026-01-01T00:00:00".into()), ..grant("ПК 1", Some("w1"), true) };
        let old = ExcelGrant { expires_at: Some("2020-01-01T00:00:00".into()), ..grant("ПК 2", Some("w1"), true) };
        let alive = grant("ПК 3", Some("w1"), true);
        let activations = [made("w1", Some("Касса"), "2026-01-01T00:00:00")];
        let grants = [gone.clone(), old.clone(), alive];
        let books = workbooks(&activations, &grants);
        assert_eq!(books[0].computers.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["ПК 3"]);
        // A hand-connected workbook nobody has open any more has nothing left to manage.
        assert!(workbooks(&[], &[gone, old]).is_empty());
    }

    #[test]
    fn the_file_takes_the_workbooks_name() {
        assert_eq!(file_name("Чеки — бухгалтерия"), "Чеки — бухгалтерия.xlsm");
        assert_eq!(file_name("Касса: 1/2?.xlsm"), "Касса 1 2.xlsm");
        assert_eq!(file_name(" .. "), "Чеки.xlsm");
    }

    #[test]
    fn a_new_workbook_needs_a_name_and_a_real_password() {
        assert!(new_book_problem("Касса", "12345678").is_none());
        assert!(new_book_problem("  ", "12345678").unwrap().contains("название"));
        assert!(new_book_problem(&"я".repeat(81), "12345678").unwrap().contains("название"));
        assert!(new_book_problem("Касса", "1234567").unwrap().contains("от 8 до 128"));
        assert!(new_book_problem("Касса", "        ").unwrap().contains("от 8 до 128"));
    }

    #[test]
    fn errors_speak_to_the_person() {
        assert_eq!(request_error(&http(429, ""), "x"), "Слишком много запросов. Попробуйте немного позже.");
        assert_eq!(request_error(&http(500, ""), "запасной"), "запасной");
        assert!(password_error(&http(422, "")).contains("от 8 до 128"));
        // A server from before this panel answers the new address with 404 or 405.
        assert_eq!(create_error(&http(404, "Not Found")), OLD_SERVER);
        assert_eq!(create_error(&http(405, "Method Not Allowed")), OLD_SERVER);
        assert_eq!(download_error(&http(404, "Not Found")), OLD_SERVER);
        assert!(download_error(&http(404, "workbook_not_found")).contains("не найдена"));
        assert_eq!(create_error(&http(422, "invalid_server_url")), NO_ADDRESS);
        assert_eq!(create_error(&http(503, "excel_public_url_not_configured")), NO_ADDRESS);
        assert_eq!(create_error(&http(503, "excel_workbook_not_built")), NO_TEMPLATE);
        assert!(create_error(&http(409, "too_many_excel_workbooks")).contains("слишком много"));
        assert!(create_error(&http(422, "")).contains("от 8 до 128"));
    }
}
