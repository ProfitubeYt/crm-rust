//! Merchant descriptions, bound to operators and copied onto their receipts.

use std::borrow::Cow;
use std::time::Duration;

use iced::widget::{Id, Space, column, container, operation, row};
use iced::{Alignment, Fill, Rectangle, Task};
use tbs_core::api::{Description, DescriptionPage};
use tbs_core::format::count;

use crate::ctx::{Ctx, text_error};
use crate::grid::{self, GridColumn, GridData, GridEvent, GridKey, Selection};
use crate::theme::{self, Tone, space};
use crate::ui::{self, El, Icon, Kind};

const SEARCH_ID: &str = "descriptions-search";
const EDITOR_ID: &str = "descriptions-editor";
const TEXT_ID: &str = "descriptions-text";
const PAGE_SIZE: i64 = 100;
const DEBOUNCE: Duration = Duration::from_millis(240);
const SHEET_WIDTH: f32 = 440.0;
const SHOWN_OPTIONS: usize = 8;

const COL_TEXT: usize = 0;
const COL_OPERATORS: usize = 1;

pub fn focus_search<T: Send + 'static>() -> Task<T> {
    operation::focus(Id::new(SEARCH_ID))
}

/// Search over the operator names, with the picked ones kept above the list.
#[derive(Clone, Debug, Default)]
struct Picker {
    query: String,
    picked: Vec<String>,
}

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Options still free to pick, matching what was typed.
fn available<'a>(options: &'a [String], query: &str, taken: &[String]) -> Vec<&'a String> {
    let needle = query.trim().to_lowercase();
    options
        .iter()
        .filter(|name| !taken.iter().any(|t| same(t, name)))
        .filter(|name| needle.is_empty() || name.to_lowercase().contains(&needle))
        .take(SHOWN_OPTIONS)
        .collect()
}

#[derive(Clone, Debug, Default)]
struct AddForm {
    text: String,
    picker: Picker,
    error: Option<String>,
    busy: bool,
}

struct Editor {
    id: i64,
    value: String,
    original: String,
    rect: Rectangle,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Escape,
    Search(String),
    SearchSettled(u64),
    Page(i64),
    Loaded(u64, Result<DescriptionPage, String>),
    Options(Result<Vec<String>, String>),
    Grid(GridEvent),
    EditText(String),
    EditCommit,
    Saved(Result<Description, String>),
    OpenAdd,
    CloseAdd,
    AddText(String),
    PickerQuery(String),
    Pick(String),
    Unpick(String),
    AddSubmit,
    AddDone(Result<Description, String>),
    CloseLinks,
    Link(String),
    Unlink(String),
    Linked(&'static str, Result<Description, String>),
    AskDelete(i64),
    CancelDelete,
    ConfirmDelete,
    Deleted(Result<(), String>),
}

pub struct Descriptions {
    loaded: bool,
    error: Option<String>,
    items: Vec<Description>,
    total: i64,
    page: i64,
    search: String,
    search_token: u64,
    load_token: u64,
    selection: Selection,
    columns: Vec<GridColumn>,
    options: Vec<String>,
    editor: Option<Editor>,
    add: Option<AddForm>,
    links: Option<(i64, Picker)>,
    ask: Option<(i64, String)>,
}

struct Rows<'a>(&'a [Description]);

impl GridData for Rows<'_> {
    fn row_count(&self) -> usize {
        self.0.len()
    }

    fn cell(&self, row: usize, col: usize) -> Cow<'_, str> {
        let Some(item) = self.0.get(row) else { return Cow::Borrowed("") };
        match col {
            COL_TEXT => Cow::Borrowed(&item.text),
            COL_OPERATORS if item.operator_keys.is_empty() => Cow::Borrowed("нет привязок"),
            COL_OPERATORS => Cow::Owned(item.operator_keys.join(", ")),
            _ if item.sources.is_empty() => Cow::Borrowed(tbs_core::format::EMPTY),
            _ => Cow::Owned(item.sources.join(", ")),
        }
    }
}

impl Descriptions {
    pub fn new(_ctx: &Ctx) -> Self {
        Self {
            loaded: false,
            error: None,
            items: Vec::new(),
            total: 0,
            page: 1,
            search: String::new(),
            search_token: 0,
            load_token: 0,
            selection: Selection::default(),
            columns: vec![GridColumn::new("Описание", 420.0).medium(), GridColumn::new("Операторы", 420.0), GridColumn::new("Источник", 160.0)],
            options: Vec::new(),
            editor: None,
            add: None,
            links: None,
            ask: None,
        }
    }

    fn load(&mut self, ctx: &Ctx) -> Task<Msg> {
        self.load_token += 1;
        let (token, api, page, search) = (self.load_token, ctx.api.clone(), self.page, self.search.trim().to_owned());
        Task::perform(
            async move {
                let mut query = vec![("page", page.to_string()), ("page_size", PAGE_SIZE.to_string())];
                if !search.is_empty() {
                    query.push(("search", search));
                }
                api.descriptions(&query).await.map_err(text_error)
            },
            move |reply| Msg::Loaded(token, reply),
        )
    }

    pub fn load_all(&mut self, ctx: &Ctx) -> Task<Msg> {
        let api = ctx.api.clone();
        let options = Task::perform(async move { api.transactions_init().await.map(|init| init.operators).map_err(text_error) }, Msg::Options);
        Task::batch([self.load(ctx), options])
    }

    fn find(&self, id: i64) -> Option<&Description> {
        self.items.iter().find(|item| item.id == id)
    }

    fn pages(&self) -> i64 {
        ((self.total + PAGE_SIZE - 1) / PAGE_SIZE).max(1)
    }

    fn begin_edit(&mut self, row: usize, rect: Rectangle, typed: Option<String>) -> Task<Msg> {
        let Some(item) = self.items.get(row) else { return Task::none() };
        self.editor = Some(Editor { id: item.id, value: typed.unwrap_or_else(|| item.text.clone()), original: item.text.clone(), rect });
        let id = Id::new(EDITOR_ID);
        Task::batch([operation::focus(id.clone()), operation::move_cursor_to_end(id)])
    }

    fn commit(&mut self, ctx: &Ctx) -> Task<Msg> {
        let Some(editor) = self.editor.take() else { return Task::none() };
        let value = editor.value.trim().to_owned();
        if value.is_empty() || value == editor.original {
            return Task::none();
        }
        let api = ctx.api.clone();
        Task::perform(async move { api.update_description(editor.id, &value).await.map_err(text_error) }, Msg::Saved)
    }

    fn replace(&mut self, item: Description) {
        if let Some(row) = self.items.iter_mut().find(|row| row.id == item.id) {
            *row = item;
        }
    }

    fn on_grid(&mut self, event: GridEvent, ctx: &mut Ctx) -> Task<Msg> {
        match event {
            GridEvent::Select(selection) => self.selection = selection,
            GridEvent::Click { row, col, .. } if col == COL_OPERATORS => {
                if let Some(item) = self.items.get(row) {
                    self.links = Some((item.id, Picker::default()));
                }
            }
            GridEvent::Activate { row, col, rect } if col == COL_TEXT => return self.begin_edit(row, rect, None),
            GridEvent::TypeChar { row, col, rect, text } if col == COL_TEXT => return self.begin_edit(row, rect, Some(text)),
            GridEvent::Resize { col, width } => {
                if let Some(column) = self.columns.get_mut(col) {
                    column.width = width;
                }
            }
            GridEvent::Dismiss => return self.commit(ctx),
            GridEvent::Context { row, .. } => {
                if let Some(item) = self.items.get(row) {
                    self.ask = Some((item.id, item.text.clone()));
                }
            }
            GridEvent::Key(GridKey::Delete) => {
                if let Some(item) = self.selection.rows().into_iter().next().and_then(|row| self.items.get(row as usize)) {
                    self.ask = Some((item.id, item.text.clone()));
                }
            }
            _ => {}
        }
        Task::none()
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Escape => {
                if self.editor.take().is_some() || self.ask.take().is_some() || self.links.take().is_some() || self.add.take().is_some() {
                } else if !self.search.is_empty() {
                    return self.update(Msg::Search(String::new()), ctx);
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
            Msg::Page(page) => {
                self.page = page;
                return self.load(ctx);
            }
            Msg::Loaded(token, reply) => {
                if token != self.load_token {
                    return Task::none();
                }
                self.loaded = true;
                match reply {
                    Ok(page) => {
                        self.error = None;
                        self.total = page.total;
                        self.items = page.items;
                        self.selection = Selection::default();
                    }
                    Err(reason) => {
                        tracing::warn!("descriptions not loaded: {reason}");
                        self.error = Some(reason);
                    }
                }
            }
            Msg::Options(Ok(options)) => self.options = options,
            Msg::Options(Err(reason)) => tracing::warn!("operator options not loaded: {reason}"),
            Msg::Grid(event) => return self.on_grid(event, ctx),
            Msg::EditText(value) => {
                if let Some(editor) = &mut self.editor {
                    editor.value = value;
                }
            }
            Msg::EditCommit => return self.commit(ctx),
            Msg::Saved(Ok(item)) => self.replace(item),
            Msg::Saved(Err(reason)) => ctx.toast(Tone::Danger, format!("Не удалось сохранить: {reason}")),
            Msg::OpenAdd => {
                self.add = Some(AddForm::default());
                return operation::focus(Id::new(TEXT_ID));
            }
            Msg::CloseAdd => self.add = None,
            Msg::AddText(value) => {
                if let Some(form) = &mut self.add {
                    form.text = value;
                    form.error = None;
                }
            }
            Msg::PickerQuery(value) => {
                if let Some(picker) = self.picker() {
                    picker.query = value;
                }
            }
            Msg::Pick(name) => {
                if let Some(picker) = self.picker() {
                    picker.query.clear();
                    picker.picked.push(name);
                }
            }
            Msg::Unpick(name) => {
                if let Some(picker) = self.picker() {
                    picker.picked.retain(|picked| *picked != name);
                }
            }
            Msg::AddSubmit => {
                let Some(form) = &mut self.add else { return Task::none() };
                let text = form.text.trim().to_owned();
                if text.is_empty() {
                    form.error = Some("Текст описания обязателен".to_owned());
                    return Task::none();
                }
                form.busy = true;
                let (operators, api) = (form.picker.picked.clone(), ctx.api.clone());
                return Task::perform(async move { api.create_description(&text, &operators).await.map_err(text_error) }, Msg::AddDone);
            }
            Msg::AddDone(Ok(_)) => {
                self.add = None;
                ctx.toast(Tone::Success, "Описание добавлено");
                return self.load(ctx);
            }
            Msg::AddDone(Err(reason)) => {
                if let Some(form) = &mut self.add {
                    form.busy = false;
                    form.error = Some(format!("Не удалось добавить: {reason}"));
                }
            }
            Msg::CloseLinks => self.links = None,
            Msg::Link(name) => {
                let Some((id, picker)) = &mut self.links else { return Task::none() };
                picker.query.clear();
                let (id, api) = (*id, ctx.api.clone());
                return Task::perform(async move { api.link_operators(id, &[name]).await.map_err(text_error) }, |reply| Msg::Linked("Не удалось привязать", reply));
            }
            Msg::Unlink(name) => {
                let Some((id, _)) = &self.links else { return Task::none() };
                let (id, api) = (*id, ctx.api.clone());
                return Task::perform(async move { api.unlink_operators(id, &[name]).await.map_err(text_error) }, |reply| Msg::Linked("Не удалось отвязать", reply));
            }
            Msg::Linked(_, Ok(item)) => {
                let linked = item.operator_keys.len() > self.find(item.id).map_or(0, |old| old.operator_keys.len());
                self.replace(item);
                if linked {
                    ctx.toast(Tone::Success, "Оператор привязан");
                }
            }
            Msg::Linked(lead, Err(reason)) => ctx.toast(Tone::Danger, format!("{lead}: {reason}")),
            Msg::AskDelete(id) => self.ask = self.find(id).map(|item| (id, item.text.clone())),
            Msg::CancelDelete => self.ask = None,
            Msg::ConfirmDelete => {
                let Some((id, _)) = self.ask.take() else { return Task::none() };
                let api = ctx.api.clone();
                return Task::perform(async move { api.delete_description(id).await.map(|_| ()).map_err(text_error) }, Msg::Deleted);
            }
            Msg::Deleted(Ok(())) => {
                ctx.toast(Tone::Success, "Описание удалено");
                return self.load(ctx);
            }
            Msg::Deleted(Err(reason)) => ctx.toast(Tone::Danger, format!("Не удалось удалить: {reason}")),
        }
        Task::none()
    }

    fn picker(&mut self) -> Option<&mut Picker> {
        self.links.as_mut().map(|(_, picker)| picker).or(self.add.as_mut().map(|form| &mut form.picker))
    }

    pub fn view(&self, _ctx: &Ctx) -> El<'_, Msg> {
        let actions = ui::btn_icon(Icon::Plus, "Добавить описание", Kind::Primary, Some(Msg::OpenAdd));
        let header = ui::page_header("Справочник описаний", Some("Человекочитаемые описания мерчантов. Описание привязывается к операторам и подставляется в каждый чек с тем же продавцом."), actions);
        let search = ui::input("Поиск по описанию или оператору", &self.search).id(Id::new(SEARCH_ID)).on_input(Msg::Search).width(Fill);
        let body: El<'_, Msg> = if !self.loaded {
            ui::loading("Загружаем описания…")
        } else if self.error.is_some() {
            ui::empty_state(Icon::Alert, "Ошибка загрузки", "Не удалось получить список описаний. Повторите попытку позже.")
        } else if self.items.is_empty() {
            let hint = if self.search.trim().is_empty() { "Добавьте первое описание через кнопку «Добавить описание»." } else { "Ничего не найдено. Измените запрос." };
            ui::empty_state(Icon::Book, "Описаний нет", hint)
        } else {
            grid::grid(Rows(&self.items), &self.columns, &self.selection, Msg::Grid).row_height(32.0).font_size(13.0).locked(self.editor.is_some()).into()
        };
        let footer = row![ui::muted(format!("{} всего · нажмите на операторов, чтобы изменить привязки", count(self.total as usize))), ui::fill(), ui::pager(self.page, self.pages(), Msg::Page)].spacing(space::SM).align_y(Alignment::Center);
        column![header, search, container(body).width(Fill).height(Fill).style(theme::card), footer].spacing(space::MD).height(Fill).into()
    }

    fn options_list<'a>(&'a self, picker: &'a Picker, taken: &[String], pick: fn(String) -> Msg) -> El<'a, Msg> {
        let mut list = column![ui::input("Выберите операторов…", &picker.query).on_input(Msg::PickerQuery)].spacing(space::XXS);
        for name in available(&self.options, &picker.query, taken) {
            list = list.push(ui::menu_item(Icon::Plus, name.as_str(), false, pick(name.clone())));
        }
        list.into()
    }

    fn add_view<'a>(&'a self, form: &'a AddForm) -> El<'a, Msg> {
        let mut chosen = column![].spacing(space::XXS);
        for name in &form.picker.picked {
            chosen = chosen.push(row![ui::label(name.as_str()), ui::fill(), ui::icon_btn(Icon::Close, "Убрать", Some(Msg::Unpick(name.clone())))].align_y(Alignment::Center));
        }
        let mut body = column![
            ui::field("Описание", ui::input("Например: Продукты · Korzinka", &form.text).id(Id::new(TEXT_ID)).on_input(Msg::AddText).on_submit(Msg::AddSubmit)),
            ui::field("Операторы (необязательно)", self.options_list(&form.picker, &form.picker.picked, Msg::Pick)),
            chosen,
        ]
        .spacing(space::MD);
        if let Some(reason) = &form.error {
            body = body.push(ui::error_line(reason.as_str()));
        }
        let footer = row![
            ui::fill(),
            ui::btn("Отмена", Kind::Secondary, Some(Msg::CloseAdd)),
            ui::btn(if form.busy { "Сохраняем…" } else { "Добавить" }, Kind::Primary, (!form.busy).then_some(Msg::AddSubmit)),
        ]
        .spacing(space::SM);
        let panel = ui::sheet_panel("Справочник · описания → operator", "Новое описание", "описание → operator", Msg::CloseAdd, body, Some(footer.into()));
        ui::sheet(Space::new(), panel, SHEET_WIDTH, Some(Msg::CloseAdd))
    }

    fn links_view<'a>(&'a self, item: &'a Description, picker: &'a Picker) -> El<'a, Msg> {
        let mut linked = column![].spacing(space::XXS);
        if item.operator_keys.is_empty() {
            linked = linked.push(ui::muted("нет привязок"));
        }
        for name in &item.operator_keys {
            linked = linked.push(row![ui::label(name.as_str()), ui::fill(), ui::icon_btn(Icon::Unlink, "Отвязать", Some(Msg::Unlink(name.clone())))].align_y(Alignment::Center));
        }
        let body = column![ui::section("Привязанные операторы", linked), ui::section("Добавить оператора", self.options_list(picker, &item.operator_keys, Msg::Link))].spacing(space::LG);
        let footer = row![ui::btn("Удалить описание", Kind::Danger, Some(Msg::AskDelete(item.id))), ui::fill(), ui::btn("Готово", Kind::Primary, Some(Msg::CloseLinks))].spacing(space::SM);
        let panel = ui::sheet_panel("Справочник · описания → operator", "Операторы описания", item.text.as_str(), Msg::CloseLinks, body, Some(footer.into()));
        ui::sheet(Space::new(), panel, SHEET_WIDTH, Some(Msg::CloseLinks))
    }

    pub fn overlay(&self, ctx: &Ctx) -> Vec<El<'_, Msg>> {
        let mut layers = Vec::new();
        if let Some(form) = &self.add {
            layers.push(self.add_view(form));
        }
        if let Some((id, picker)) = &self.links
            && let Some(item) = self.find(*id)
        {
            layers.push(self.links_view(item, picker));
        }
        if let Some(editor) = &self.editor {
            layers.push(ui::cell_editor(Id::new(EDITOR_ID), &editor.value, editor.rect, ctx.window, Msg::EditText, Msg::EditCommit, Msg::EditCommit));
        }
        if let Some((_, text)) = &self.ask {
            let body = format!("Будет удалено: «{text}». Привязки операторов также снимутся.");
            layers.push(ask_view(body));
        }
        layers
    }
}

fn ask_view<'a>(body: String) -> El<'a, Msg> {
    let content = column![
        ui::heading("Удалить описание?"),
        ui::label(body),
        row![ui::fill(), ui::btn("Отмена", Kind::Secondary, Some(Msg::CancelDelete)), ui::btn("Удалить", Kind::Danger, Some(Msg::ConfirmDelete))].spacing(space::SM),
    ]
    .spacing(space::MD);
    ui::modal(Space::new(), content, 420.0, Some(Msg::CancelDelete))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taken_operators_are_not_offered_again() {
        let options = vec!["Korzinka".to_owned(), "Makro".to_owned(), "Havas".to_owned()];
        let taken = vec!["korzinka".to_owned()];
        let names: Vec<&str> = available(&options, "", &taken).into_iter().map(String::as_str).collect();
        assert_eq!(names, ["Makro", "Havas"]);
    }

    #[test]
    fn search_ignores_case_and_caps_the_list() {
        let options: Vec<String> = (0..20).map(|n| format!("Shop {n}")).collect();
        assert_eq!(available(&options, "SHOP", &[]).len(), SHOWN_OPTIONS);
        assert_eq!(available(&options, "shop 19", &[]).len(), 1);
    }

    #[test]
    fn empty_links_read_as_such() {
        let items = [Description { id: 1, text: "x".to_owned(), ..Default::default() }];
        assert_eq!(Rows(&items).cell(0, COL_OPERATORS), "нет привязок");
        assert_eq!(Rows(&items).cell(0, 2), tbs_core::format::EMPTY);
    }
}
