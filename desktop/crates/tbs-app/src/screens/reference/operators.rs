//! Operator → application mapping, the source the recognition cascade reads.

use std::borrow::Cow;
use std::time::Duration;

use chrono::Local;
use iced::widget::{Id, Space, column, container, operation, row};
use iced::{Alignment, Fill, Point, Rectangle, Size, Task};
use serde_json::json;
use tbs_core::api::{Api, ImportReply, OperatorPage, OperatorRef};
use tbs_core::format::{count, plural};

use crate::ctx::{Ctx, text_error};
use crate::files;
use crate::grid::{self, Align, CellStyle, GridColumn, GridData, GridEvent, GridKey, Selection};
use crate::theme::{self, DARK, LIGHT, Tone, space};
use crate::ui::{self, El, Icon, Kind};

const SEARCH_ID: &str = "operators-search";
const EDITOR_ID: &str = "operators-editor";
const DEBOUNCE: Duration = Duration::from_millis(240);
const FORM_WIDTH: f32 = 420.0;
const MENU: Size = Size::new(200.0, 84.0);
const FOOTER_HEIGHT: f32 = 32.0;
const PAGE_DEFAULT: PageSize = PageSize(Some(100));
const SIZES: [PageSize; 7] = [PageSize(None), PageSize(Some(20)), PageSize(Some(50)), PageSize(Some(100)), PageSize(Some(200)), PageSize(Some(500)), PageSize(Some(1000))];

const COL_OPERATOR: usize = 0;
const COL_APPLICATION: usize = 1;
const COL_P2P: usize = 2;
const COL_ACTIVE: usize = 3;

pub fn focus_search<T: Send + 'static>() -> Task<T> {
    operation::focus(Id::new(SEARCH_ID))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    All,
    Active,
    Inactive,
    P2p,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageSize(Option<i64>);

impl std::fmt::Display for PageSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(size) => write!(f, "{size} / стр"),
            None => f.write_str("Все"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    total: usize,
    active: usize,
    inactive: usize,
    p2p: usize,
    applications: usize,
}

impl Stats {
    fn of(items: &[OperatorRef]) -> Self {
        let active = items.iter().filter(|o| o.is_active).count();
        let mut names: Vec<&str> = items.iter().map(|o| o.application_name.as_str()).filter(|n| !n.is_empty()).collect();
        names.sort_unstable();
        names.dedup();
        Self { total: items.len(), active, inactive: items.len() - active, p2p: items.iter().filter(|o| o.is_p2p).count(), applications: names.len() }
    }

    fn active_share(&self) -> usize {
        if self.total == 0 { 0 } else { (self.active * 100 + self.total / 2) / self.total }
    }
}

#[derive(Clone, Debug)]
pub struct Form {
    id: Option<i64>,
    operator: String,
    application: String,
    is_p2p: bool,
    is_active: bool,
    error: Option<String>,
    busy: bool,
}

impl Form {
    fn empty() -> Self {
        Self { id: None, operator: String::new(), application: String::new(), is_p2p: false, is_active: true, error: None, busy: false }
    }

    fn of(item: &OperatorRef) -> Self {
        Self {
            id: Some(item.id),
            operator: item.operator_name.clone(),
            application: item.application_name.clone(),
            is_p2p: item.is_p2p,
            is_active: item.is_active,
            error: None,
            busy: false,
        }
    }

    fn body(&self) -> Option<serde_json::Value> {
        let (operator, application) = (self.operator.trim(), self.application.trim());
        if operator.is_empty() || application.is_empty() {
            return None;
        }
        Some(json!({ "operator_name": operator, "application_name": application, "is_p2p": self.is_p2p, "is_active": self.is_active }))
    }
}

struct Editor {
    id: i64,
    col: usize,
    value: String,
    original: String,
    rect: Rectangle,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Escape,
    Search(String),
    SearchSettled(u64),
    Status(Status),
    Application(String),
    Compact(bool),
    PageSize(PageSize),
    Page(i64),
    Loaded(u64, Result<OperatorPage, String>),
    AllLoaded(Result<Vec<OperatorRef>, String>),
    Apps(Result<Vec<String>, String>),
    Grid(GridEvent),
    EditText(String),
    EditCommit,
    Saved(Result<OperatorRef, String>),
    ClearSelection,
    Bulk(bool),
    BulkDone(bool, Result<usize, String>),
    AskDelete(Vec<i64>),
    CancelDelete,
    ConfirmDelete,
    Deleted(Result<usize, String>),
    OpenAdd,
    OpenEdit(i64),
    CloseForm,
    FormOperator(String),
    FormApplication(String),
    FormApp(String),
    FormP2p(bool),
    FormActive(bool),
    FormSubmit,
    FormDone(Result<OperatorRef, String>),
    CloseMenu,
    Export,
    Exported(Result<Option<String>, String>),
    Import,
    Imported(Result<Option<ImportReply>, String>),
    DismissErrors,
}

pub struct Operators {
    loaded: bool,
    error: Option<String>,
    rows: Vec<OperatorRef>,
    all: Vec<OperatorRef>,
    stats: Option<Stats>,
    applications: Vec<String>,
    total: i64,
    page: i64,
    page_size: PageSize,
    search: String,
    search_token: u64,
    load_token: u64,
    status: Status,
    application: Option<String>,
    compact: bool,
    selection: Selection,
    sort: Option<(usize, bool)>,
    columns: Vec<GridColumn>,
    editor: Option<Editor>,
    form: Option<Form>,
    ask: Option<(Vec<i64>, String, String, &'static str)>,
    menu: Option<(i64, Point)>,
    importing: bool,
    import_errors: Vec<String>,
}

struct Rows<'a> {
    rows: &'a [OperatorRef],
    dark: bool,
}

impl GridData for Rows<'_> {
    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn cell(&self, row: usize, col: usize) -> Cow<'_, str> {
        let Some(item) = self.rows.get(row) else { return Cow::Borrowed("") };
        match col {
            COL_OPERATOR => Cow::Borrowed(&item.operator_name),
            COL_APPLICATION => Cow::Borrowed(&item.application_name),
            COL_P2P => Cow::Borrowed(if item.is_p2p { "Да" } else { "—" }),
            _ => Cow::Borrowed(if item.is_active { "Да" } else { "Нет" }),
        }
    }

    fn cell_style(&self, row: usize, col: usize) -> CellStyle {
        let palette = if self.dark { &DARK } else { &LIGHT };
        let dimmed = self.rows.get(row).is_some_and(|item| !item.is_active);
        let color = match col {
            COL_ACTIVE if !dimmed => Some(palette.success),
            COL_P2P if self.rows.get(row).is_some_and(|item| item.is_p2p) && !dimmed => Some(palette.accent),
            _ if dimmed => Some(palette.text3),
            _ => None,
        };
        CellStyle { color, ..CellStyle::default() }
    }
}

fn base_columns() -> Vec<GridColumn> {
    vec![
        GridColumn::new("Оператор", 420.0).medium().sortable(None),
        GridColumn::new("Приложение", 280.0).sortable(None),
        GridColumn::new("P2P", 90.0).align(Align::Center).sortable(None),
        GridColumn::new("Активен", 100.0).align(Align::Center).sortable(None),
    ]
}

fn sort_rows(rows: &mut [OperatorRef], sort: Option<(usize, bool)>) {
    let Some((col, descending)) = sort else { return };
    rows.sort_by(|a, b| {
        let order = match col {
            COL_OPERATOR => a.operator_name.to_lowercase().cmp(&b.operator_name.to_lowercase()),
            COL_APPLICATION => a.application_name.to_lowercase().cmp(&b.application_name.to_lowercase()),
            COL_P2P => a.is_p2p.cmp(&b.is_p2p),
            _ => a.is_active.cmp(&b.is_active),
        };
        if descending { order.reverse() } else { order }
    });
}

fn page_count(total: i64, size: PageSize) -> i64 {
    match size.0 {
        Some(size) if size > 0 => ((total + size - 1) / size).max(1),
        _ => 1,
    }
}

/// "21–40 из 57", or "0 из 0" on an empty list.
fn range_label(page: i64, size: PageSize, shown: usize, total: i64) -> String {
    if shown == 0 {
        return "0 из 0".to_owned();
    }
    let first = size.0.map_or(1, |size| (page - 1) * size + 1);
    format!("{}–{} из {}", first, first + shown as i64 - 1, total)
}

impl Operators {
    pub fn new(_ctx: &Ctx) -> Self {
        Self {
            loaded: false,
            error: None,
            rows: Vec::new(),
            all: Vec::new(),
            stats: None,
            applications: Vec::new(),
            total: 0,
            page: 1,
            page_size: PAGE_DEFAULT,
            search: String::new(),
            search_token: 0,
            load_token: 0,
            status: Status::All,
            application: None,
            compact: false,
            selection: Selection::default(),
            sort: None,
            columns: base_columns(),
            editor: None,
            form: None,
            ask: None,
            menu: None,
            importing: false,
            import_errors: Vec::new(),
        }
    }

    pub fn total_records(&self) -> Option<usize> {
        self.stats.map(|stats| stats.total)
    }

    fn query(&self) -> Vec<(&'static str, String)> {
        let mut query = Vec::new();
        match self.page_size.0 {
            Some(size) => {
                query.push(("page", self.page.to_string()));
                query.push(("page_size", size.to_string()));
            }
            None => {
                query.push(("page", "1".to_owned()));
                query.push(("all", "true".to_owned()));
            }
        }
        let search = self.search.trim();
        if !search.is_empty() {
            query.push(("search", search.to_owned()));
        }
        if let Some(application) = &self.application {
            query.push(("application", application.clone()));
        }
        match self.status {
            Status::All => {}
            Status::Active => query.push(("is_active", "true".to_owned())),
            Status::Inactive => query.push(("is_active", "false".to_owned())),
            Status::P2p => query.push(("is_p2p", "true".to_owned())),
        }
        query
    }

    fn load(&mut self, ctx: &Ctx) -> Task<Msg> {
        self.load_token += 1;
        let (token, api, query) = (self.load_token, ctx.api.clone(), self.query());
        Task::perform(
            async move {
                let query: Vec<(&str, String)> = query.iter().map(|(k, v)| (*k, v.clone())).collect();
                api.operators(&query).await.map_err(text_error)
            },
            move |reply| Msg::Loaded(token, reply),
        )
    }

    fn load_side(&self, ctx: &Ctx) -> Task<Msg> {
        let (all, apps) = (ctx.api.clone(), ctx.api.clone());
        Task::batch([
            Task::perform(async move { all.all_operators().await.map_err(text_error) }, Msg::AllLoaded),
            Task::perform(async move { apps.applications().await.map_err(text_error) }, Msg::Apps),
        ])
    }

    pub fn load_all(&self, ctx: &Ctx) -> Task<Msg> {
        let api = ctx.api.clone();
        let query = self.query();
        let first = Task::perform(
            async move {
                let query: Vec<(&str, String)> = query.iter().map(|(k, v)| (*k, v.clone())).collect();
                api.operators(&query).await.map_err(text_error)
            },
            |reply| Msg::Loaded(0, reply),
        );
        Task::batch([first, self.load_side(ctx)])
    }

    fn reload(&mut self, ctx: &Ctx) -> Task<Msg> {
        Task::batch([self.load(ctx), self.load_side(ctx)])
    }

    fn selected_ids(&self) -> Vec<i64> {
        self.selection.rows().into_iter().filter_map(|row| self.rows.get(row as usize)).map(|item| item.id).collect()
    }

    fn find(&self, id: i64) -> Option<&OperatorRef> {
        self.rows.iter().find(|item| item.id == id)
    }

    fn ask_delete(&mut self, ids: Vec<i64>) {
        let names: Vec<&str> = ids.iter().filter_map(|id| self.find(*id)).map(|item| item.operator_name.as_str()).collect();
        let many = ids.len() > 1;
        let title = if many { format!("Удалить {} {}?", count(ids.len()), plural(ids.len(), "запись", "записи", "записей")) } else { "Удалить запись?".to_owned() };
        let shown: Vec<&str> = names.iter().take(5).copied().collect();
        let rest = names.len().saturating_sub(shown.len());
        let mut body = format!("Будет удалено: {}", shown.join(", "));
        if rest > 0 {
            body.push_str(&format!(" и ещё {rest}"));
        }
        body.push('.');
        self.ask = Some((ids, title, body, if many { "Удалить все" } else { "Удалить" }));
    }

    fn begin_edit(&mut self, row: usize, col: usize, rect: Rectangle, typed: Option<String>) -> Task<Msg> {
        let Some(item) = self.rows.get(row) else { return Task::none() };
        if col != COL_OPERATOR && col != COL_APPLICATION {
            return Task::none();
        }
        let original = if col == COL_OPERATOR { item.operator_name.clone() } else { item.application_name.clone() };
        let value = typed.unwrap_or_else(|| original.clone());
        self.editor = Some(Editor { id: item.id, col, value, original, rect });
        let id = Id::new(EDITOR_ID);
        Task::batch([operation::focus(id.clone()), operation::move_cursor_to_end(id)])
    }

    fn commit_edit(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let Some(editor) = self.editor.take() else { return Task::none() };
        let value = editor.value.trim().to_owned();
        if value == editor.original {
            return Task::none();
        }
        if value.is_empty() {
            ctx.toast(Tone::Warning, "Оператор и приложение обязательны");
            return Task::none();
        }
        let field = if editor.col == COL_OPERATOR { "operator_name" } else { "application_name" };
        self.patch(editor.id, json!({ field: value }), ctx)
    }

    fn patch(&self, id: i64, body: serde_json::Value, ctx: &Ctx) -> Task<Msg> {
        let api = ctx.api.clone();
        Task::perform(async move { api.update_operator(id, &body).await.map_err(text_error) }, Msg::Saved)
    }

    fn on_grid(&mut self, event: GridEvent, ctx: &mut Ctx) -> Task<Msg> {
        match event {
            GridEvent::Select(selection) => self.selection = selection,
            GridEvent::Click { row, col, .. } if col == COL_P2P || col == COL_ACTIVE => {
                if let Some(item) = self.rows.get_mut(row) {
                    let body = if col == COL_P2P {
                        item.is_p2p = !item.is_p2p;
                        json!({ "is_p2p": item.is_p2p })
                    } else {
                        item.is_active = !item.is_active;
                        json!({ "is_active": item.is_active })
                    };
                    let id = item.id;
                    return self.patch(id, body, ctx);
                }
            }
            GridEvent::Activate { row, col, rect } => return self.begin_edit(row, col, rect, None),
            GridEvent::TypeChar { row, col, rect, text } => return self.begin_edit(row, col, rect, Some(text)),
            GridEvent::Context { row, at, .. } => {
                if let Some(item) = self.rows.get(row) {
                    self.menu = Some((item.id, at));
                }
            }
            GridEvent::Sort(col) => {
                self.sort = match self.sort {
                    Some((current, false)) if current == col => Some((col, true)),
                    Some((current, true)) if current == col => None,
                    _ => Some((col, false)),
                };
                for (index, column) in self.columns.iter_mut().enumerate() {
                    column.sorted = self.sort.filter(|(sorted, _)| *sorted == index).map(|(_, descending)| descending);
                }
                self.selection = Selection::default();
                sort_rows(&mut self.rows, self.sort);
            }
            GridEvent::Resize { col, width } => {
                if let Some(column) = self.columns.get_mut(col) {
                    column.width = width;
                }
            }
            GridEvent::Dismiss => return self.commit_edit(ctx),
            GridEvent::Key(GridKey::Delete) => {
                let ids = self.selected_ids();
                if !ids.is_empty() {
                    self.ask_delete(ids);
                }
            }
            GridEvent::Key(GridKey::Escape) => self.selection = Selection::default(),
            _ => {}
        }
        Task::none()
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Escape => {
                if self.editor.take().is_some() {
                } else if self.menu.take().is_some() {
                } else if self.ask.take().is_some() {
                } else if self.form.take().is_some() {
                } else if !self.search.is_empty() {
                    return self.update(Msg::Search(String::new()), ctx);
                } else {
                    self.selection = Selection::default();
                }
            }
            Msg::Search(value) => {
                self.search = value;
                self.search_token += 1;
                let token = self.search_token;
                return Task::perform(tokio::time::sleep(DEBOUNCE), move |_| Msg::SearchSettled(token));
            }
            Msg::SearchSettled(token) => {
                if token == self.search_token {
                    self.page = 1;
                    return self.load(ctx);
                }
            }
            Msg::Status(status) => {
                self.status = status;
                self.page = 1;
                return self.load(ctx);
            }
            Msg::Application(name) => {
                self.application = (name != ALL_APPS).then_some(name);
                self.page = 1;
                return self.load(ctx);
            }
            Msg::Compact(compact) => self.compact = compact,
            Msg::PageSize(size) => {
                self.page_size = size;
                self.page = 1;
                return self.load(ctx);
            }
            Msg::Page(page) => {
                self.page = page;
                return self.load(ctx);
            }
            Msg::Loaded(token, reply) => {
                if token != 0 && token != self.load_token {
                    return Task::none();
                }
                self.loaded = true;
                match reply {
                    Ok(page) => {
                        self.error = None;
                        self.total = page.total;
                        self.rows = page.items;
                        sort_rows(&mut self.rows, self.sort);
                        self.selection = Selection::default();
                    }
                    Err(reason) => self.error = Some(reason),
                }
            }
            Msg::AllLoaded(Ok(items)) => {
                self.stats = Some(Stats::of(&items));
                self.all = items;
            }
            Msg::AllLoaded(Err(reason)) => tracing::warn!("operator stats not loaded: {reason}"),
            Msg::Apps(Ok(names)) => self.applications = names,
            Msg::Apps(Err(reason)) => tracing::warn!("applications not loaded: {reason}"),
            Msg::Grid(event) => return self.on_grid(event, ctx),
            Msg::EditText(value) => {
                if let Some(editor) = &mut self.editor {
                    editor.value = value;
                }
            }
            Msg::EditCommit => return self.commit_edit(ctx),
            Msg::Saved(Ok(item)) => {
                if let Some(row) = self.rows.iter_mut().find(|row| row.id == item.id) {
                    *row = item;
                }
                return self.load_side(ctx);
            }
            Msg::Saved(Err(reason)) => {
                ctx.toast(Tone::Danger, format!("Не удалось сохранить: {reason}"));
                return self.reload(ctx);
            }
            Msg::ClearSelection => self.selection = Selection::default(),
            Msg::Bulk(active) => {
                let (ids, api) = (self.selected_ids(), ctx.api.clone());
                return Task::perform(
                    async move {
                        for id in &ids {
                            api.update_operator(*id, &json!({ "is_active": active })).await.map_err(text_error)?;
                        }
                        Ok(ids.len())
                    },
                    move |reply| Msg::BulkDone(active, reply),
                );
            }
            Msg::BulkDone(active, reply) => {
                match reply {
                    Ok(_) => ctx.toast(Tone::Success, if active { "Записи активированы" } else { "Записи деактивированы" }),
                    Err(reason) => ctx.toast(Tone::Danger, format!("Не удалось сохранить: {reason}")),
                }
                return self.reload(ctx);
            }
            Msg::AskDelete(ids) => {
                self.menu = None;
                self.ask_delete(ids);
            }
            Msg::CancelDelete => self.ask = None,
            Msg::ConfirmDelete => {
                let Some((ids, ..)) = self.ask.take() else { return Task::none() };
                let api = ctx.api.clone();
                return Task::perform(
                    async move {
                        for id in &ids {
                            api.delete_operator(*id).await.map_err(text_error)?;
                        }
                        Ok(ids.len())
                    },
                    Msg::Deleted,
                );
            }
            Msg::Deleted(reply) => {
                match reply {
                    Ok(1) => ctx.toast(Tone::Success, "Запись удалена"),
                    Ok(_) => ctx.toast(Tone::Success, "Удалено"),
                    Err(reason) => ctx.toast(Tone::Danger, format!("Не удалось удалить: {reason}")),
                }
                return self.reload(ctx);
            }
            Msg::OpenAdd => {
                self.form = Some(Form::empty());
                return operation::focus(Id::new(FORM_OPERATOR_ID));
            }
            Msg::OpenEdit(id) => {
                self.menu = None;
                self.form = self.find(id).map(Form::of);
                return operation::focus(Id::new(FORM_OPERATOR_ID));
            }
            Msg::CloseForm => self.form = None,
            Msg::FormOperator(value) => self.with_form(|form| form.operator = value),
            Msg::FormApplication(value) | Msg::FormApp(value) => self.with_form(|form| form.application = value),
            Msg::FormP2p(value) => self.with_form(|form| form.is_p2p = value),
            Msg::FormActive(value) => self.with_form(|form| form.is_active = value),
            Msg::FormSubmit => {
                let Some(form) = &mut self.form else { return Task::none() };
                let Some(body) = form.body() else {
                    form.error = Some("Оператор и приложение обязательны".to_owned());
                    return Task::none();
                };
                form.error = None;
                form.busy = true;
                let (id, api) = (form.id, ctx.api.clone());
                return Task::perform(
                    async move {
                        match id {
                            Some(id) => api.update_operator(id, &body).await,
                            None => api.create_operator(&body).await,
                        }
                        .map_err(text_error)
                    },
                    Msg::FormDone,
                );
            }
            Msg::FormDone(reply) => {
                let adding = self.form.as_ref().is_some_and(|form| form.id.is_none());
                match reply {
                    Ok(_) => {
                        self.form = None;
                        ctx.toast(Tone::Success, if adding { "Запись добавлена" } else { "Сохранено" });
                        return self.reload(ctx);
                    }
                    Err(reason) => {
                        let lead = if adding { "Не удалось добавить" } else { "Не удалось сохранить" };
                        if let Some(form) = &mut self.form {
                            form.busy = false;
                            form.error = Some(format!("{lead}: {reason}"));
                        }
                    }
                }
            }
            Msg::CloseMenu => self.menu = None,
            Msg::Export => {
                let api = ctx.api.clone();
                let fallback = format!("operators-{}.xlsx", Local::now().format("%Y-%m-%d"));
                return Task::perform(
                    async move {
                        let (name, bytes) = api.export_operators().await.map_err(text_error)?;
                        files::save_as(name.unwrap_or(fallback), bytes).await
                    },
                    Msg::Exported,
                );
            }
            Msg::Exported(Ok(Some(_))) => ctx.toast(Tone::Success, "Файл сохранён"),
            Msg::Exported(Ok(None)) => {}
            Msg::Exported(Err(reason)) => ctx.toast(Tone::Danger, format!("Экспорт не удался: {reason}")),
            Msg::Import => {
                self.importing = true;
                self.import_errors.clear();
                let api = ctx.api.clone();
                return Task::perform(import_book(api), Msg::Imported);
            }
            Msg::Imported(reply) => {
                self.importing = false;
                match reply {
                    Ok(None) => {}
                    Ok(Some(reply)) => {
                        ctx.toast(Tone::Success, format!("Импортировано: {} · пропущено: {}", reply.imported, reply.skipped));
                        if !reply.errors.is_empty() {
                            ctx.toast(Tone::Warning, format!("Импорт с ошибками: {}", reply.errors.len()));
                        }
                        self.import_errors = reply.errors;
                        return self.reload(ctx);
                    }
                    Err(reason) => ctx.toast(Tone::Danger, format!("Ошибка импорта: {reason}")),
                }
            }
            Msg::DismissErrors => self.import_errors.clear(),
        }
        Task::none()
    }

    fn with_form(&mut self, change: impl FnOnce(&mut Form)) {
        if let Some(form) = &mut self.form {
            change(form);
            form.error = None;
        }
    }

    fn header(&self) -> El<'_, Msg> {
        let import_caption = if self.importing { "Импорт…" } else { "Импорт" };
        let actions = row![
            ui::btn_icon(Icon::Upload, import_caption, Kind::Secondary, (!self.importing).then_some(Msg::Import)),
            ui::btn_icon(Icon::Download, "Экспорт", Kind::Secondary, Some(Msg::Export)),
            ui::btn_icon(Icon::Plus, "Добавить", Kind::Primary, Some(Msg::OpenAdd)),
        ]
        .spacing(space::SM);
        ui::page_header("Справочник операторов", Some("Единый источник маппинга: имя продавца на чеке → нормализованное приложение. Используется в каскаде распознавания."), actions)
    }

    fn stats_row(&self) -> El<'_, Msg> {
        let stats = self.stats.unwrap_or_default();
        row![
            ui::stat("Всего", count(stats.total), "записей в справочнике"),
            ui::stat("Активных", count(stats.active), format!("{}% от общего", stats.active_share())),
            ui::stat("P2P", count(stats.p2p), "помечено как P2P"),
            ui::stat("Приложений", count(stats.applications), "уникальных целей"),
        ]
        .spacing(space::SM)
        .into()
    }

    fn toolbar(&self) -> El<'_, Msg> {
        let stats = self.stats.unwrap_or_default();
        let chip = |status: Status, caption: &str, amount: usize| ui::chip(format!("{caption} {}", count(amount)), self.status == status, Msg::Status(status));
        let mut options = vec![ALL_APPS.to_owned()];
        options.extend(self.applications.iter().cloned());
        let picked = self.application.clone().unwrap_or_else(|| ALL_APPS.to_owned());
        row![
            ui::input("Поиск по оператору или приложению", &self.search).id(Id::new(SEARCH_ID)).on_input(Msg::Search).width(Fill),
            chip(Status::All, "Все", stats.total),
            chip(Status::Active, "Активные", stats.active),
            chip(Status::Inactive, "Неактивные", stats.inactive),
            chip(Status::P2p, "P2P", stats.p2p),
            ui::select(options, Some(picked), Msg::Application),
            ui::segments(&[(false, "Обычно"), (true, "Плотно")], self.compact, Msg::Compact),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center)
        .into()
    }

    fn footer(&self) -> El<'_, Msg> {
        let mut line = row![ui::muted(range_label(self.page, self.page_size, self.rows.len(), self.total)), ui::fill()].spacing(space::SM).align_y(Alignment::Center);
        let ids = self.selected_ids();
        if !ids.is_empty() {
            let picked = ids.len();
            line = line
                .push(ui::label(format!("{} выбрано", count(picked))))
                .push(ui::btn("Активировать", Kind::Secondary, Some(Msg::Bulk(true))))
                .push(ui::btn("Деактивировать", Kind::Secondary, Some(Msg::Bulk(false))))
                .push(ui::btn(format!("Удалить · {}", count(picked)), Kind::Danger, Some(Msg::AskDelete(ids))))
                .push(ui::btn("Снять выбор", Kind::Ghost, Some(Msg::ClearSelection)));
        }
        line = line.push(ui::select(SIZES.to_vec(), Some(self.page_size), Msg::PageSize)).push(ui::pager(self.page, page_count(self.total, self.page_size), Msg::Page));
        container(line).height(FOOTER_HEIGHT + 4.0).width(Fill).into()
    }

    pub fn view(&self, ctx: &Ctx) -> El<'_, Msg> {
        let body: El<'_, Msg> = if !self.loaded {
            ui::loading("Загружаем справочник…")
        } else if let Some(reason) = &self.error {
            container(ui::notice(Tone::Danger, reason.as_str())).padding(space::MD).into()
        } else if self.rows.is_empty() {
            ui::empty_state(Icon::Book, "Записей нет", "Очистите фильтры или добавьте первую запись через кнопку «+ Добавить».")
        } else {
            let (height, font) = if self.compact { (26.0, 12.0) } else { (32.0, 13.0) };
            grid::grid(Rows { rows: &self.rows, dark: ctx.dark }, &self.columns, &self.selection, Msg::Grid)
                .row_height(height)
                .font_size(font)
                .locked(self.editor.is_some())
                .into()
        };
        let mut page = column![self.header(), self.stats_row(), self.toolbar()].spacing(space::MD);
        if !self.import_errors.is_empty() {
            let shown: Vec<&str> = self.import_errors.iter().take(5).map(String::as_str).collect();
            let line = row![ui::notice(Tone::Warning, shown.join("; ")), ui::icon_btn(Icon::Close, "Скрыть", Some(Msg::DismissErrors))].spacing(space::SM).align_y(Alignment::Center);
            page = page.push(line);
        }
        page.push(container(body).width(Fill).height(Fill).style(theme::card)).push(self.footer()).height(Fill).into()
    }

    fn form_view<'a>(&'a self, form: &'a Form) -> El<'a, Msg> {
        let typed = form.application.to_lowercase();
        let mut suggestions = column![].spacing(space::XXS);
        let matches: Vec<&String> = self.applications.iter().filter(|name| !typed.is_empty() && name.to_lowercase().contains(&typed) && **name != form.application).take(6).collect();
        for name in matches {
            suggestions = suggestions.push(ui::menu_item(Icon::Tag, name.as_str(), false, Msg::FormApp(name.clone())));
        }
        let mut body = column![
            ui::field("Оператор · продавец", ui::input("Например: Магнит", &form.operator).id(Id::new(FORM_OPERATOR_ID)).on_input(Msg::FormOperator).on_submit(Msg::FormSubmit)),
            ui::field("Приложение", ui::input("uzcard / humo / ofd / ...", &form.application).on_input(Msg::FormApplication).on_submit(Msg::FormSubmit)),
            suggestions,
            row![ui::label("P2P"), ui::fill(), ui::switch(form.is_p2p, Msg::FormP2p)].align_y(Alignment::Center),
            row![ui::label("Активен"), ui::fill(), ui::switch(form.is_active, Msg::FormActive)].align_y(Alignment::Center),
        ]
        .spacing(space::MD);
        if let Some(reason) = &form.error {
            body = body.push(ui::error_line(reason.as_str()));
        }
        let submit = if form.busy {
            "Сохраняем…"
        } else if form.id.is_some() {
            "Сохранить"
        } else {
            "Добавить"
        };
        let footer = row![ui::fill(), ui::btn("Отмена", Kind::Secondary, Some(Msg::CloseForm)), ui::btn(submit, Kind::Primary, (!form.busy).then_some(Msg::FormSubmit))].spacing(space::SM);
        let (name, hint) = if form.id.is_some() { ("Редактирование", form.operator.as_str()) } else { ("Новая запись", "оператор → приложение") };
        let panel = ui::sheet_panel("Справочник · operator → application", name, hint, Msg::CloseForm, body, Some(footer.into()));
        ui::sheet(Space::new(), panel, FORM_WIDTH, Some(Msg::CloseForm))
    }

    pub fn overlay(&self, ctx: &Ctx) -> Vec<El<'_, Msg>> {
        let mut layers = Vec::new();
        if let Some(form) = &self.form {
            layers.push(self.form_view(form));
        }
        if let Some((id, at)) = self.menu {
            let body = column![
                ui::menu_item(Icon::Edit, "Изменить", false, Msg::OpenEdit(id)),
                ui::menu_item(Icon::Trash, "Удалить", true, Msg::AskDelete(vec![id])),
            ]
            .spacing(space::XXS);
            layers.push(ui::floating(ui::menu_panel(body, MENU.width), at, MENU, ctx.window, Msg::CloseMenu));
        }
        if let Some(editor) = &self.editor {
            layers.push(ui::cell_editor(Id::new(EDITOR_ID), &editor.value, editor.rect, ctx.window, Msg::EditText, Msg::EditCommit, Msg::EditCommit));
        }
        if let Some((_, title, body, action)) = &self.ask {
            layers.push(ui::confirm(Space::new(), title, body, action, Msg::ConfirmDelete, Msg::CancelDelete));
        }
        layers
    }
}

const ALL_APPS: &str = "Все приложения";
const FORM_OPERATOR_ID: &str = "operators-form-name";

async fn import_book(api: Api) -> Result<Option<ImportReply>, String> {
    let Some(book) = files::pick_file(&["xlsx", "xls"]).await? else { return Ok(None) };
    api.import_operators(&book.name, book.bytes).await.map(Some).map_err(text_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64, name: &str, app: &str, p2p: bool, active: bool) -> OperatorRef {
        OperatorRef { id, operator_name: name.to_owned(), application_name: app.to_owned(), is_p2p: p2p, is_active: active }
    }

    #[test]
    fn stats_count_each_group_once() {
        let items = [item(1, "a", "uzcard", true, true), item(2, "b", "uzcard", false, true), item(3, "c", "humo", false, false), item(4, "d", "", false, true)];
        let stats = Stats::of(&items);
        assert_eq!((stats.total, stats.active, stats.inactive, stats.p2p, stats.applications), (4, 3, 1, 1, 2));
        assert_eq!(stats.active_share(), 75);
        assert_eq!(Stats::of(&[]).active_share(), 0);
    }

    #[test]
    fn pages_round_up_and_all_is_one_page() {
        assert_eq!(page_count(0, PageSize(Some(100))), 1);
        assert_eq!(page_count(101, PageSize(Some(100))), 2);
        assert_eq!(page_count(5000, PageSize(None)), 1);
    }

    #[test]
    fn range_counts_from_the_page_start() {
        assert_eq!(range_label(2, PageSize(Some(20)), 20, 57), "21–40 из 57");
        assert_eq!(range_label(1, PageSize(None), 57, 57), "1–57 из 57");
        assert_eq!(range_label(1, PageSize(Some(20)), 0, 0), "0 из 0");
    }

    #[test]
    fn sort_follows_column_and_direction() {
        let mut rows = vec![item(1, "б", "z", false, true), item(2, "А", "y", true, false)];
        sort_rows(&mut rows, Some((COL_OPERATOR, false)));
        assert_eq!(rows[0].id, 2);
        sort_rows(&mut rows, Some((COL_OPERATOR, true)));
        assert_eq!(rows[0].id, 1);
        sort_rows(&mut rows, Some((COL_P2P, true)));
        assert_eq!(rows[0].id, 2);
    }

    #[test]
    fn form_refuses_blank_fields_and_trims_the_rest() {
        let mut form = Form::empty();
        form.operator = "  ".to_owned();
        form.application = "uzcard".to_owned();
        assert!(form.body().is_none());
        form.operator = " Магнит ".to_owned();
        let body = form.body().unwrap();
        assert_eq!(body["operator_name"], "Магнит");
        assert_eq!(body["is_active"], true);
    }
}
