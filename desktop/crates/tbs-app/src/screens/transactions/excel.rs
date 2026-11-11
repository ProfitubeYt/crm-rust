//! The sheet that connects an Excel workbook to the transactions.

use std::collections::HashMap;

use chrono::Utc;
use iced::widget::{column, container, row};
use iced::{Alignment, Fill, Task};
use tbs_core::api::{ApiError, ExcelActivation, ExcelGrant, ExcelGrantCreated};
use tbs_core::format;

use super::{Msg as Screen, Parcel, Sheet, Transactions};
use crate::ctx::Ctx;
use crate::theme::{Tone, space};
use crate::ui::{self, El, Icon, Kind};

const WIDTH: f32 = 620.0;
const WORKBOOK_NAME: &str = "Чеки — синхронизация.xlsm";
const MIN_PASSWORD: usize = 8;
const MAX_PASSWORD: usize = 128;
const TERMS: [(u32, &str); 4] = [(30, "30 дней"), (90, "90 дней"), (180, "180 дней"), (365, "1 год")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Copied {
    Server,
    Token,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Load,
    Loaded(Result<Vec<ExcelGrant>, String>, Result<Vec<ExcelActivation>, String>),
    Download,
    Downloaded(Result<Parcel<Vec<u8>>, String>),
    Saved(Result<Option<String>, String>),
    Name(String),
    Term(u32),
    Create,
    Created(Result<ExcelGrantCreated, String>),
    Copy(Copied),
    Password(String, String),
    SavePassword(String),
    PasswordSaved(String, Result<ExcelActivation, String>),
    RemovePassword(String),
    PasswordRemoved(String, Result<(), String>),
    Revoke(String),
    Revoked(String, Result<(), String>),
    Close,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Book {
    pub id: String,
    pub name: String,
}

pub struct ExcelPanel {
    grants: Vec<ExcelGrant>,
    activations: HashMap<String, ExcelActivation>,
    loading: bool,
    list_error: Option<String>,
    activations_error: Option<String>,
    name: String,
    term: u32,
    creating: bool,
    error: Option<String>,
    connection: Option<ExcelGrantCreated>,
    issued: bool,
    copied: Option<Copied>,
    downloading: bool,
    download_error: Option<String>,
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
            activations: HashMap::new(),
            loading: true,
            list_error: None,
            activations_error: None,
            name: "Моя книга Excel".to_string(),
            term: 90,
            creating: false,
            error: None,
            connection: None,
            issued: false,
            copied: None,
            downloading: false,
            download_error: None,
            revoking: None,
            passwords: HashMap::new(),
            password_book: None,
            password_saved: None,
            password_error: None,
        }
    }

    fn busy(&self) -> bool {
        self.creating || self.revoking.is_some() || self.password_book.is_some()
    }
}

/// What a person should read when a request fails.
fn request_error(error: &ApiError, fallback: &str) -> String {
    match error.status() {
        Some(401) => "Войдите в приложение заново.".to_string(),
        Some(403) => "Доступ к подключению Excel закрыт. Проверьте доступ к транзакциям.".to_string(),
        Some(429) => "Слишком много запросов. Попробуйте немного позже.".to_string(),
        _ if error.is_network() => "Нет связи с сервером. Проверьте подключение и повторите попытку.".to_string(),
        _ => fallback.to_string(),
    }
}

fn download_error(error: &ApiError) -> String {
    match error.status() {
        Some(404 | 503) => format!("Книга пока недоступна для скачивания. Если вы уже получили файл «{WORKBOOK_NAME}», подключите его ниже."),
        _ => request_error(error, "Не удалось скачать книгу. Попробуйте ещё раз."),
    }
}

fn password_error(error: &ApiError) -> String {
    match error.status() {
        Some(404) => "Книга не найдена. Обновите список подключений.".to_string(),
        Some(422) => password_length_error(),
        _ => request_error(error, "Не удалось сохранить пароль."),
    }
}

fn password_length_error() -> String {
    format!("Пароль должен содержать от {MIN_PASSWORD} до {MAX_PASSWORD} символов.")
}

/// One entry per workbook; the name a person gave it wins over a device's copy.
pub fn connected_workbooks(grants: &[ExcelGrant]) -> Vec<Book> {
    let mut books: Vec<Book> = Vec::new();
    for grant in grants {
        let Some(id) = grant.workbook_id.as_deref().filter(|id| !id.is_empty()) else { continue };
        match books.iter_mut().find(|book| book.id == id) {
            Some(book) => {
                if !grant.device_bound {
                    book.name = grant.name.clone();
                }
            }
            None => books.push(Book { id: id.to_string(), name: grant.name.clone() }),
        }
    }
    books
}

fn when(raw: Option<&str>) -> String {
    match raw {
        None => "Ещё не подключалась".to_string(),
        Some(raw) => format::datetime_str(Some(raw)),
    }
}

fn expired(grant: &ExcelGrant) -> bool {
    grant.expires_at.as_deref().and_then(format::parse_datetime).is_some_and(|at| at <= Utc::now().naive_utc())
}

fn grant_status(grant: &ExcelGrant) -> &'static str {
    if grant.revoked_at.is_some() {
        "Отозвано"
    } else if expired(grant) {
        "Срок истёк"
    } else if grant.workbook_id.is_some() {
        "Подключено"
    } else {
        "Ожидает подключения"
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

/// Asks where to put the workbook, then writes it.
async fn save_workbook(bytes: Vec<u8>) -> Result<Option<String>, String> {
    let Some(target) = rfd::AsyncFileDialog::new().set_file_name(WORKBOOK_NAME).add_filter("Книга Excel", &["xlsm"]).save_file().await else { return Ok(None) };
    let path = target.path().to_path_buf();
    tokio::fs::write(&path, bytes).await.map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}

impl Transactions {
    pub(super) fn open_excel(&mut self, ctx: &Ctx) -> Task<Screen> {
        self.sheet = Sheet::Excel(Box::new(ExcelPanel::new()));
        load(ctx)
    }

    pub(super) fn excel_update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Screen> {
        let Sheet::Excel(panel) = &mut self.sheet else {
            // A reply to a panel that was closed: the token must not linger anywhere.
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
                        panel.activations = items.into_iter().map(|a| (a.workbook_id.clone(), a)).collect();
                        panel.activations_error = None;
                    }
                    Err(reason) => panel.activations_error = Some(reason),
                }
            }
            Msg::Download => {
                if panel.downloading {
                    return Task::none();
                }
                panel.downloading = true;
                panel.download_error = None;
                let api = ctx.api.clone();
                return Task::perform(
                    async move { api.excel_workbook().await.map(Parcel::new).map_err(|e| download_error(&e)) },
                    |result| send(Msg::Downloaded(result)),
                );
            }
            Msg::Downloaded(Err(reason)) => {
                panel.downloading = false;
                panel.download_error = Some(reason);
            }
            Msg::Downloaded(Ok(parcel)) => {
                let Some(bytes) = parcel.take() else { return Task::none() };
                return Task::perform(save_workbook(bytes), |result| send(Msg::Saved(result)));
            }
            Msg::Saved(result) => {
                panel.downloading = false;
                match result {
                    Ok(Some(path)) => ctx.toast(Tone::Success, format!("Книга сохранена: {path}")),
                    Ok(None) => {}
                    Err(reason) => panel.download_error = Some(format!("Не удалось записать файл: {reason}")),
                }
            }
            Msg::Name(value) => panel.name = value,
            Msg::Term(value) => panel.term = value,
            Msg::Create => {
                let name = panel.name.trim().to_string();
                if name.is_empty() || name.chars().count() > 120 {
                    panel.error = Some("Введите название книги: от 1 до 120 символов.".into());
                    return Task::none();
                }
                if panel.busy() || panel.loading {
                    return Task::none();
                }
                panel.creating = true;
                panel.error = None;
                let (api, days) = (ctx.api.clone(), panel.term);
                return Task::perform(
                    async move {
                        api.excel_issue_grant(&name, days)
                            .await
                            .map_err(|e| request_error(&e, "Не удалось создать подключение. Обновите список перед повторной попыткой."))
                    },
                    |result| send(Msg::Created(result)),
                );
            }
            Msg::Created(result) => {
                panel.creating = false;
                match result {
                    Ok(created) => {
                        panel.grants.insert(0, created.grant.clone());
                        panel.connection = Some(created);
                        panel.issued = true;
                    }
                    Err(reason) => panel.error = Some(reason),
                }
            }
            Msg::Copy(field) => {
                let Some(connection) = &panel.connection else { return Task::none() };
                panel.copied = Some(field);
                let value = if field == Copied::Token { connection.token.clone() } else { connection.server_url.clone() };
                return iced::clipboard::write(value);
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
                    Ok(activation) => {
                        panel.activations.insert(book.clone(), activation);
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
                        panel.activations.remove(&book);
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
                        let result = api.excel_revoke_grant(&id).await.map_err(|e| request_error(&e, "Не удалось отозвать подключение."));
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
        let mut download = column![
            hint("Откройте её в Excel для Windows. После подключения синхронизация работает, пока книга открыта."),
            ui::btn_icon(
                Icon::Download,
                if panel.downloading { "Скачиваем…" } else { "Скачать книгу .xlsm" },
                Kind::Secondary,
                (!panel.downloading).then_some(send(Msg::Download)),
            ),
        ]
        .spacing(space::SM);
        if let Some(line) = fail(&panel.download_error) {
            download = download.push(line);
        }

        let mut connect = column![].spacing(space::MD);
        if !panel.issued {
            let enabled = !busy && !panel.loading && panel.list_error.is_none();
            connect = connect
                .push(ui::field("Название книги", ui::input("", &panel.name).on_input(|v| send(Msg::Name(v)))))
                .push(ui::field(
                    "Срок доступа",
                    TERMS.iter().fold(row![].spacing(space::XS), |line, (days, name)| line.push(ui::chip(*name, panel.term == *days, send(Msg::Term(*days))))),
                ))
                .push(hint("Для каждой книги создавайте отдельное подключение. Его можно отозвать в любой момент."))
                .push(ui::btn(if panel.creating { "Создаём…" } else { "Создать подключение" }, Kind::Primary, enabled.then_some(send(Msg::Create))));
        }
        if let Some(connection) = &panel.connection {
            let line = |caption: &'a str, value: &'a str, field: Copied| -> El<'a, Screen> {
                let done = panel.copied == Some(field);
                let button = ui::btn_icon(if done { Icon::Check } else { Icon::Copy }, if done { "Скопировано" } else { "Копировать" }, Kind::Secondary, Some(send(Msg::Copy(field))));
                ui::field(caption, row![ui::input("", value), button].spacing(space::SM).align_y(Alignment::Center)).into()
            };
            connect = connect
                .push(hint("В книге нажмите «Подключить» и вставьте адрес сервера и токен. Токен показывается только сейчас: после закрытия этой панели его нельзя посмотреть снова."))
                .push(line("Адрес сервера", &connection.server_url, Copied::Server))
                .push(line("Токен подключения", &connection.token, Copied::Token))
                .push(hint("Токен даёт доступ к вашим чекам. Вставляйте его только в свою книгу."));
        } else if panel.issued {
            connect = connect.push(hint("Токен больше не показывается. Для нового подключения закройте эту панель и откройте её снова."));
        }
        if let Some(line) = fail(&panel.error) {
            connect = connect.push(line);
        }

        let books = connected_workbooks(&panel.grants);
        let mut passwords = column![hint(
            "На новом компьютере книга показывает только лист «Доступ», пока не введён пароль. После верного пароля чеки загружаются, а компьютер запоминается и больше пароль не спрашивает. Копия файла на другом компьютере снова попросит пароль."
        )]
        .spacing(space::MD);
        if let Some(line) = fail(&panel.activations_error) {
            passwords = passwords.push(line);
        }
        if !panel.loading && panel.list_error.is_none() && books.is_empty() {
            passwords = passwords.push(hint("Пароль можно задать после первого подключения книги."));
        }
        for book in books {
            passwords = passwords.push(ui::divider()).push(password_block(panel, book));
        }

        let refresh = (!panel.loading && !busy).then_some(send(Msg::Load));
        let mut grants = column![
            row![ui::strong("Ваши подключения"), ui::fill(), ui::btn_icon(Icon::Refresh, "Обновить", Kind::Ghost, refresh)].align_y(Alignment::Center),
        ]
        .spacing(space::MD);
        if panel.loading {
            grants = grants.push(hint("Загружаем подключения…"));
        }
        if let Some(line) = fail(&panel.list_error) {
            grants = grants.push(line);
        }
        if !panel.loading && panel.list_error.is_none() && panel.grants.is_empty() {
            grants = grants.push(hint("Подключённых книг пока нет."));
        }
        for grant in &panel.grants {
            grants = grants.push(ui::divider()).push(grant_block(panel, grant));
        }

        let body = column![
            ui::section("1. Скачайте книгу", download),
            ui::section("2. Подключите Excel", connect),
            ui::section("3. Защитите книгу паролем", passwords),
            grants,
        ]
        .spacing(space::XL);

        let done = (!busy).then_some(send(Msg::Close));
        let footer = row![ui::fill(), ui::btn("Готово", Kind::Secondary, done.clone())];
        let panel_view = ui::sheet_panel(
            "Excel",
            "Синхронизация с Excel",
            "Книга получает новые чеки и отправляет ваши изменения в приложение. Доступ ограничен вашими правами на транзакции.",
            send(Msg::Close),
            body,
            Some(footer.into()),
        );
        ui::sheet(iced::widget::Space::new(), panel_view, WIDTH, done)
    }
}

fn password_block<'a>(panel: &'a ExcelPanel, book: Book) -> El<'a, Screen> {
    let activation = panel.activations.get(&book.id);
    let saving = panel.password_book.as_ref() == Some(&book.id);
    let draft = panel.passwords.get(&book.id).map_or("", String::as_str);
    let open = !panel.busy() && !panel.loading;
    let state = match activation {
        Some(item) => match item.updated_at.as_deref() {
            Some(at) => format!("Пароль задан {}.", format::datetime_str(Some(at))),
            None => "Пароль задан.".to_string(),
        },
        None => "Пароль не задан: на новом компьютере книгу открыть нельзя.".to_string(),
    };
    let id = book.id.clone();
    let input = ui::secret_input("Не короче 8 символов", draft).on_input(move |value| send(Msg::Password(id.clone(), value)));
    let caption = match (saving, activation.is_some()) {
        (true, _) => "Сохраняем…",
        (false, true) => "Сменить пароль",
        (false, false) => "Задать пароль",
    };
    let can_save = open && !draft.trim().is_empty();
    let mut actions = row![ui::btn_icon(Icon::Key, caption, Kind::Primary, can_save.then(|| send(Msg::SavePassword(book.id.clone()))))].spacing(space::SM);
    if activation.is_some() {
        actions = actions.push(ui::btn("Убрать пароль", Kind::Secondary, open.then(|| send(Msg::RemovePassword(book.id.clone())))));
    }
    let mut block = column![ui::strong(book.name), hint(state)].spacing(space::SM);
    if let Some(until) = activation.and_then(|a| a.locked_until.as_deref()) {
        block = block.push(hint(format!(
            "Слишком много неверных попыток: ввод пароля заблокирован до {}. Новый пароль снимает блокировку.",
            format::datetime_str(Some(until))
        )));
    }
    block = block.push(ui::field(if activation.is_some() { "Новый пароль" } else { "Пароль" }, input)).push(actions);
    if panel.password_saved.as_ref() == Some(&book.id) {
        block = block.push(hint("Пароль сохранён. Уже активированные компьютеры продолжают работать."));
    }
    if let Some((_, reason)) = panel.password_error.as_ref().filter(|(id, _)| *id == book.id) {
        block = block.push(ui::error_line(reason.as_str()));
    }
    block.into()
}

fn grant_block<'a>(panel: &'a ExcelPanel, grant: &'a ExcelGrant) -> El<'a, Screen> {
    let term = match grant.expires_at.as_deref() {
        None => "Бессрочно".to_string(),
        Some(at) => format!("до {}", format::datetime_str(Some(at))),
    };
    let mut info = column![ui::strong(grant.name.as_str()), hint(format!("{} · {term}", grant_status(grant)))].spacing(space::XXS);
    if grant.device_bound {
        let device = grant.device_name.as_deref().filter(|d| !d.is_empty()).unwrap_or("без имени");
        info = info.push(hint(format!("Работает только на компьютере «{device}». Отзовите, чтобы он снова попросил пароль.")));
    }
    info = info.push(hint(format!("Последняя связь: {}", when(grant.last_used_at.as_deref()))));
    let mut line = row![container(info).width(Fill)].spacing(space::MD);
    if grant.revoked_at.is_none() {
        let revoking = panel.revoking.as_ref() == Some(&grant.id);
        let open = !panel.busy() && !panel.loading;
        line = line.push(ui::btn_icon(Icon::Unlink, if revoking { "Отзываем…" } else { "Отозвать" }, Kind::Secondary, open.then(|| send(Msg::Revoke(grant.id.clone())))));
    }
    line.into()
}

fn load(ctx: &Ctx) -> Task<Screen> {
    let api = ctx.api.clone();
    Task::perform(
        async move {
            let grants = api.excel_grants().await.map_err(|e| request_error(&e, "Не удалось загрузить подключения."));
            let activations = api.excel_activations().await.map_err(|e| request_error(&e, "Не удалось загрузить пароли книг."));
            (grants, activations)
        },
        |(grants, activations)| send(Msg::Loaded(grants, activations)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(name: &str, book: Option<&str>, device_bound: bool) -> ExcelGrant {
        ExcelGrant { name: name.into(), workbook_id: book.map(str::to_string), device_bound, ..Default::default() }
    }

    #[test]
    fn a_book_keeps_the_name_a_person_gave_it() {
        let grants = [grant("ПК Анвара", Some("w1"), true), grant("Касса", Some("w1"), false), grant("Новая", None, false), grant("Склад", Some("w2"), true)];
        let books = connected_workbooks(&grants);
        assert_eq!(books, [Book { id: "w1".into(), name: "Касса".into() }, Book { id: "w2".into(), name: "Склад".into() }]);
    }

    #[test]
    fn grant_status_follows_its_life() {
        assert_eq!(grant_status(&grant("a", None, false)), "Ожидает подключения");
        assert_eq!(grant_status(&grant("a", Some("w"), false)), "Подключено");
        let old = ExcelGrant { expires_at: Some("2020-01-01T00:00:00".into()), ..grant("a", Some("w"), false) };
        assert_eq!(grant_status(&old), "Срок истёк");
        let gone = ExcelGrant { revoked_at: Some("2020-01-01T00:00:00".into()), ..old };
        assert_eq!(grant_status(&gone), "Отозвано");
    }

    #[test]
    fn errors_speak_to_the_person() {
        let http = |status| ApiError::Http { status, code: None, message: String::new() };
        assert_eq!(request_error(&http(429), "x"), "Слишком много запросов. Попробуйте немного позже.");
        assert_eq!(request_error(&http(500), "запасной"), "запасной");
        assert!(download_error(&http(404)).contains(WORKBOOK_NAME));
        assert!(password_error(&http(422)).contains("от 8 до 128"));
    }
}
