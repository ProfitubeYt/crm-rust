//! What the short codes in the client's book mean: articles and persons.

use std::borrow::Cow;

use iced::widget::{Id, Space, column, container, operation, row};
use iced::{Alignment, Fill, Point, Size, Task};
use serde_json::json;
use tbs_core::api::{AnnotationArticle, AnnotationPerson};
use tbs_core::format::count;

use crate::ctx::{Ctx, text_error};
use crate::grid::{self, CellStyle, GridColumn, GridData, GridEvent, GridKey, Selection};
use crate::theme::{self, DARK, LIGHT, Tone, space};
use crate::ui::{self, El, Icon, Kind};

const SEARCH_ID: &str = "dictionary-search";
const CODE_ID: &str = "dictionary-code";
const MENU: Size = Size::new(200.0, 84.0);

pub fn focus_search<T: Send + 'static>() -> Task<T> {
    operation::focus(Id::new(SEARCH_ID))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Book {
    #[default]
    Articles,
    Persons,
}

/// One line of either dictionary.
#[derive(Clone, Debug, Default, PartialEq)]
struct Entry {
    id: i64,
    code: String,
    label: String,
    direction: Option<&'static str>,
    is_active: bool,
    is_auto: bool,
}

impl Entry {
    fn status(&self) -> &'static str {
        match (self.is_active, self.is_auto) {
            (false, _) => "скрыт",
            (true, true) => "новый",
            (true, false) => "готов",
        }
    }
}

fn from_article(article: AnnotationArticle) -> Entry {
    let direction = if article.direction == "income" { "приход" } else { "расход" };
    Entry { id: article.id, code: article.code, label: article.label, direction: Some(direction), is_active: article.is_active, is_auto: article.is_auto }
}

fn from_person(person: AnnotationPerson) -> Entry {
    Entry { id: person.id, code: person.code, label: person.full_name.unwrap_or_default(), direction: None, is_active: person.is_active, is_auto: person.is_auto }
}

fn matches(entry: &Entry, needle: &str, only_new: bool) -> bool {
    if only_new && !entry.is_auto {
        return false;
    }
    needle.is_empty() || entry.code.to_lowercase().contains(needle) || entry.label.to_lowercase().contains(needle)
}

/// The server answers with stable codes; people read sentences.
fn error_text(raw: &str) -> String {
    if raw.contains("article_code_exists") {
        "Такой код статьи уже есть.".to_owned()
    } else if raw.contains("person_code_exists") {
        "Такой код лица уже есть.".to_owned()
    } else if raw.contains("code_required") {
        "Укажите код.".to_owned()
    } else {
        "Не удалось сохранить".to_owned()
    }
}

#[derive(Clone, Debug, Default)]
struct Form {
    id: Option<i64>,
    code: String,
    label: String,
    error: Option<String>,
    busy: bool,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Escape,
    Book(Book),
    Search(String),
    OnlyNew,
    Articles(Result<Vec<AnnotationArticle>, String>),
    Persons(Result<Vec<AnnotationPerson>, String>),
    Grid(GridEvent),
    CloseMenu,
    OpenAdd,
    OpenEdit(i64),
    CloseForm,
    FormCode(String),
    FormLabel(String),
    FormSubmit,
    FormDone(Result<(), String>),
    AskDelete(i64),
    CancelDelete,
    ConfirmDelete,
    Deleted(Result<(), String>),
}

pub struct Dictionary {
    book: Book,
    articles: Vec<Entry>,
    persons: Vec<Entry>,
    loaded: bool,
    error: Option<String>,
    search: String,
    only_new: bool,
    /// Positions into the active book that pass the filters.
    shown: Vec<usize>,
    selection: Selection,
    article_columns: Vec<GridColumn>,
    person_columns: Vec<GridColumn>,
    form: Option<Form>,
    menu: Option<(i64, Point)>,
    ask: Option<(i64, String)>,
    admin: bool,
}

struct Rows<'a> {
    entries: &'a [Entry],
    shown: &'a [usize],
    dark: bool,
}

impl Rows<'_> {
    fn entry(&self, row: usize) -> Option<&Entry> {
        self.shown.get(row).and_then(|index| self.entries.get(*index))
    }
}

impl GridData for Rows<'_> {
    fn row_count(&self) -> usize {
        self.shown.len()
    }

    fn cell(&self, row: usize, col: usize) -> Cow<'_, str> {
        let Some(entry) = self.entry(row) else { return Cow::Borrowed("") };
        match (col, entry.direction) {
            (0, _) => Cow::Borrowed(&entry.code),
            (1, _) if entry.label.is_empty() => Cow::Borrowed("— нет расшифровки —"),
            (1, _) => Cow::Borrowed(&entry.label),
            (2, Some(direction)) => Cow::Borrowed(direction),
            _ => Cow::Borrowed(entry.status()),
        }
    }

    fn cell_style(&self, row: usize, col: usize) -> CellStyle {
        let palette = if self.dark { &DARK } else { &LIGHT };
        let Some(entry) = self.entry(row) else { return CellStyle::default() };
        let last = if entry.direction.is_some() { 3 } else { 2 };
        let color = if !entry.is_active || (col == 1 && entry.label.is_empty()) {
            Some(palette.text3)
        } else if col == last {
            Some(if entry.is_auto { palette.warning } else { palette.success })
        } else {
            None
        };
        CellStyle { color, ..CellStyle::default() }
    }
}

impl Dictionary {
    pub fn new(ctx: &Ctx) -> Self {
        Self {
            book: Book::Articles,
            articles: Vec::new(),
            persons: Vec::new(),
            loaded: false,
            error: None,
            search: String::new(),
            only_new: false,
            shown: Vec::new(),
            selection: Selection::default(),
            article_columns: vec![GridColumn::new("Код", 140.0).mono().medium(), GridColumn::new("Расшифровка", 520.0), GridColumn::new("Направление", 110.0), GridColumn::new("Статус", 120.0)],
            person_columns: vec![GridColumn::new("Код", 140.0).mono().medium(), GridColumn::new("Расшифровка", 520.0), GridColumn::new("Статус", 120.0)],
            form: None,
            menu: None,
            ask: None,
            admin: ctx.is_admin(),
        }
    }

    pub fn load(&self, ctx: &Ctx) -> Task<Msg> {
        let (articles, persons) = (ctx.api.clone(), ctx.api.clone());
        Task::batch([
            Task::perform(async move { articles.annotation_articles(true).await.map_err(text_error) }, Msg::Articles),
            Task::perform(async move { persons.annotation_persons(true).await.map_err(text_error) }, Msg::Persons),
        ])
    }

    fn entries(&self) -> &[Entry] {
        match self.book {
            Book::Articles => &self.articles,
            Book::Persons => &self.persons,
        }
    }

    fn refresh(&mut self) {
        let needle = self.search.trim().to_lowercase();
        self.shown = self.entries().iter().enumerate().filter(|(_, entry)| matches(entry, &needle, self.only_new)).map(|(index, _)| index).collect();
        self.selection = Selection::default();
    }

    fn at(&self, row: usize) -> Option<&Entry> {
        self.shown.get(row).and_then(|index| self.entries().get(*index))
    }

    fn by_id(&self, id: i64) -> Option<&Entry> {
        self.entries().iter().find(|entry| entry.id == id)
    }

    fn pending(&self) -> usize {
        self.entries().iter().filter(|entry| entry.is_auto).count()
    }

    fn open_edit(&mut self, id: i64) -> Task<Msg> {
        self.menu = None;
        if !self.admin {
            return Task::none();
        }
        self.form = self.by_id(id).map(|entry| Form { id: Some(id), code: entry.code.clone(), label: entry.label.clone(), ..Form::default() });
        operation::focus(Id::new(CODE_ID))
    }

    fn on_grid(&mut self, event: GridEvent) -> Task<Msg> {
        match event {
            GridEvent::Select(selection) => self.selection = selection,
            GridEvent::Activate { row, .. } | GridEvent::TypeChar { row, .. } => {
                if let Some(id) = self.at(row).map(|entry| entry.id) {
                    return self.open_edit(id);
                }
            }
            GridEvent::Context { row, at, .. } => {
                if self.admin {
                    self.menu = self.at(row).map(|entry| (entry.id, at));
                }
            }
            GridEvent::Key(GridKey::Delete) => {
                if self.admin
                    && let Some(entry) = self.selection.rows().into_iter().next().and_then(|row| self.at(row as usize))
                {
                    self.ask = Some((entry.id, entry.code.clone()));
                }
            }
            GridEvent::Resize { col, width } => {
                let columns = if self.book == Book::Articles { &mut self.article_columns } else { &mut self.person_columns };
                if let Some(column) = columns.get_mut(col) {
                    column.width = width;
                }
            }
            _ => {}
        }
        Task::none()
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Escape => {
                if self.ask.take().is_some() || self.menu.take().is_some() || self.form.take().is_some() {
                } else if !self.search.is_empty() {
                    self.search.clear();
                    self.refresh();
                }
            }
            Msg::Book(book) => {
                self.book = book;
                self.refresh();
            }
            Msg::Search(value) => {
                self.search = value;
                self.refresh();
            }
            Msg::OnlyNew => {
                self.only_new = !self.only_new;
                self.refresh();
            }
            Msg::Articles(Ok(items)) => {
                self.articles = items.into_iter().map(from_article).collect();
                self.loaded = true;
                self.error = None;
                self.refresh();
            }
            Msg::Persons(Ok(items)) => {
                self.persons = items.into_iter().map(from_person).collect();
                self.refresh();
            }
            Msg::Articles(Err(reason)) | Msg::Persons(Err(reason)) => {
                self.loaded = true;
                self.error = Some(reason);
            }
            Msg::Grid(event) => return self.on_grid(event),
            Msg::CloseMenu => self.menu = None,
            Msg::OpenAdd => {
                if self.admin {
                    self.form = Some(Form::default());
                    return operation::focus(Id::new(CODE_ID));
                }
            }
            Msg::OpenEdit(id) => return self.open_edit(id),
            Msg::CloseForm => self.form = None,
            Msg::FormCode(value) => {
                if let Some(form) = &mut self.form {
                    form.code = value;
                    form.error = None;
                }
            }
            Msg::FormLabel(value) => {
                if let Some(form) = &mut self.form {
                    form.label = value;
                    form.error = None;
                }
            }
            Msg::FormSubmit => {
                let book = self.book;
                let Some(form) = &mut self.form else { return Task::none() };
                let (code, label) = (form.code.trim().to_owned(), form.label.trim().to_owned());
                if code.is_empty() {
                    form.error = Some("Укажите код.".to_owned());
                    return Task::none();
                }
                form.busy = true;
                let (id, api) = (form.id, ctx.api.clone());
                return Task::perform(
                    async move {
                        match (book, id) {
                            (Book::Articles, None) => api.create_article(&code, &label).await.map(|_| ()),
                            (Book::Persons, None) => api.create_person(&code, &label).await.map(|_| ()),
                            (Book::Articles, Some(id)) => api.update_article(id, &json!({ "code": code, "label": label })).await.map(|_| ()),
                            (Book::Persons, Some(id)) => api.update_person(id, &json!({ "code": code, "full_name": label })).await.map(|_| ()),
                        }
                        .map_err(text_error)
                    },
                    Msg::FormDone,
                );
            }
            Msg::FormDone(Ok(())) => {
                self.form = None;
                ctx.toast(Tone::Success, "Сохранено");
                return self.load(ctx);
            }
            Msg::FormDone(Err(reason)) => {
                if let Some(form) = &mut self.form {
                    form.busy = false;
                    form.error = Some(error_text(&reason));
                }
            }
            Msg::AskDelete(id) => {
                self.menu = None;
                self.ask = self.by_id(id).map(|entry| (id, entry.code.clone()));
            }
            Msg::CancelDelete => self.ask = None,
            Msg::ConfirmDelete => {
                let Some((id, _)) = self.ask.take() else { return Task::none() };
                let (book, api) = (self.book, ctx.api.clone());
                return Task::perform(
                    async move {
                        match book {
                            Book::Articles => api.delete_article(id).await,
                            Book::Persons => api.delete_person(id).await,
                        }
                        .map(|_| ())
                        .map_err(text_error)
                    },
                    Msg::Deleted,
                );
            }
            Msg::Deleted(Ok(())) => {
                ctx.toast(Tone::Success, "Удалено");
                return self.load(ctx);
            }
            Msg::Deleted(Err(reason)) => ctx.toast(Tone::Danger, format!("Не удалось удалить: {reason}")),
        }
        Task::none()
    }

    pub fn view(&self, ctx: &Ctx) -> El<'_, Msg> {
        let tab = |book: Book, caption: &str, amount: usize| ui::chip(format!("{caption} {}", count(amount)), self.book == book, Msg::Book(book));
        let pending = self.pending();
        let mut toolbar = row![
            tab(Book::Articles, "Статьи", self.articles.len()),
            tab(Book::Persons, "Лица", self.persons.len()),
            ui::input("Код или расшифровка", &self.search).id(Id::new(SEARCH_ID)).on_input(Msg::Search).width(Fill),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center);
        if pending > 0 || self.only_new {
            toolbar = toolbar.push(ui::hint_on(ui::chip(format!("Без расшифровки {}", count(pending)), self.only_new, Msg::OnlyNew), "Коды пришли из книги клиента и ждут расшифровки"));
        }
        if self.admin {
            toolbar = toolbar.push(ui::btn_icon(Icon::Plus, "Добавить", Kind::Primary, Some(Msg::OpenAdd)));
        }
        let body: El<'_, Msg> = if !self.loaded {
            ui::loading("Загружаем словари…")
        } else if let Some(reason) = &self.error {
            container(ui::notice(Tone::Danger, reason.as_str())).padding(space::MD).into()
        } else if self.shown.is_empty() {
            ui::empty_state(Icon::Search, "Ничего не найдено", "Измените запрос или добавьте код.")
        } else {
            let columns = if self.book == Book::Articles { &self.article_columns } else { &self.person_columns };
            grid::grid(Rows { entries: self.entries(), shown: &self.shown, dark: ctx.dark }, columns, &self.selection, Msg::Grid).row_height(32.0).font_size(13.0).into()
        };
        let header = ui::page_header("Разметка", Some("Что значат короткие коды из книги учёта: статьи и лица."), Space::new());
        column![header, toolbar, container(body).width(Fill).height(Fill).style(theme::card)].spacing(space::MD).height(Fill).into()
    }

    fn form_view<'a>(&'a self, form: &'a Form) -> El<'a, Msg> {
        let (code_hint, label_hint) = if self.book == Book::Articles { ("р1", "ФЗП,ФМП,МП") } else { ("СТ", "Имя") };
        let mut body = column![
            ui::dialog_title(if form.id.is_some() { "Изменить код" } else { "Новый код" }, Some(Msg::CloseForm)),
            ui::field("Код", ui::input(code_hint, &form.code).id(Id::new(CODE_ID)).on_input(Msg::FormCode).on_submit(Msg::FormSubmit)),
            ui::field("Расшифровка", ui::input(label_hint, &form.label).on_input(Msg::FormLabel).on_submit(Msg::FormSubmit)),
        ]
        .spacing(space::MD);
        if let Some(reason) = &form.error {
            body = body.push(ui::error_line(reason.as_str()));
        }
        body = body.push(
            row![
                ui::fill(),
                ui::btn("Отмена", Kind::Secondary, Some(Msg::CloseForm)),
                ui::btn(if form.busy { "Сохраняем…" } else { "Сохранить" }, Kind::Primary, (!form.busy).then_some(Msg::FormSubmit)),
            ]
            .spacing(space::SM),
        );
        ui::modal(Space::new(), body, 420.0, Some(Msg::CloseForm))
    }

    pub fn overlay(&self, ctx: &Ctx) -> Vec<El<'_, Msg>> {
        let mut layers = Vec::new();
        if let Some(form) = &self.form {
            layers.push(self.form_view(form));
        }
        if let Some((id, at)) = self.menu {
            let body = column![ui::menu_item(Icon::Edit, "Изменить", false, Msg::OpenEdit(id)), ui::menu_item(Icon::Trash, "Удалить", true, Msg::AskDelete(id))].spacing(space::XXS);
            layers.push(ui::floating(ui::menu_panel(body, MENU.width), at, MENU, ctx.window, Msg::CloseMenu));
        }
        if let Some((_, code)) = &self.ask {
            let text = format!("Код {code} будет удалён. Если он уже проставлен в транзакциях, он останется в базе и будет просто скрыт из списков.");
            let content = column![
                ui::heading("Удалить код?"),
                ui::label(text),
                row![ui::fill(), ui::btn("Отмена", Kind::Secondary, Some(Msg::CancelDelete)), ui::btn("Удалить", Kind::Danger, Some(Msg::ConfirmDelete))].spacing(space::SM),
            ]
            .spacing(space::MD);
            layers.push(ui::modal(Space::new(), content, 420.0, Some(Msg::CancelDelete)));
        }
        layers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(code: &str, label: &str, active: bool, auto: bool) -> Entry {
        Entry { id: 1, code: code.to_owned(), label: label.to_owned(), direction: None, is_active: active, is_auto: auto }
    }

    #[test]
    fn status_prefers_hidden_over_new() {
        assert_eq!(entry("а", "", false, true).status(), "скрыт");
        assert_eq!(entry("а", "", true, true).status(), "новый");
        assert_eq!(entry("а", "Х", true, false).status(), "готов");
    }

    #[test]
    fn filter_reads_code_and_label() {
        let one = entry("Р1", "Зарплата", true, false);
        assert!(matches(&one, "р1", false));
        assert!(matches(&one, "зарп", false));
        assert!(!matches(&one, "ст", false));
        assert!(!matches(&one, "", true));
    }

    #[test]
    fn income_is_the_only_income() {
        let income = AnnotationArticle { direction: "income".to_owned(), ..Default::default() };
        let other = AnnotationArticle { direction: "".to_owned(), ..Default::default() };
        assert_eq!(from_article(income).direction, Some("приход"));
        assert_eq!(from_article(other).direction, Some("расход"));
    }

    #[test]
    fn server_codes_become_sentences() {
        assert_eq!(error_text("article_code_exists"), "Такой код статьи уже есть.");
        assert_eq!(error_text("boom"), "Не удалось сохранить");
    }
}
