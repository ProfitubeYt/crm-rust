//! The main table: local data, its view, and the actions on it.

mod access;
mod add;
mod detail;
pub(crate) mod edit;
mod excel;
mod facets;
mod filters;
mod menus;
pub mod table;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::event::{self, Event};
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Id, Space, Stack, column, container, opaque, operation, row};
use iced::{Alignment, Fill, Subscription, Task, window};
use tbs_core::api::Api;
use tbs_core::columns::{Column, Labels};
use tbs_core::model::Transaction;
use tbs_core::query::{Dataset, Scope, Sort};
use tbs_core::store::Store;
use tbs_core::sync::{Mode, SyncResult, sync_transactions};

use crate::ctx::{Ctx, SyncState, text_error};
use crate::grid::{self, GridColumn, GridEvent, GridKey, Selection};
use crate::theme::{self, Tone, space};
use crate::ui::{self, El, Icon, Kind};
use access::Access;
use add::AddForm;
use detail::Detail;
use edit::{Choices, DeleteAsk, Editor, History};
use excel::ExcelPanel;
use filters::{FilterForm, Kept};
use menus::{Popup, Preset, PresetAsk};
use table::{PageSize, TableView, TxGrid};

pub use access::Msg as AccessMsg;

const STATE: &str = "transactions";
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(200);
const SEARCH_ID: &str = "tx-search";
const FOOTER_HEIGHT: f32 = 36.0;
/// The narrowest page in which the tools above the table fit one line and
/// still leave a search box worth typing in.
const ROOMY_PAGE: f32 = 1090.0;
/// How long the manifest alone is believed. It cannot see everything (see
/// `tbs_core::sync`), so this often the rows themselves are compared.
const VERIFY_EVERY: Duration = Duration::from_secs(300);

/// A value too heavy to clone that must still ride in a message once.
pub struct Parcel<T>(Arc<Mutex<Option<T>>>);

impl<T> Clone for Parcel<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Parcel<T> {
    pub fn new(value: T) -> Self {
        Self(Arc::new(Mutex::new(Some(value))))
    }

    pub fn take(&self) -> Option<T> {
        self.0.lock().unwrap().take()
    }
}

impl<T> std::fmt::Debug for Parcel<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Parcel")
    }
}

#[derive(Clone, Debug)]
pub enum Msg {
    Noop,
    Enter,
    Cached(Parcel<Dataset>),
    Sync(bool),
    SyncDone(Mode, Result<Option<Parcel<Dataset>>, String>),
    Options(Result<(Vec<String>, Vec<String>), String>),
    Choices(Result<Choices, String>),
    Search(String),
    SearchSettled(u64),
    Grid(GridEvent),
    Year(i32),
    Escape,
    FocusSearch,
    Newest,
    Page(i64),
    PageSize(PageSize),
    /// One of the type chips above the table; `None` shows every type.
    Quick(Option<&'static str>),
    HighlightOff(i64),
    ClearSelection,
    OpenAdd,
    OpenFilters,
    OpenExcel,
    Edit(edit::Msg),
    Add(add::Msg),
    Filters(filters::Msg),
    Excel(excel::Msg),
    Access(access::Msg),
    Menu(menus::Msg),
    Detail(detail::Msg),
}

#[derive(Default)]
pub enum Sheet {
    #[default]
    None,
    Add(AddForm),
    Filters(Box<FilterForm>),
    Excel(Box<ExcelPanel>),
    Detail(Detail),
}

#[cfg(test)]
#[derive(Debug, PartialEq)]
pub struct Peek {
    pub loaded: bool,
    pub total: usize,
    pub shown: usize,
    pub year: Option<i32>,
    pub page: usize,
    pub on_page: usize,
}

/// Rows each type chip would leave, the other filters kept. Counted once per
/// change of the filtered rows.
#[derive(Default)]
struct Summary {
    any_type: usize,
    debits: usize,
    credits: usize,
}

pub struct Transactions {
    data: Dataset,
    summary: Summary,
    /// Every row that passes the filters, in table order.
    matched: Vec<u32>,
    /// The page of `matched` on screen; grid rows index into this.
    rows: Vec<u32>,
    page: usize,
    offset: usize,
    table: TableView,
    columns: Vec<Column>,
    grid_columns: Vec<GridColumn>,
    selection: Selection,
    search: String,
    search_token: u64,
    year: Option<i32>,
    years: Vec<i32>,
    access: Access,
    admin: bool,
    offline: bool,
    want_sync: Option<bool>,
    /// When the rows on screen were last proven equal to the server's.
    verified_at: Option<Instant>,
    loaded: bool,
    reveal: Option<(u64, usize)>,
    reveal_token: u64,
    highlight: Option<i64>,
    choices: Choices,
    operator_options: Vec<String>,
    app_options: Vec<String>,
    editor: Option<Editor>,
    history: History,
    delete_ask: Option<DeleteAsk>,
    sheet: Sheet,
    /// What the filter sheet keeps between its openings: its folds and a form nobody applied.
    kept: Kept,
    popup: Popup,
    presets: Vec<Preset>,
    preset_name: String,
    preset_ask: Option<PresetAsk>,
    export_from: String,
    export_to: String,
}

async fn read_cache(store: Arc<Store>) -> Parcel<Dataset> {
    let rows = tokio::task::spawn_blocking(move || store.load_transactions().unwrap_or_default()).await.unwrap_or_default();
    build(rows).await
}

async fn build(rows: Vec<Transaction>) -> Parcel<Dataset> {
    let data = tokio::task::spawn_blocking(move || Dataset::build(rows)).await.unwrap_or_default();
    Parcel::new(data)
}

async fn run_sync(api: Api, store: Arc<Store>, mode: Mode) -> Result<Option<Parcel<Dataset>>, String> {
    match sync_transactions(&api, store, mode, |_, _| {}).await.map_err(text_error)? {
        SyncResult::Unchanged { .. } => Ok(None),
        SyncResult::Updated(rows) => Ok(Some(build(rows).await)),
    }
}

async fn read_options(api: Api) -> Result<(Vec<String>, Vec<String>), String> {
    let operators = api.all_operators().await.map_err(text_error)?;
    let apps = api.applications().await.map_err(text_error)?;
    Ok((operators.into_iter().map(|o| o.operator_name).collect(), apps))
}

async fn read_choices(api: Api) -> Result<Choices, String> {
    let persons = api.annotation_persons(false).await.map_err(text_error)?;
    let articles = api.annotation_articles(false).await.map_err(text_error)?;
    Ok(Choices {
        persons: persons.into_iter().map(|p| { let name = p.full_name.unwrap_or_else(|| p.code.clone()); (p.code, name) }).collect(),
        articles: articles.into_iter().map(|a| (a.code, a.label)).collect(),
    })
}

fn merge(into: &mut Vec<String>, items: impl IntoIterator<Item = String>) {
    into.extend(items.into_iter().filter(|s| !s.trim().is_empty()));
    into.sort();
    into.dedup();
}

/// A press on these opens the operation rather than an editor.
fn opens_detail(column: Column) -> bool {
    matches!(column, Column::RowNumber | Column::OperationNumber | Column::TransactionDate | Column::Day)
}

fn hotkey(event: Event, _status: event::Status, _window: window::Id) -> Option<Msg> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, physical_key, .. }) = event else { return None };
    if key == Key::Named(Named::Escape) {
        return Some(Msg::Escape);
    }
    (modifiers.command() && key.to_latin(physical_key) == Some('k')).then_some(Msg::FocusSearch)
}

impl Transactions {
    pub fn new(ctx: &Ctx) -> (Self, Task<Msg>) {
        let path = ctx.state_path(STATE);
        let presets = Self::load_presets(ctx);
        let starting = presets.iter().find(|p| p.default).filter(|_| !path.exists()).map(|p| p.view.clone());
        let mut table: TableView = starting.unwrap_or_else(|| tbs_core::config::load_json(&path));
        table.repair();
        let search = table.filters.search.clone();
        let mut screen = Self {
            data: Dataset::default(),
            summary: Summary::default(),
            matched: Vec::new(),
            rows: Vec::new(),
            page: 0,
            offset: 0,
            columns: Vec::new(),
            grid_columns: Vec::new(),
            table,
            selection: Selection::default(),
            search,
            search_token: 0,
            year: Self::saved_year(ctx),
            years: Vec::new(),
            access: Access::default(),
            admin: ctx.is_admin(),
            offline: !ctx.online,
            want_sync: None,
            verified_at: None,
            loaded: false,
            reveal: None,
            reveal_token: 0,
            highlight: None,
            choices: Choices::default(),
            operator_options: Vec::new(),
            app_options: Vec::new(),
            editor: None,
            history: History::default(),
            delete_ask: None,
            sheet: Sheet::None,
            kept: Kept::default(),
            popup: Popup::None,
            presets,
            preset_name: String::new(),
            preset_ask: None,
            export_from: String::new(),
            export_to: String::new(),
        };
        screen.refresh_columns();
        screen.settle_year(ctx);
        let mut tasks = vec![access::fetch_all(ctx), Self::fetch_dictionaries(ctx)];
        if let Some(store) = ctx.store.clone() {
            tasks.push(Task::perform(read_cache(store), Msg::Cached));
        }
        (screen, Task::batch(tasks))
    }

    fn fetch_dictionaries(ctx: &Ctx) -> Task<Msg> {
        let (first, second) = (ctx.api.clone(), ctx.api.clone());
        Task::batch([Task::perform(read_options(first), Msg::Options), Task::perform(read_choices(second), Msg::Choices)])
    }

    /// Keys belong to the table only while it is on screen; syncing never stops.
    pub fn subscription(&self, on_screen: bool) -> Subscription<Msg> {
        let keys = if on_screen { event::listen_with(hotkey) } else { Subscription::none() };
        Subscription::batch([iced::time::every(Duration::from_secs(60)).map(|_| Msg::Sync(false)), keys, self.access.timer()])
    }

    #[cfg(feature = "devtools")]
    pub fn is_ready(&self) -> bool {
        self.loaded
    }

    /// The message a test hook sends to show one panel.
    #[cfg(feature = "devtools")]
    pub fn probe(name: &str) -> Option<Msg> {
        let at = iced::Point::new(620.0, 300.0);
        let rect = iced::Rectangle::new(iced::Point::new(700.0, 176.0), iced::Size::new(240.0, 28.0));
        let open = |popup| Msg::Menu(menus::Msg::Open(popup));
        Some(match name {
            "filters" => Msg::OpenFilters,
            "add" => Msg::OpenAdd,
            "excel" => Msg::OpenExcel,
            "view" => open(Popup::View),
            "export" => open(Popup::Export),
            "help" => open(Popup::Help),
            "cell" => open(Popup::Cell { row: 1, col: 6, at }),
            "header" => open(Popup::Header { col: 6, at }),
            "detail" => Msg::Grid(GridEvent::Click { row: 1, col: 0, at: iced::Point::ORIGIN }),
            "edit" => Msg::Grid(GridEvent::Activate { row: 1, col: 6, rect }),
            "delete" => Msg::Edit(edit::Msg::AskDelete(vec![1])),
            _ => return None,
        })
    }

    /// Opening a locked folder again takes a fresh code: leaving closes it.
    pub fn leave(&mut self, ctx: &Ctx) {
        if ctx.is_admin() {
            return;
        }
        ctx.api.set_scope_token(None);
        self.access.close_door();
        self.refresh_rows();
    }

    fn refresh_columns(&mut self) {
        self.columns = self.table.visible();
        self.grid_columns = self.table.grid_columns(&self.columns);
    }

    fn scope(&self) -> Scope {
        self.access.data_scope(self.year)
    }

    /// The dictionaries as the table's cells need them: a name for each code.
    fn labels(&self) -> Labels {
        Labels::new(self.choices.persons.iter().cloned(), self.choices.articles.iter().cloned())
    }

    fn locked_view(&self) -> bool {
        self.access.locked_view(self.admin, self.year)
    }

    fn editing_blocked(&self) -> bool {
        self.offline || self.locked_view()
    }

    /// Whether a cell is open for typing, where Tab means "done with this one".
    pub fn editing(&self) -> bool {
        self.editor.is_some()
    }

    /// Cuts the page on screen out of the filtered rows.
    fn cut_page(&mut self) {
        let (page, range) = self.table.page_size.window(self.page, self.matched.len());
        self.page = page;
        self.offset = range.start;
        self.rows = self.matched[range].to_vec();
        self.selection.clamp(self.rows.len(), self.columns.len());
    }

    /// The same page after the data moved under it: a sync, an edit, a delete.
    fn refresh_keeping_selection(&mut self) {
        self.table.filters.search = self.search.clone();
        self.matched = if self.locked_view() { Vec::new() } else { self.data.view(&self.table.filters, &self.scope(), &self.table.sort) };
        self.cut_page();
        self.summarize();
        self.filters_refreshed();
    }

    fn summarize(&mut self) {
        let mut sum = Summary::default();
        if !self.locked_view() {
            let mut loose = self.table.filters.clone();
            loose.transaction_types.clear();
            let rows = if loose == self.table.filters { self.matched.clone() } else { self.data.view(&loose, &self.scope(), &self.table.sort) };
            sum.any_type = rows.len();
            for index in rows {
                match self.data.rows[index as usize].tx.transaction_type.as_deref() {
                    Some("DEBIT") => sum.debits += 1,
                    Some("CREDIT") => sum.credits += 1,
                    _ => {}
                }
            }
        }
        self.summary = sum;
    }

    /// A new question to the table, so the answer starts from its first page.
    fn refresh_rows(&mut self) {
        self.page = 0;
        self.refresh_keeping_selection();
        self.selection = Selection::default();
    }

    fn turn_page(&mut self, page: usize) {
        self.page = page;
        self.cut_page();
        self.selection = Selection::default();
        self.editor = None;
        self.popup = Popup::None;
        self.reveal_token += 1;
        self.reveal = Some((self.reveal_token, 0));
    }

    /// Where a row on screen stands in the whole filtered table.
    pub(super) fn position(&self, row: usize) -> usize {
        self.offset + row
    }

    pub(super) fn matched_len(&self) -> usize {
        self.matched.len()
    }

    fn row_ids(&self) -> Vec<i64> {
        self.rows.iter().map(|&i| self.data.rows[i as usize].tx.id).collect()
    }

    fn accept(&mut self, data: Dataset, ctx: &Ctx) {
        let before = self.row_ids();
        self.data = data;
        let labels = self.labels();
        self.data.set_labels(labels);
        self.loaded = true;
        merge(&mut self.operator_options, self.data.operators.clone());
        merge(&mut self.app_options, self.data.apps.clone());
        self.settle_year(ctx);
        self.refresh_keeping_selection();
        if self.row_ids() != before {
            self.selection = Selection::default();
        }
    }

    fn add_options(&mut self, operators: Vec<String>, apps: Vec<String>) {
        merge(&mut self.operator_options, operators);
        merge(&mut self.app_options, apps);
    }

    fn take_new_row(&mut self, tx: Transaction, ctx: &Ctx) -> Task<Msg> {
        self.take_rows(vec![tx], ctx)
    }

    fn reveal_id(&mut self, id: i64) {
        let Some(position) = self.matched.iter().position(|&i| self.data.rows[i as usize].tx.id == id) else { return };
        let page = self.table.page_size.page_of(position);
        if page != self.page {
            self.page = page;
            self.cut_page();
            self.selection = Selection::default();
        }
        self.reveal_token += 1;
        self.reveal = Some((self.reveal_token, position - self.offset));
    }

    fn save(&self, ctx: &Ctx) {
        if let Err(error) = tbs_core::config::save_json(&ctx.state_path(STATE), &self.table) {
            tracing::warn!("table state not saved: {error}");
        }
    }

    fn escape(&mut self) -> Task<Msg> {
        if self.editor.is_some() {
            self.editor = None;
        } else if self.popup != Popup::None {
            self.popup = Popup::None;
        } else if self.preset_ask.is_some() {
            self.preset_ask = None;
        } else if self.delete_ask.is_some() {
            self.delete_ask = None;
        } else if !matches!(self.sheet, Sheet::None) {
            // Esc on the filter sheet is not «Отменить изменения»: what was typed waits for the next opening.
            self.close_filters();
            self.sheet = Sheet::None;
        } else {
            self.selection = Selection::default();
        }
        Task::none()
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        self.offline = !ctx.online;
        self.admin = ctx.is_admin();
        match message {
            Msg::Noop => {}
            Msg::Enter => {
                self.refresh_keeping_selection();
                // Another screen, or another person, may have changed the rows or
                // the dictionaries while the table was out of sight, and the
                // manifest is too slow a witness for that: the rows are compared.
                return Task::batch([access::fetch_all(ctx), Self::fetch_dictionaries(ctx), Task::done(Msg::Sync(true))]);
            }
            Msg::Cached(parcel) => {
                if let Some(data) = parcel.take()
                    && !data.is_empty()
                {
                    self.accept(data, ctx);
                }
                // The cache is what the table said last time. It is shown at once
                // and proven against the server straight after.
                return Task::done(Msg::Sync(true));
            }
            Msg::Sync(force) => {
                let Some(store) = ctx.store.clone() else { return Task::none() };
                if self.locked_view() {
                    self.want_sync = Some(self.want_sync.unwrap_or(false) || force);
                    return Task::none();
                }
                if ctx.sync == SyncState::Syncing {
                    // A demanded refresh must not be lost to the one in flight,
                    // which may have read the server before the change it is for.
                    if force {
                        self.want_sync = Some(true);
                    }
                    return Task::none();
                }
                let stale = self.verified_at.is_none_or(|at| at.elapsed() >= VERIFY_EVERY);
                let mode = if force || stale { Mode::Verify } else { Mode::Quick };
                ctx.sync = SyncState::Syncing;
                return Task::perform(run_sync(ctx.api.clone(), store, mode), move |result| Msg::SyncDone(mode, result));
            }
            Msg::SyncDone(mode, result) => {
                match result {
                    Ok(parcel) => {
                        ctx.sync = SyncState::Idle;
                        if mode == Mode::Verify {
                            self.verified_at = Some(Instant::now());
                        }
                        if let Some(data) = parcel.and_then(|p| p.take()) {
                            self.accept(data, ctx);
                        }
                    }
                    Err(reason) => ctx.sync = SyncState::Error(reason),
                }
                self.loaded = true;
                if !self.locked_view()
                    && let Some(force) = self.want_sync.take()
                {
                    return Task::done(Msg::Sync(force));
                }
            }
            Msg::Options(Ok((operators, apps))) => self.add_options(operators, apps),
            Msg::Options(Err(reason)) => tracing::warn!("operator list: {reason}"),
            Msg::Choices(Ok(choices)) => {
                self.choices = choices;
                let labels = self.labels();
                self.data.set_labels(labels);
            }
            Msg::Choices(Err(reason)) => tracing::warn!("annotation dictionaries: {reason}"),
            Msg::Search(value) => {
                self.search = value;
                self.search_token += 1;
                let token = self.search_token;
                return Task::perform(tokio::time::sleep(SEARCH_DEBOUNCE), move |_| Msg::SearchSettled(token));
            }
            Msg::SearchSettled(token) => {
                if token == self.search_token {
                    self.refresh_rows();
                    self.save(ctx);
                }
            }
            Msg::Year(year) => {
                if self.year == Some(year) {
                    return Task::none();
                }
                self.remember_year(Some(year), ctx);
                let guarded = !self.admin && self.access.status.as_ref().is_some_and(|s| s.scopes_enabled);
                if guarded {
                    ctx.api.set_scope_token(self.access.token_for(year));
                    self.access.expect_status();
                }
                self.refresh_rows();
                if guarded {
                    return access::fetch_status(ctx);
                }
            }
            Msg::Grid(event) => return self.grid_event(event, ctx),
            Msg::Escape => return self.escape(),
            Msg::FocusSearch => return operation::focus(Id::new(SEARCH_ID)),
            Msg::Newest => {
                let newest = Sort { column: Column::TransactionDate, descending: true };
                self.table.sort = if self.table.sort == newest { Sort::default() } else { newest };
                self.refresh_columns();
                self.refresh_rows();
                self.reveal_token += 1;
                self.reveal = Some((self.reveal_token, 0));
                self.save(ctx);
            }
            Msg::Page(page) => {
                let last = self.table.page_size.pages(self.matched.len()) as i64;
                let page = (page.clamp(1, last) - 1) as usize;
                if page != self.page {
                    self.turn_page(page);
                }
            }
            Msg::Quick(code) => {
                self.table.filters.transaction_types = code.map(|c| vec![c.to_string()]).unwrap_or_default();
                self.forget_filter_draft(ctx);
                self.refresh_rows();
                self.save(ctx);
            }
            Msg::PageSize(size) => {
                if self.table.page_size != size {
                    // The first row on screen stays on screen under the new size.
                    let first = self.offset;
                    self.table.page_size = size;
                    self.turn_page(size.page_of(first));
                    self.save(ctx);
                }
            }
            Msg::HighlightOff(id) => {
                if self.highlight == Some(id) {
                    self.highlight = None;
                }
            }
            Msg::ClearSelection => self.selection = Selection::default(),
            Msg::OpenAdd => {
                if !self.editing_blocked() {
                    self.sheet = Sheet::Add(AddForm::new());
                }
            }
            Msg::OpenFilters => self.open_filters(),
            Msg::OpenExcel => {
                if !self.editing_blocked() {
                    return self.open_excel(ctx);
                }
            }
            Msg::Edit(message) => return self.edit_update(message, ctx),
            Msg::Add(message) => return self.add_update(message, ctx),
            Msg::Filters(message) => return self.filters_update(message, ctx),
            Msg::Excel(message) => return self.excel_update(message, ctx),
            Msg::Access(message) => return self.access_update(message, ctx),
            Msg::Menu(message) => return self.menu_update(message, ctx),
            Msg::Detail(message) => self.detail_update(message),
        }
        Task::none()
    }

    /// What a test can see of the table, which it cannot read off the pixels.
    #[cfg(test)]
    pub fn peek(&self) -> Peek {
        Peek { loaded: self.loaded, total: self.data.len(), shown: self.matched.len(), year: self.year, page: self.page, on_page: self.rows.len() }
    }

    #[cfg(test)]
    pub fn row_id(&self, row: usize) -> Option<i64> {
        self.id_at(row)
    }

    #[cfg(test)]
    pub fn cell_text(&self, row: usize, column: Column) -> Option<String> {
        self.row_at(row).map(|r| r.cell(column).to_owned())
    }

    /// The middle of a cell, from the top left corner of the grid.
    #[cfg(test)]
    pub fn cell_offset(&self, row: usize, column: Column) -> Option<iced::Vector> {
        let index = self.columns.iter().position(|c| *c == column)?;
        let left: f32 = self.grid_columns[..index].iter().map(|c| c.width).sum();
        let height = self.table.density.row_height();
        Some(iced::Vector::new(left + self.grid_columns[index].width / 2.0, 30.0 + (row as f32 + 0.5) * height))
    }

    fn id_at(&self, row: usize) -> Option<i64> {
        self.row_at(row).map(|r| r.tx.id)
    }

    fn grid_event(&mut self, event: GridEvent, ctx: &mut Ctx) -> Task<Msg> {
        match event {
            GridEvent::Select(selection) => self.selection = selection,
            GridEvent::Click { row, col, .. } => {
                if self.columns.get(col).copied().is_some_and(opens_detail)
                    && let Some(id) = self.id_at(row)
                {
                    return self.open_detail(id, ctx);
                }
            }
            GridEvent::Activate { row, col, rect } => {
                if self.columns.get(col).copied().is_some_and(opens_detail) {
                    return Task::none();
                }
                return self.begin_edit(row, col, rect, None, ctx);
            }
            GridEvent::TypeChar { row, col, rect, text } => return self.begin_edit(row, col, rect, Some(text), ctx),
            GridEvent::Context { row, col, at } => self.popup = Popup::Cell { row, col, at },
            GridEvent::HeaderContext { col, at } => self.popup = Popup::Header { col, at },
            GridEvent::Sort(col) => {
                let column = self.columns[col];
                let descending = self.table.sort.column == column && !self.table.sort.descending;
                self.table.sort = Sort { column, descending };
                self.refresh_columns();
                self.refresh_rows();
                self.save(ctx);
            }
            GridEvent::Resize { col, width } => {
                self.table.widths.insert(self.columns[col], width);
                self.refresh_columns();
                self.save(ctx);
            }
            GridEvent::Move { from, to } => {
                let (moved, target) = (self.columns[from], self.columns[to]);
                let order = &mut self.table.order;
                if let (Some(a), Some(b)) = (order.iter().position(|c| *c == moved), order.iter().position(|c| *c == target)) {
                    let column = order.remove(a);
                    order.insert(b, column);
                }
                self.selection = Selection::default();
                self.refresh_columns();
                self.save(ctx);
            }
            GridEvent::Dismiss => return self.edit_update(edit::Msg::Commit, ctx),
            GridEvent::Key(key) => return self.grid_key(key, ctx),
        }
        Task::none()
    }

    fn grid_key(&mut self, key: GridKey, ctx: &mut Ctx) -> Task<Msg> {
        match key {
            GridKey::Delete => {
                let ids = self.selected_ids();
                self.ask_delete(ids);
                Task::none()
            }
            GridKey::Copy => match self.copy_selection() {
                Some(text) => {
                    ctx.toast(Tone::Success, "Скопировано в буфер обмена");
                    iced::clipboard::write(text)
                }
                None => Task::none(),
            },
            GridKey::Paste => iced::clipboard::read().map(|text| Msg::Edit(edit::Msg::Pasted(text))),
            GridKey::Undo => self.undo(ctx),
            GridKey::Redo => self.redo(ctx),
            // Comes through the window-wide hotkey, which sees it in text fields too.
            GridKey::Escape => Task::none(),
        }
    }

    /// Whether the tools above the table stand on one line. In a narrower page
    /// they take two, or the search box would be left with no width at all.
    pub(super) fn tools_on_one_line(&self, ctx: &Ctx) -> bool {
        ctx.window.width - ctx.rail - ui::PAGE_PADDING.left - ui::PAGE_PADDING.right >= ROOMY_PAGE
    }

    fn toolbar(&self, ctx: &Ctx) -> El<'_, Msg> {
        let locked = self.locked_view();
        let open = !self.editing_blocked();
        let head = row![
            ui::title("Транзакции"),
            ui::select(self.years.clone(), self.year, Msg::Year),
            ui::fill(),
            ui::hint_on(ui::btn_icon(Icon::Sheet, "Excel-синхронизация", Kind::Secondary, open.then_some(Msg::OpenExcel)), "Подключить книгу Excel к транзакциям"),
            ui::btn_icon(Icon::Refresh, "Обновить", Kind::Secondary, Some(Msg::Sync(true))),
            ui::btn_icon(Icon::Download, "Экспорт", Kind::Secondary, (!locked).then_some(Msg::Menu(menus::Msg::Open(Popup::Export)))),
            ui::btn_icon(Icon::Plus, "Добавить", Kind::Primary, open.then_some(Msg::OpenAdd)),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center);

        let s = &self.summary;
        let active = self.table.filters.active_count();
        let mut filter_caption = if active > 0 { format!("Фильтры · {active}") } else { "Фильтры".to_string() };
        // A form closed without «Применить» is still there, and the button says so.
        if self.has_filter_draft() {
            filter_caption.push_str(" · черновик");
        }
        let newest = self.table.sort == Sort { column: Column::TransactionDate, descending: true };
        let types = &self.table.filters.transaction_types;
        let picked = |code: &str| types.len() == 1 && types[0] == code;
        let quick = row![
            ui::count_chip("Все", s.any_type, types.is_empty(), Msg::Quick(None)),
            ui::count_chip("Списания", s.debits, picked("DEBIT"), Msg::Quick(Some("DEBIT"))),
            ui::count_chip("Пополнения", s.credits, picked("CREDIT"), Msg::Quick(Some("CREDIT"))),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center);
        let search = ui::input("Поиск: 1250000, \"Korzinka\", date:2026-01-28, status:ошибка", &self.search).id(Id::new(SEARCH_ID)).on_input(Msg::Search).width(Fill);
        let help = ui::icon_btn(Icon::Help, "Как искать", Some(Msg::Menu(menus::Msg::Open(Popup::Help))));
        let filters = ui::btn_icon(Icon::Filter, filter_caption, if active > 0 { Kind::Primary } else { Kind::Secondary }, Some(Msg::OpenFilters));
        let order = ui::btn_icon(Icon::Newest, "Сначала новые", if newest { Kind::Primary } else { Kind::Secondary }, Some(Msg::Newest));
        let look = ui::btn_icon(Icon::Columns, "Вид", Kind::Secondary, Some(Msg::Menu(menus::Msg::Open(Popup::View))));
        let tools: El<'_, Msg> = if self.tools_on_one_line(ctx) {
            row![quick, Space::new().width(space::SM), search, help, filters, order, look].spacing(space::SM).align_y(Alignment::Center).into()
        } else {
            // What the table shows stands above; what narrows it below, where the search box has the line to itself.
            column![
                row![quick, ui::fill(), order, look].spacing(space::SM).align_y(Alignment::Center),
                row![search, help, filters].spacing(space::SM).align_y(Alignment::Center),
            ]
            .spacing(space::SM)
            .into()
        };

        column![head, tools].spacing(space::MD).into()
    }

    fn footer(&self) -> El<'_, Msg> {
        let matched = self.matched.len();
        let mut caption = table::range_label(&(self.offset..self.offset + self.rows.len()), matched);
        if matched != self.data.len() {
            caption.push_str(&format!(" · всего {}", tbs_core::format::count(self.data.len())));
        }
        let mut line = row![ui::muted(caption), ui::fill()].spacing(space::SM).align_y(Alignment::Center);
        if !self.selection.is_empty() {
            let picked = self.selection.rows().len();
            line = line
                .push(ui::label(format!("{} выбрано", tbs_core::format::count(picked))))
                .push(ui::btn("Снять", Kind::Ghost, Some(Msg::ClearSelection)))
                .push(ui::btn("Удалить", Kind::Danger, (!self.editing_blocked()).then_some(Msg::Edit(edit::Msg::AskDelete(self.selected_ids())))))
                .push(ui::btn("Экспорт", Kind::Secondary, Some(Msg::Menu(menus::Msg::ExportSelected))));
        }
        let size = self.table.page_size;
        line = line
            .push(ui::muted("На странице"))
            .push(ui::select(PageSize::ALL, Some(size), Msg::PageSize).padding([4.0, 10.0]))
            .push(ui::pager(self.page as i64 + 1, size.pages(matched) as i64, Msg::Page));
        container(line).height(FOOTER_HEIGHT).width(Fill).into()
    }

    pub fn view(&self, ctx: &Ctx) -> El<'_, Msg> {
        let data = TxGrid { data: &self.data, rows: &self.rows, offset: self.offset, total: self.matched.len(), columns: &self.columns, table: &self.table, highlight: self.highlight };
        let body: El<'_, Msg> = if self.locked_view() {
            ui::empty_state(Icon::Lock, "Период закрыт", "Подтвердите код, чтобы увидеть транзакции.")
        } else if !self.loaded {
            ui::loading("Загружаем транзакции…")
        } else if self.rows.is_empty() {
            ui::empty_state(Icon::Table, "Транзакции не найдены", "Измените фильтры или добавьте новую транзакцию.")
        } else {
            grid::grid(data, &self.grid_columns, &self.selection, Msg::Grid)
                .pinned(self.table.pinned)
                .row_height(self.table.density.row_height())
                .font_size(self.table.density.font_size())
                .locked(self.editor.is_some())
                .reveal(self.reveal)
                .into()
        };

        let mut page = column![self.toolbar(ctx)].spacing(space::MD);
        for banner in self.banners(ctx) {
            page = page.push(banner);
        }
        // What narrows the table stands right above it, also while its rows are hidden.
        if let Some(bar) = self.filter_bar() {
            page = page.push(bar);
        }
        page = page.push(container(body).width(Fill).height(Fill).style(theme::card)).push(self.footer());
        container(page).padding(ui::PAGE_PADDING).width(Fill).height(Fill).style(theme::page).into()
    }

    /// Everything that floats above the page; drawn by the app over the whole window.
    pub fn overlay(&self, ctx: &Ctx) -> Option<El<'_, Msg>> {
        let mut layers: Vec<El<'_, Msg>> = Vec::new();
        match &self.sheet {
            Sheet::None => {}
            Sheet::Add(form) => layers.push(self.add_view(form)),
            Sheet::Filters(form) => layers.push(self.filters_view(form)),
            Sheet::Excel(panel) => layers.push(self.excel_view(panel)),
            Sheet::Detail(detail) => {
                if let Some(row) = self.data.by_id(detail.id) {
                    layers.push(self.detail_view(detail, row));
                }
            }
        }
        layers.extend(self.popup_view(ctx));
        layers.extend(self.editor_view(ctx));
        if let Some(ask) = &self.delete_ask {
            layers.push(ui::confirm(Space::new(), &ask.title, &ask.body, "Удалить", Msg::Edit(edit::Msg::ConfirmDelete), Msg::Edit(edit::Msg::CancelDelete)));
        }
        layers.extend(self.preset_ask_view());
        if layers.is_empty() {
            return None;
        }
        Some(opaque(Stack::with_children(layers).width(Fill).height(Fill)))
    }
}
