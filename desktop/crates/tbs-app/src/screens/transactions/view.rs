//! The sheet that arranges the table: its columns, the way it is drawn, and
//! the views kept under a name.
//!
//! It has no draft, unlike the filter sheet: a press here changes the table at
//! once, the way a drag on a column header does. What narrows the table is not
//! its business: «Сбросить вид» leaves the filters and the search alone.

use iced::widget::{Column as Lines, Space, button, center, column, container, row, svg, text};
use iced::{Alignment, Fill, Task, Theme};
use serde::{Deserialize, Serialize};
use tbs_core::columns::{self, Column};
use tbs_core::query::Sort;

use super::facets::shorten;
use super::menus::Popup;
use super::table::{Density, LOCKED, MAX_PINNED, PageSize, Step, TableView};
use super::{Msg as Screen, Sheet, Transactions};
use crate::ctx::Ctx;
use crate::grid::Selection;
use crate::theme::{self, Tone, control, size, space};
use crate::ui::{self, El, Icon, Kind};

const PRESETS: &str = "transactions-presets";
const WIDTH: f32 = 480.0;
const ARROW_GLYPH: f32 = 14.0;
/// A longer name of a saved view is cut in the list, so its buttons keep their place.
const NAME_LENGTH: usize = 44;
const NEWEST: Sort = Sort { column: Column::TransactionDate, descending: true };
const PAGE_SIZES: [(PageSize, &str); 4] = [(PageSize::Fifty, "50"), (PageSize::Hundred, "100"), (PageSize::TwoHundred, "200"), (PageSize::All, "Все")];
const PINS: [(usize, &str); MAX_PINNED + 1] = [(0, "Нет"), (1, "1"), (2, "2"), (3, "3")];

const NAME_MISSING: &str = "Введите название вида";
const NAME_TAKEN: &str = "Вид с таким названием уже есть";
const DEFAULT_HINT: &str = "С вида по умолчанию начинается таблица, у которой ещё нет своих настроек";

/// A part of the sheet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Columns,
    Display,
    Saved,
}

/// The sheet while it is open.
#[derive(Clone, Debug, Default)]
pub struct ViewPanel {
    tab: Tab,
    /// Narrows the list of columns. The table does not hear of it.
    search: String,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Open(Tab),
    Close,
    Tab(Tab),
    Search(String),
    Toggle(Column, bool),
    Move(Column, Step),
    ShowAll,
    HideAll,
    /// «Как в книге Excel»: the ledger's columns in the ledger's order.
    Ledger,
    ResetWidths,
    Density(Density),
    Pinned(usize),
    Name(String),
    Save(bool),
    Apply(usize),
    Default(usize),
    AskRename(usize),
    AskDelete(usize),
    /// «Сбросить вид»: the layout only, and only after a yes.
    AskReset,
    AskText(String),
    AskConfirm,
    AskClose,
}

/// A view kept under a name: the columns, the look, the sorting and the filters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub view: TableView,
    #[serde(default)]
    pub default: bool,
}

/// What the sheet asks before it does something that cannot be taken back.
pub enum Ask {
    Rename { index: usize, name: String },
    Delete { index: usize, title: String },
    Reset,
}

fn send(message: Msg) -> Screen {
    Screen::View(message)
}

/// Keeps one view of a name and at most one default. Says whether the name was there already.
pub fn store_preset(presets: &mut Vec<Preset>, name: &str, view: TableView, default: bool) -> bool {
    if default {
        presets.iter_mut().for_each(|p| p.default = false);
    }
    match presets.iter_mut().find(|p| p.name == name) {
        Some(preset) => {
            preset.view = view;
            preset.default |= default;
            true
        }
        None => {
            presets.push(Preset { name: name.to_string(), view, default });
            false
        }
    }
}

/// Why the view cannot take the name, in words for under the box.
fn rename_problem(presets: &[Preset], index: usize, name: &str) -> Option<&'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Some(NAME_MISSING);
    }
    let taken = presets.iter().enumerate().any(|(other, preset)| other != index && preset.name == name);
    taken.then_some(NAME_TAKEN)
}

/// What a saved view holds, in one line under its name.
fn describe(view: &TableView) -> String {
    let mut parts = vec![format!("Столбцов: {}", view.visible().len()), view.density.label().to_string()];
    match view.filters.active_count() {
        0 => {}
        count => parts.push(format!("фильтров: {count}")),
    }
    if !view.filters.search.trim().is_empty() {
        parts.push("поиск".to_string());
    }
    parts.join(" · ")
}

/// How the rows are ordered now, for the line under «Сначала новые».
fn sort_words(sort: &Sort) -> String {
    if *sort == NEWEST {
        "Сейчас: по дате, новые сверху.".to_string()
    } else if *sort == Sort::default() {
        "Сейчас: по дате, старые сверху.".to_string()
    } else {
        let way = if sort.descending { "по убыванию" } else { "по возрастанию" };
        format!("Сейчас: по столбцу «{}», {way}. Столбец сортировки выбирается нажатием на его заголовок.", sort.column.title())
    }
}

/// An arrow of a line of the list. One that leads nowhere is drawn faint.
fn arrow<'a>(glyph: Icon, hint: &'a str, on_press: Option<Msg>) -> El<'a, Screen> {
    let live = on_press.is_some();
    let image = svg(ui::icon_handle(glyph)).width(ARROW_GLYPH).height(ARROW_GLYPH).style(move |theme: &Theme, _| {
        let c = theme::p(theme);
        svg::Style { color: Some(if live { c.text2 } else { c.border_strong }) }
    });
    let press = button(center(image)).width(control::SM).height(control::SM).padding(0).style(theme::btn_ghost).on_press_maybe(on_press.map(send));
    ui::hint_on(press, hint)
}

/// One column of the list: its tick, whether it is pinned, and the arrows that move it.
fn column_line<'a>(column: Column, shown: bool, pinned: bool, arrows: Option<(bool, bool)>) -> El<'a, Screen> {
    let tick: El<'a, Screen> = if column == LOCKED {
        let fixed = iced::widget::checkbox(true).label(column.title()).size(16).text_size(size::BASE).style(theme::check);
        ui::hint_on(fixed, "Номер строки показан всегда")
    } else {
        ui::check(column.title(), shown, move |on| send(Msg::Toggle(column, on))).into()
    };
    let mut line = row![container(tick).width(Fill)].spacing(space::XS).align_y(Alignment::Center).height(control::SM);
    if pinned {
        line = line.push(ui::badge("закреплён", Tone::Accent));
    }
    if let Some((up, down)) = arrows {
        line = line
            .push(arrow(Icon::ChevronUp, "Выше в списке, левее в таблице", up.then_some(Msg::Move(column, Step::Up))))
            .push(arrow(Icon::ChevronDown, "Ниже в списке, правее в таблице", down.then_some(Msg::Move(column, Step::Down))));
    }
    line.into()
}

/// A setting of the «Отображение» tab: its name, its control, and what it does.
fn part<'a>(name: &'a str, control: impl Into<El<'a, Screen>>, hint: impl text::IntoFragment<'a>) -> El<'a, Screen> {
    ui::section(name, column![control.into(), ui::faint(hint)].spacing(space::XS))
}

impl Transactions {
    /// The saved views, read the way the table reads its own state: one saved
    /// by an older build is told from the table it makes today.
    pub(super) fn load_presets(ctx: &Ctx) -> Vec<Preset> {
        let mut presets: Vec<Preset> = tbs_core::config::load_json(&ctx.state_path(PRESETS));
        presets.iter_mut().for_each(|preset| preset.view.repair());
        presets
    }

    fn save_presets(&self, ctx: &Ctx) {
        if let Err(error) = tbs_core::config::save_json(&ctx.state_path(PRESETS), &self.presets) {
            tracing::warn!("presets not saved: {error}");
        }
    }

    /// Puts a whole view on: its columns, its look and its filters.
    fn apply_view(&mut self, view: TableView, ctx: &mut Ctx) {
        self.table = view;
        self.table.repair();
        self.search = self.table.filters.search.clone();
        self.forget_filter_draft(ctx);
        self.refresh_columns();
        self.refresh_rows();
    }

    /// The table as a saved view keeps it, with the search as it stands in its box.
    fn current_view(&self) -> TableView {
        let mut view = self.table.clone();
        view.filters.search = self.search.clone();
        view
    }

    /// The columns moved, came or went: the grid follows and the table is saved.
    fn columns_changed(&mut self, ctx: &Ctx) {
        self.selection = Selection::default();
        self.refresh_columns();
        self.save(ctx);
    }

    pub(super) fn open_view(&mut self, tab: Tab) {
        // A filter form nobody applied waits for its next opening, as after Esc.
        self.close_filters();
        self.popup = Popup::None;
        self.view_tab = tab;
        self.sheet = Sheet::View(ViewPanel { tab, search: String::new() });
    }

    pub(super) fn view_update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Screen> {
        match message {
            Msg::Open(tab) => self.open_view(tab),
            Msg::Close => {
                if matches!(self.sheet, Sheet::View(_)) {
                    self.sheet = Sheet::None;
                }
            }
            Msg::Tab(tab) => {
                self.view_tab = tab;
                if let Sheet::View(panel) = &mut self.sheet {
                    panel.tab = tab;
                }
            }
            Msg::Search(value) => {
                if let Sheet::View(panel) = &mut self.sheet {
                    panel.search = value;
                }
            }
            Msg::Toggle(column, shown) => {
                if column == LOCKED {
                    return Task::none();
                }
                self.table.hidden.retain(|c| *c != column);
                if !shown {
                    self.table.hidden.push(column);
                }
                self.columns_changed(ctx);
            }
            Msg::Move(column, step) => {
                if self.table.move_column(column, step) {
                    self.columns_changed(ctx);
                }
            }
            Msg::ShowAll => {
                self.table.hidden.clear();
                self.columns_changed(ctx);
            }
            Msg::HideAll => {
                self.table.hidden = columns::ALL.iter().copied().filter(|c| *c != LOCKED).collect();
                self.columns_changed(ctx);
            }
            Msg::Ledger => {
                self.table.use_ledger_columns();
                self.columns_changed(ctx);
            }
            Msg::ResetWidths => {
                self.table.widths.clear();
                self.columns_changed(ctx);
            }
            Msg::Density(density) => {
                self.table.density = density;
                self.save(ctx);
            }
            Msg::Pinned(count) => {
                self.table.pinned = count.min(MAX_PINNED);
                self.save(ctx);
            }
            Msg::Name(value) => self.preset_name = value,
            Msg::Save(default) => {
                let name = self.preset_name.trim().to_string();
                if name.is_empty() {
                    ctx.toast(Tone::Warning, NAME_MISSING);
                    return Task::none();
                }
                let view = self.current_view();
                let replaced = store_preset(&mut self.presets, &name, view, default);
                self.preset_name.clear();
                self.save_presets(ctx);
                ctx.toast(Tone::Success, if replaced { format!("Вид «{name}» заменён") } else { format!("Вид «{name}» сохранён") });
            }
            Msg::Apply(index) => {
                if let Some(preset) = self.presets.get(index) {
                    let view = preset.view.clone();
                    self.apply_view(view, ctx);
                    self.save(ctx);
                }
            }
            Msg::Default(index) => {
                let chosen = self.presets.get(index).is_some_and(|p| !p.default);
                self.presets.iter_mut().for_each(|p| p.default = false);
                if chosen {
                    self.presets[index].default = true;
                }
                self.save_presets(ctx);
            }
            Msg::AskRename(index) => {
                if let Some(preset) = self.presets.get(index) {
                    self.view_ask = Some(Ask::Rename { index, name: preset.name.clone() });
                }
            }
            Msg::AskDelete(index) => {
                if let Some(preset) = self.presets.get(index) {
                    self.view_ask = Some(Ask::Delete { index, title: format!("Удалить вид «{}»?", shorten(&preset.name, NAME_LENGTH)) });
                }
            }
            Msg::AskReset => self.view_ask = Some(Ask::Reset),
            Msg::AskText(value) => {
                if let Some(Ask::Rename { name, .. }) = &mut self.view_ask {
                    *name = value;
                }
            }
            Msg::AskConfirm => match self.view_ask.take() {
                Some(Ask::Rename { index, name }) => {
                    if rename_problem(&self.presets, index, &name).is_some() {
                        // The dialog says under its box what is wrong, and stays.
                        self.view_ask = Some(Ask::Rename { index, name });
                        return Task::none();
                    }
                    if let Some(preset) = self.presets.get_mut(index) {
                        preset.name = name.trim().to_string();
                    }
                    self.save_presets(ctx);
                }
                Some(Ask::Delete { index, .. }) => {
                    if index < self.presets.len() {
                        self.presets.remove(index);
                    }
                    self.save_presets(ctx);
                }
                Some(Ask::Reset) => {
                    // The layout only. The filters, the search and a filter form
                    // nobody applied are not the view's to throw away.
                    self.table.reset_layout();
                    self.refresh_columns();
                    self.refresh_rows();
                    self.save(ctx);
                    ctx.toast(Tone::Success, "Вид таблицы сброшен. Фильтры и поиск остались как были.");
                }
                None => {}
            },
            Msg::AskClose => self.view_ask = None,
        }
        Task::none()
    }

    /// What stays above the list of columns while the list scrolls.
    fn columns_bar<'a>(&'a self, panel: &'a ViewPanel) -> El<'a, Screen> {
        let table = &self.table;
        let shown = self.columns.len();
        let pinned = match table.pinned_shown() {
            0 => "закреплённых нет".to_string(),
            count => format!("закреплено слева: {count}"),
        };
        let tally = row![ui::strong(format!("Показано {shown} из {}", columns::COUNT)), ui::muted(pinned)].spacing(space::SM).align_y(Alignment::Center);
        let mut search = row![ui::input("Поиск столбца", &panel.search).on_input(|value| send(Msg::Search(value)))].spacing(space::XS).align_y(Alignment::Center);
        if !panel.search.is_empty() {
            search = search.push(ui::link("Очистить", Some(send(Msg::Search(String::new())))));
        }
        let actions = ui::chip_row(vec![
            ui::hint_on(ui::btn_small("Показать все", Kind::Secondary, (!table.hidden.is_empty()).then(|| send(Msg::ShowAll))), "Показать все столбцы, какие есть"),
            ui::hint_on(ui::btn_small("Как в книге Excel", Kind::Secondary, (!table.has_ledger_columns()).then(|| send(Msg::Ledger))), "Набор и порядок столбцов как в книге. Ширина остаётся."),
            ui::hint_on(ui::btn_small("Сбросить ширину", Kind::Secondary, (!table.widths.is_empty()).then(|| send(Msg::ResetWidths))), "Вернуть столбцам исходную ширину"),
            ui::hint_on(ui::btn_small("Скрыть все", Kind::Ghost, (shown > 1).then(|| send(Msg::HideAll))), "Оставить номер строки и отметить нужные столбцы заново"),
        ]);
        column![tally, search, actions].spacing(space::SM).into()
    }

    /// Every column in the order of the table, shown or not.
    fn columns_list<'a>(&'a self, panel: &'a ViewPanel) -> El<'a, Screen> {
        let table = &self.table;
        let needle = panel.search.trim().to_lowercase();
        let searching = !needle.is_empty();
        let pinned = table.pinned_shown();
        let last = table.order.len().saturating_sub(1);
        let mut lines = Lines::new().spacing(space::XXS);
        let (mut place, mut found) = (0, 0);
        for (index, &column) in table.order.iter().enumerate() {
            let shown = !table.hidden.contains(&column);
            let is_pinned = shown && place < pinned;
            place += usize::from(shown);
            if searching && !column.title().to_lowercase().contains(&needle) {
                continue;
            }
            found += 1;
            // A list cut by the search has gaps: a step in it would not be the step it looks like.
            let arrows = (!searching).then_some((index > 0, index < last));
            lines = lines.push(column_line(column, shown, is_pinned, arrows));
        }
        if found == 0 {
            return ui::faint("Такого столбца нет. Измените поиск.").into();
        }
        let note = if searching {
            "Стрелки порядка вернутся, когда поиск будет пуст."
        } else {
            "Сверху вниз в списке — слева направо в таблице. Закреплены первые показанные столбцы; сколько их, задаёт вкладка «Отображение»."
        };
        column![lines, ui::faint(note)].spacing(space::MD).into()
    }

    fn display_tab(&self) -> El<'_, Screen> {
        let table = &self.table;
        let densities = Density::ALL.iter().map(|density| ui::chip(density.label(), table.density == *density, send(Msg::Density(*density)))).collect();
        let newest = row![ui::switch(table.sort == NEWEST, |_| Screen::Newest), ui::label("Сначала новые")].spacing(space::SM).align_y(Alignment::Center);
        column![
            part("Плотность строк", ui::chip_row(densities), "Высота строки и размер текста в таблице."),
            part("Строк на странице", ui::segments(&PAGE_SIZES, table.page_size, Screen::PageSize), "«Все» показывает таблицу целиком, без страниц."),
            part("Закреплённые столбцы", ui::segments(&PINS, table.pinned, |count| send(Msg::Pinned(count))), "Столько первых показанных столбцов остаются на месте, когда таблица прокручивается вправо."),
            part("Порядок строк", newest, sort_words(&table.sort)),
        ]
        .spacing(space::LG)
        .into()
    }

    fn saved_tab(&self) -> El<'_, Screen> {
        let name = self.preset_name.trim();
        let ready = !name.is_empty();
        let replaces = self.presets.iter().any(|preset| preset.name == name);
        let save = column![
            ui::input("Название вида, например «Отчёт за месяц»", &self.preset_name).on_input(|value| send(Msg::Name(value))).on_submit(send(Msg::Save(false))),
            ui::chip_row(vec![
                ui::btn(if replaces { "Заменить вид" } else { "Сохранить вид" }, Kind::Primary, ready.then(|| send(Msg::Save(false)))).into(),
                ui::hint_on(ui::btn("Сохранить и по умолчанию", Kind::Secondary, ready.then(|| send(Msg::Save(true)))), DEFAULT_HINT),
            ]),
            ui::faint(if replaces { "Вид с таким названием уже есть: его заменит то, что сейчас в таблице." } else { "Вид запоминает столбцы, отображение, сортировку, фильтры и поиск." }),
        ]
        .spacing(space::SM);

        let mut list = Lines::new().spacing(space::SM);
        if self.presets.is_empty() {
            list = list.push(ui::muted("Пока нет сохранённых видов."));
        }
        for (index, preset) in self.presets.iter().enumerate() {
            // The same table to the last setting: applying it again would change nothing.
            let current = preset.view == self.table;
            let mut title = row![text(shorten(&preset.name, NAME_LENGTH)).size(size::BASE).font(theme::FONT_MEDIUM).wrapping(text::Wrapping::None)].spacing(space::SM).align_y(Alignment::Center);
            if preset.default {
                title = title.push(ui::badge("по умолчанию", Tone::Accent));
            }
            if current {
                title = title.push(ui::badge("применён", Tone::Success));
            }
            let actions = row![
                ui::btn_small("Применить", Kind::Secondary, (!current).then(|| send(Msg::Apply(index)))),
                ui::fill(),
                ui::icon_btn(Icon::Edit, "Переименовать", Some(send(Msg::AskRename(index)))),
                ui::icon_btn(Icon::Bookmark, if preset.default { "Больше не по умолчанию" } else { "Сделать видом по умолчанию" }, Some(send(Msg::Default(index)))),
                ui::icon_btn(Icon::Trash, "Удалить", Some(send(Msg::AskDelete(index)))),
            ]
            .spacing(space::XXS)
            .align_y(Alignment::Center);
            let card = column![container(title).width(Fill).clip(true), ui::faint(describe(&preset.view)), actions].spacing(space::XS);
            list = list.push(container(card).padding([space::SM, space::MD]).width(Fill).style(theme::group(current)));
        }
        column![ui::section("Сохранить текущий вид", save), ui::section("Сохранённые", list)].spacing(space::LG).into()
    }

    pub(super) fn view_sheet<'a>(&'a self, panel: &'a ViewPanel) -> El<'a, Screen> {
        let tab = |tab: Tab, caption: String| ui::chip(caption, panel.tab == tab, send(Msg::Tab(tab)));
        let saved = match self.presets.len() {
            0 => "Сохранённые виды".to_string(),
            count => format!("Сохранённые виды · {count}"),
        };
        let tabs = ui::chip_row(vec![tab(Tab::Columns, format!("Столбцы · {}", self.columns.len())), tab(Tab::Display, "Отображение".to_string()), tab(Tab::Saved, saved)]);
        let title = ui::sheet_head("Настройка", "Вид таблицы", None, "Изменения видны в таблице сразу и запоминаются.", send(Msg::Close));
        let mut head = column![title, tabs].spacing(space::MD);
        let body = match panel.tab {
            Tab::Columns => {
                head = head.push(self.columns_bar(panel));
                self.columns_list(panel)
            }
            Tab::Display => self.display_tab(),
            Tab::Saved => self.saved_tab(),
        };
        let fresh = self.table.has_fresh_layout();
        let reset = ui::hint_on(
            ui::btn_icon(Icon::Refresh, "Сбросить вид", Kind::Ghost, (!fresh).then(|| send(Msg::AskReset))),
            if fresh { "Столбцы и отображение уже исходные" } else { "Вернуть столбцы и отображение к исходным. Фильтры и поиск останутся." },
        );
        let footer = ui::sheet_foot(None, vec![reset], vec![ui::btn("Готово", Kind::Primary, Some(send(Msg::Close))).into()]);
        ui::sheet(Space::new(), ui::sheet_frame(head, body, Some(footer)), WIDTH, Some(send(Msg::Close)))
    }

    pub(super) fn view_ask_view(&self) -> Option<El<'_, Screen>> {
        Some(match self.view_ask.as_ref()? {
            Ask::Delete { title, .. } => ui::confirm(Space::new(), title, "Вид будет удалён без возможности восстановления. Таблица не изменится.", "Удалить", send(Msg::AskConfirm), send(Msg::AskClose)),
            Ask::Reset => ui::confirm(
                Space::new(),
                "Сбросить вид таблицы?",
                "Столбцы, их ширина и оформление, плотность, закрепление, сортировка и число строк на странице вернутся к исходным. Фильтры, поиск, оформление отдельных ячеек и сохранённые виды останутся.",
                "Сбросить",
                send(Msg::AskConfirm),
                send(Msg::AskClose),
            ),
            Ask::Rename { index, name } => {
                let problem = rename_problem(&self.presets, *index, name);
                let mut input = ui::input("Название вида", name).on_input(|value| send(Msg::AskText(value))).on_submit(send(Msg::AskConfirm));
                if problem.is_some() {
                    input = input.style(theme::input_error);
                }
                let body = column![
                    ui::dialog_title("Переименовать вид", Some(send(Msg::AskClose))),
                    ui::field_checked("Новое название", input, problem),
                    row![ui::fill(), ui::btn("Отмена", Kind::Ghost, Some(send(Msg::AskClose))), ui::btn("Сохранить", Kind::Primary, problem.is_none().then(|| send(Msg::AskConfirm)))].spacing(space::SM),
                ]
                .spacing(space::MD);
                ui::modal(Space::new(), body, 380, Some(send(Msg::AskClose)))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Local, NaiveDate};
    use tbs_core::query::{AnnotationState, Filters};

    use super::super::STATE;
    use super::super::facets::Facet;
    use super::super::filters::Msg as Narrow;
    use super::super::filters::tests::{Bench, SHOPS, SMALL_WINDOW, recent_rows, shots_folder};
    use super::super::table::{LAYOUT, Paint, QUIET, default_order};
    use super::*;

    const PRESETS_FIXTURE: &str = include_str!("fixtures/presets.json");

    fn act(bench: &mut Bench, message: Msg) {
        let _ = bench.screen.update(Screen::View(message), &mut bench.ctx);
    }

    fn tell(bench: &mut Bench, message: Screen) {
        let _ = bench.screen.update(message, &mut bench.ctx);
    }

    fn shown_of_all(shown: usize) -> String {
        format!("Показано {shown} из {}", columns::COUNT)
    }

    #[test]
    fn saving_a_name_again_replaces_the_view() {
        let mut presets = Vec::new();
        assert!(!store_preset(&mut presets, "Отчёт", TableView::default(), false));
        let wide = TableView { pinned: 1, ..Default::default() };
        assert!(store_preset(&mut presets, "Отчёт", wide, false));
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].view.pinned, 1);
    }

    #[test]
    fn only_one_preset_stays_default() {
        let mut presets = Vec::new();
        store_preset(&mut presets, "А", TableView::default(), true);
        store_preset(&mut presets, "Б", TableView::default(), true);
        assert_eq!(presets.iter().filter(|p| p.default).map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Б"]);
        // Saved again without the mark, the default one stays the default.
        store_preset(&mut presets, "Б", TableView { pinned: 0, ..Default::default() }, false);
        assert_eq!(presets.iter().filter(|p| p.default).map(|p| (p.name.as_str(), p.view.pinned)).collect::<Vec<_>>(), [("Б", 0)]);
    }

    #[test]
    fn the_tab_of_display_offers_every_page_size_and_pin_count() {
        assert_eq!(PAGE_SIZES.map(|(size, _)| size), PageSize::ALL);
        for (size, caption) in PAGE_SIZES {
            assert_eq!(size.to_string(), caption);
        }
        assert_eq!(PINS.map(|(count, _)| count).to_vec(), (0..=MAX_PINNED).collect::<Vec<_>>());
    }

    #[test]
    fn the_order_of_the_rows_and_a_saved_view_are_said_in_words() {
        assert_eq!(sort_words(&NEWEST), "Сейчас: по дате, новые сверху.");
        assert_eq!(sort_words(&Sort::default()), "Сейчас: по дате, старые сверху.");
        let by_amount = sort_words(&Sort { column: Column::Amount, descending: true });
        assert!(by_amount.starts_with("Сейчас: по столбцу «Сумма», по убыванию."), "{by_amount}");
        assert!(sort_words(&Sort { column: Column::Operation, descending: false }).contains("«Операция», по возрастанию"));

        assert_eq!(describe(&TableView::default()), "Столбцов: 19 · Средний");
        let narrowed = TableView {
            hidden: Vec::new(),
            density: Density::Ultra,
            filters: Filters { search: " такси ".into(), currency: Some("UZS".into()), amount_min: Some(1.0), amount_max: Some(2.0), ..Default::default() },
            ..Default::default()
        };
        assert_eq!(describe(&narrowed), format!("Столбцов: {} · Ультра-компактный · фильтров: 2 · поиск", columns::COUNT));
    }

    #[test]
    fn the_sheet_opens_on_the_tab_it_was_left_on() {
        let mut bench = Bench::new(true);
        bench.press("Вид");
        assert!(matches!(bench.screen.sheet, Sheet::View(_)));
        // Columns first: how many are shown, of all there are today.
        assert!(bench.sees("Вид таблицы") && bench.sees(&shown_of_all(19)) && bench.sees("закреплено слева: 3"));
        assert!(bench.sees("Столбцы · 19") && bench.sees("Отображение") && bench.sees("Сохранённые виды"));
        bench.press("Отображение");
        for name in ["Плотность строк", "Строк на странице", "Закреплённые столбцы", "Порядок строк"] {
            assert!(bench.sees(name), "no «{name}»");
        }
        assert!(!bench.sees("Показать все"));
        bench.press("Сохранённые виды");
        assert!(bench.sees("Сохранить текущий вид") && bench.sees("Пока нет сохранённых видов."));
        // Closed and opened from the toolbar again, it is where it was left.
        bench.press("Готово");
        assert!(matches!(bench.screen.sheet, Sheet::None));
        bench.press("Вид");
        assert!(bench.sees("Сохранить текущий вид"));
        // A tab can be asked for by name, as the screenshot hook does.
        act(&mut bench, Msg::Close);
        act(&mut bench, Msg::Open(Tab::Columns));
        assert!(bench.sees("Показать все"));
    }

    #[test]
    fn a_tick_shows_and_hides_a_column_at_once() {
        let mut bench = Bench::new(true);
        act(&mut bench, Msg::Open(Tab::Columns));
        assert!(!bench.screen.columns.contains(&Column::Description));
        // Found by the search and ticked by its label, as a person would.
        act(&mut bench, Msg::Search("опис".into()));
        bench.press("Описание");
        assert!(bench.screen.columns.contains(&Column::Description));
        assert!(bench.sees(&shown_of_all(20)) && bench.sees("Столбцы · 20"));
        assert!(!bench.on_disk().hidden.contains(&Column::Description));
        act(&mut bench, Msg::Search(String::new()));
        bench.press("Сумма");
        assert!(!bench.screen.columns.contains(&Column::Amount));
        assert!(bench.on_disk().hidden.contains(&Column::Amount));
        assert!(bench.sees(&shown_of_all(19)));
        // The row number stays, whatever is pressed or sent.
        assert!(!bench.try_press("№"));
        act(&mut bench, Msg::Toggle(LOCKED, false));
        assert_eq!(bench.screen.columns[0], LOCKED);
        // The grid has exactly the shown columns, in the order of the list.
        assert_eq!(bench.screen.columns, bench.screen.table.visible());
        assert_eq!(bench.screen.grid_columns.len(), bench.screen.columns.len());
        assert_eq!(bench.on_disk(), bench.screen.table);
    }

    #[test]
    fn arrows_move_a_column_and_the_table_follows() {
        let mut bench = Bench::new(true);
        act(&mut bench, Msg::Open(Tab::Columns));
        let place = |bench: &Bench, column: Column| bench.screen.columns.iter().position(|c| *c == column).unwrap();
        let before = place(&bench, Column::Amount);
        act(&mut bench, Msg::Move(Column::Amount, Step::Up));
        assert_eq!(place(&bench, Column::Amount), before - 1);
        assert_eq!(bench.on_disk().order, bench.screen.table.order);
        act(&mut bench, Msg::Move(Column::Amount, Step::Down));
        assert_eq!(bench.screen.table.order, default_order());

        // The ends hold, and a step that goes nowhere writes nothing.
        let (first, last) = (bench.screen.table.order[0], bench.screen.table.order[columns::COUNT - 1]);
        std::fs::remove_file(bench.ctx.state_path(STATE)).unwrap();
        act(&mut bench, Msg::Move(first, Step::Up));
        act(&mut bench, Msg::Move(last, Step::Down));
        assert_eq!(bench.screen.table.order, default_order());
        assert!(!bench.ctx.state_path(STATE).exists());

        // The first three shown are the pinned ones, and the list marks them.
        assert_eq!(bench.count("закреплён"), 3);
        assert_eq!(bench.screen.columns[..3], [Column::RowNumber, Column::OperationNumber, Column::DateTime]);
        // Moved to the front, a column is one of them, and the one it pushed out is not.
        for _ in 0..place(&bench, Column::Amount) {
            act(&mut bench, Msg::Move(Column::Amount, Step::Up));
        }
        assert_eq!(bench.screen.columns[..3], [Column::Amount, Column::RowNumber, Column::OperationNumber]);
        assert_eq!(bench.count("закреплён"), 3);
        // Fewer pinned, fewer marks; none pinned, and the tally says so.
        act(&mut bench, Msg::Pinned(1));
        assert_eq!(bench.count("закреплён"), 1);
        assert!(bench.sees("закреплено слева: 1"));
        act(&mut bench, Msg::Pinned(0));
        assert_eq!(bench.count("закреплён"), 0);
        assert!(bench.sees("закреплённых нет"));
    }

    #[test]
    fn the_search_narrows_the_list_and_leaves_the_table_alone() {
        let mut bench = Bench::new(true);
        act(&mut bench, Msg::Open(Tab::Columns));
        let columns = bench.screen.columns.clone();
        act(&mut bench, Msg::Search("  ПОЛУЧ ".into()));
        assert!(bench.sees("Получатель") && bench.sees("Карта получателя"));
        assert!(!bench.sees("Сумма") && !bench.sees("Остаток"));
        assert_eq!(bench.screen.columns, columns);
        assert!(bench.sees("Стрелки порядка вернутся, когда поиск будет пуст."));
        // The tally above the list is of the table, not of what the search left.
        assert!(bench.sees(&shown_of_all(19)));
        act(&mut bench, Msg::Search("нет такого".into()));
        assert!(bench.sees("Такого столбца нет. Измените поиск."));
        bench.press("Очистить");
        assert!(bench.sees("Сумма") && !bench.sees("Такого столбца нет. Измените поиск."));
        // The search is of one opening of the sheet.
        act(&mut bench, Msg::Search("сум".into()));
        act(&mut bench, Msg::Close);
        act(&mut bench, Msg::Open(Tab::Columns));
        assert!(bench.sees("Остаток"));
    }

    #[test]
    fn show_all_the_ledger_and_the_widths_each_have_a_button() {
        let mut bench = Bench::new(true);
        bench.screen.table.widths.insert(Column::OperatorRaw, 300.0);
        act(&mut bench, Msg::Open(Tab::Columns));
        // A fresh table is the ledger already: that button has nothing to do.
        assert!(!bench.try_press("Как в книге Excel"));
        bench.press("Показать все");
        assert_eq!(bench.screen.columns.len(), columns::COUNT);
        assert!(bench.sees(&shown_of_all(columns::COUNT)));
        assert!(!bench.try_press("Показать все"));
        act(&mut bench, Msg::Move(Column::Operation, Step::Down));

        bench.press("Как в книге Excel");
        assert_eq!((&bench.screen.table.order, &bench.screen.table.hidden), (&default_order(), &QUIET.to_vec()));
        assert_eq!(bench.screen.columns, TableView::default().visible());
        // The widths are another matter, and another button.
        assert_eq!(bench.screen.table.width(Column::OperatorRaw), 300.0);
        bench.press("Сбросить ширину");
        assert!(bench.screen.table.widths.is_empty());
        assert!(!bench.try_press("Сбросить ширину"));
        assert_eq!(bench.on_disk(), bench.screen.table);

        bench.press("Скрыть все");
        assert_eq!(bench.screen.columns, [LOCKED]);
        assert!(bench.sees(&shown_of_all(1)) && bench.sees("закреплено слева: 1"));
        assert!(!bench.try_press("Скрыть все"));
        assert_eq!(bench.on_disk().hidden.len(), columns::COUNT - 1);
    }

    #[test]
    fn the_tab_of_display_changes_the_table_at_once() {
        let mut bench = Bench::new(true);
        act(&mut bench, Msg::Open(Tab::Display));
        bench.press("Компактный");
        assert_eq!(bench.screen.table.density, Density::Compact);
        bench.press("200");
        assert_eq!(bench.screen.table.page_size, PageSize::TwoHundred);
        bench.press("Нет");
        assert_eq!(bench.screen.table.pinned, 0);
        act(&mut bench, Msg::Pinned(9));
        assert_eq!(bench.screen.table.pinned, MAX_PINNED);
        assert_eq!(bench.on_disk(), bench.screen.table);

        // The switch and the button of the toolbar are one setting.
        assert!(bench.sees("Сейчас: по дате, старые сверху."));
        tell(&mut bench, Screen::Newest);
        assert_eq!(bench.screen.table.sort, NEWEST);
        assert!(bench.sees("Сейчас: по дате, новые сверху."));
        tell(&mut bench, Screen::Newest);
        assert_eq!(bench.screen.table.sort, Sort::default());
        // Another column sorted by its header is told as it is.
        bench.screen.table.sort = Sort { column: Column::Amount, descending: true };
        assert_eq!(bench.count(&sort_words(&bench.screen.table.sort)), 1);
    }

    #[test]
    fn resetting_the_view_asks_first_and_leaves_what_narrows_the_table() {
        let mut bench = Bench::new(true);
        bench.screen.search = "маркет".into();
        bench.screen.table.filters = Filters { currency: Some("UZS".into()), ..Default::default() };
        bench.screen.refresh_rows();
        let narrowed = (bench.applied().clone(), bench.rows_shown());
        assert!(narrowed.1 > 0 && narrowed.1 < 60);

        act(&mut bench, Msg::Open(Tab::Columns));
        // A table as it comes has nothing to reset.
        assert!(!bench.try_press("Сбросить вид"));
        act(&mut bench, Msg::Toggle(Column::Amount, false));
        act(&mut bench, Msg::Move(Column::Operation, Step::Up));
        act(&mut bench, Msg::Density(Density::Compact));
        act(&mut bench, Msg::Pinned(1));
        tell(&mut bench, Screen::PageSize(PageSize::Fifty));
        tell(&mut bench, Screen::Newest);
        bench.screen.table.widths.insert(Column::OperatorRaw, 300.0);
        bench.screen.table.column_paint.insert(Column::BalanceAfter, Paint { fill: Some(2), align: None });
        bench.screen.table.cell_paint.insert((7, Column::Amount), Paint { fill: Some(1), align: None });
        let arranged = bench.screen.table.clone();
        assert!(!arranged.has_fresh_layout());

        // The button asks, and the question alone changes nothing.
        bench.press("Сбросить вид");
        assert!(bench.sees("Сбросить вид таблицы?"));
        assert_eq!(bench.screen.table, arranged);
        bench.press("Отмена");
        assert!(bench.screen.view_ask.is_none());
        assert_eq!(bench.screen.table, arranged);

        bench.ctx.toasts.clear();
        bench.press("Сбросить вид");
        bench.press("Сбросить");
        let fresh = TableView::default();
        assert!(bench.screen.table.has_fresh_layout());
        assert_eq!(bench.screen.columns, fresh.visible());
        assert_eq!((bench.screen.table.density, bench.screen.table.pinned, bench.screen.table.page_size, bench.screen.table.sort), (fresh.density, fresh.pinned, fresh.page_size, fresh.sort));
        // What narrows the table is as it was, to the row; and so is a cell somebody marked.
        assert_eq!((bench.applied().clone(), bench.rows_shown()), narrowed);
        assert_eq!(bench.screen.search, "маркет");
        assert_eq!(bench.screen.table.cell_paint.len(), 1);
        assert_eq!(bench.on_disk(), bench.screen.table);
        assert_eq!(bench.ctx.toasts.len(), 1);
        assert!(bench.ctx.toasts[0].1.contains("Фильтры и поиск остались"), "{:?}", bench.ctx.toasts);
        // The sheet stays open, with nothing left to reset.
        assert!(matches!(bench.screen.sheet, Sheet::View(_)));
        assert!(!bench.try_press("Сбросить вид"));
    }

    #[test]
    fn a_view_is_saved_applied_renamed_made_default_and_deleted() {
        let mut bench = Bench::new(true);
        bench.screen.table.filters = Filters { currency: Some("USD".into()), ..Default::default() };
        bench.screen.refresh_rows();
        act(&mut bench, Msg::Open(Tab::Saved));
        act(&mut bench, Msg::Toggle(Column::Description, true));
        act(&mut bench, Msg::Density(Density::Compact));

        // Without a name there is nothing to save: the buttons are off, and Enter says why.
        assert!(!bench.try_press("Сохранить вид") && !bench.try_press("Сохранить и по умолчанию"));
        act(&mut bench, Msg::Name("   ".into()));
        act(&mut bench, Msg::Save(false));
        assert!(bench.screen.presets.is_empty());
        assert_eq!(bench.ctx.toasts.last(), Some(&(Tone::Warning, NAME_MISSING.to_string())));

        act(&mut bench, Msg::Name("  Доллары  ".into()));
        bench.press("Сохранить вид");
        let saved = bench.screen.table.clone();
        assert_eq!(bench.screen.presets.iter().map(|p| (p.name.as_str(), p.default)).collect::<Vec<_>>(), [("Доллары", false)]);
        assert_eq!(bench.screen.presets[0].view, saved);
        assert_eq!(bench.screen.preset_name, "");
        assert!(bench.sees("Сохранённые виды · 1") && bench.sees("применён"));
        assert!(bench.sees("Столбцов: 20 · Компактный · фильтров: 1"));
        let on_disk = Transactions::load_presets(&bench.ctx);
        assert_eq!((on_disk.len(), &on_disk[0].view), (1, &saved));

        // The table moves on; the saved view does not, and brings everything back when applied.
        act(&mut bench, Msg::Density(Density::Standard));
        act(&mut bench, Msg::Toggle(Column::Description, false));
        tell(&mut bench, Screen::Filters(Narrow::RemoveAll));
        assert_eq!(bench.rows_shown(), 60);
        assert!(!bench.sees("применён"));
        bench.press("Применить");
        assert_eq!(bench.screen.table, saved);
        assert_eq!(bench.rows_shown(), 6);
        assert!(bench.screen.columns.contains(&Column::Description));
        assert!(bench.sees("применён") && !bench.try_press("Применить"));
        assert_eq!(bench.on_disk(), saved);

        // The same name again replaces the view, and the sheet says that it will.
        act(&mut bench, Msg::Pinned(0));
        act(&mut bench, Msg::Name("Доллары".into()));
        assert!(bench.sees("Заменить вид") && !bench.sees("Сохранить вид"));
        bench.press("Заменить вид");
        assert_eq!((bench.screen.presets.len(), bench.screen.presets[0].view.pinned), (1, 0));

        // A second one, saved as the default.
        tell(&mut bench, Screen::Filters(Narrow::RemoveAll));
        act(&mut bench, Msg::Name("Всё".into()));
        bench.press("Сохранить и по умолчанию");
        assert_eq!(bench.screen.presets.iter().map(|p| (p.name.as_str(), p.default)).collect::<Vec<_>>(), [("Доллары", false), ("Всё", true)]);
        assert!(bench.sees("по умолчанию") && bench.sees("Сохранённые виды · 2"));

        // A name another view has is refused where it is typed; so is no name at all.
        act(&mut bench, Msg::AskRename(1));
        assert!(bench.sees("Переименовать вид"));
        act(&mut bench, Msg::AskText("Доллары".into()));
        assert!(bench.sees(NAME_TAKEN));
        act(&mut bench, Msg::AskConfirm);
        assert!(bench.screen.view_ask.is_some());
        act(&mut bench, Msg::AskText("  ".into()));
        assert!(bench.sees(NAME_MISSING));
        act(&mut bench, Msg::AskConfirm);
        assert_eq!(bench.screen.presets[1].name, "Всё");
        act(&mut bench, Msg::AskText("  Всё подряд ".into()));
        assert!(!bench.sees(NAME_TAKEN) && !bench.sees(NAME_MISSING));
        bench.press("Сохранить");
        assert!(bench.screen.view_ask.is_none());
        assert_eq!(bench.screen.presets[1].name, "Всё подряд");
        // Its own name back is no clash.
        act(&mut bench, Msg::AskRename(1));
        act(&mut bench, Msg::AskConfirm);
        assert!(bench.screen.view_ask.is_none());

        // One default at most; pressed on the default one, the mark comes off.
        act(&mut bench, Msg::Default(0));
        assert_eq!(bench.screen.presets.iter().map(|p| p.default).collect::<Vec<_>>(), [true, false]);
        act(&mut bench, Msg::Default(0));
        assert_eq!(bench.screen.presets.iter().filter(|p| p.default).count(), 0);

        // Deleting asks, and takes the view only.
        let table = bench.screen.table.clone();
        act(&mut bench, Msg::AskDelete(0));
        assert!(bench.sees("Удалить вид «Доллары»?"));
        bench.press("Отмена");
        assert_eq!(bench.screen.presets.len(), 2);
        act(&mut bench, Msg::AskDelete(0));
        bench.press("Удалить");
        assert_eq!(bench.screen.presets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Всё подряд"]);
        assert_eq!(bench.screen.table, table);
        assert_eq!(Transactions::load_presets(&bench.ctx).iter().map(|p| p.name.clone()).collect::<Vec<_>>(), ["Всё подряд"]);
    }

    #[test]
    fn applying_a_view_drops_a_filter_draft_and_opening_the_sheet_keeps_it() {
        let mut bench = Bench::new(true);
        act(&mut bench, Msg::Name("Как есть".into()));
        act(&mut bench, Msg::Save(false));
        bench.screen.open_filters();
        tell(&mut bench, Screen::Filters(Narrow::Operator(SHOPS[0].into())));
        // «Вид» opened over the form: the form waits, as it does after Esc.
        act(&mut bench, Msg::Open(Tab::Saved));
        assert!(matches!(bench.screen.sheet, Sheet::View(_)));
        assert!(bench.screen.has_filter_draft());
        // Columns and display are not filters: the draft still waits.
        act(&mut bench, Msg::Toggle(Column::Amount, false));
        act(&mut bench, Msg::Density(Density::Ultra));
        assert!(bench.screen.has_filter_draft());
        // A saved view brings its own filters, and a form made from the old ones goes.
        bench.ctx.toasts.clear();
        act(&mut bench, Msg::Apply(0));
        assert!(!bench.screen.has_filter_draft());
        assert_eq!(bench.ctx.toasts.len(), 1);
    }

    #[test]
    fn the_views_saved_by_older_builds_load_and_apply() {
        let day = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d);
        let mut bench = Bench::new(true);
        std::fs::write(bench.ctx.state_path(PRESETS), PRESETS_FIXTURE).unwrap();
        bench.screen.presets = Transactions::load_presets(&bench.ctx);
        let presets = bench.screen.presets.clone();
        assert_eq!(presets.iter().map(|p| (p.name.as_str(), p.default)).collect::<Vec<_>>(), [("Ноябрь · Telegram", true), ("Без разметки", false), ("Старый вид", false)]);
        // Each has every column there is today, once, whatever build saved it.
        for preset in &presets {
            let mut order = preset.view.order.clone();
            order.sort_by_key(|c| c.index());
            assert_eq!(order, columns::ALL, "{}", preset.name);
            assert_eq!(preset.view.layout, LAYOUT, "{}", preset.name);
        }

        // The untouched ledger view of the build before «Операция»: today's ledger, the column in its place.
        let november = &presets[0].view;
        assert_eq!((&november.order, &november.hidden), (&default_order(), &QUIET.to_vec()));
        assert_eq!(november.sort, NEWEST);
        assert_eq!(
            november.filters,
            Filters { date_from: day(1), date_to: day(30), transaction_types: vec!["DEBIT".into()], source_channel: Some("TELEGRAM".into()), source_chat_ids: vec![-1001234567890], ..Default::default() }
        );

        // One arranged by hand («Описание» shown) keeps its columns; the new one waits unticked at the end.
        let unmarked = &presets[1].view;
        assert_eq!(unmarked.hidden, [Column::TransactionDate, Column::Time, Column::Day, Column::Operation]);
        assert_eq!(unmarked.order.last(), Some(&Column::Operation));
        assert_eq!(unmarked.visible().len(), columns::COUNT - 4);
        assert!(unmarked.visible().contains(&Column::Description));
        assert_eq!((unmarked.width(Column::Description), unmarked.density, unmarked.pinned, unmarked.page_size), (320.0, Density::Compact, 1, PageSize::All));
        assert_eq!(unmarked.sort, Sort { column: Column::Amount, descending: true });
        assert_eq!(
            unmarked.filters,
            Filters { search: "такси".into(), amount_min: Some(50000.0), currency: Some("UZS".into()), confidence_max: Some(0.6), annotation_state: AnnotationState::Missing, ..Default::default() }
        );

        // The untouched default of the first builds moves to the ledger; what it did set stays.
        let old = &presets[2].view;
        assert_eq!((&old.order, &old.hidden), (&default_order(), &QUIET.to_vec()));
        assert_eq!((old.density, old.pinned, old.page_size, old.sort), (Density::Comfortable, MAX_PINNED, PageSize::Hundred, Sort::default()));
        assert_eq!(
            old.filters,
            Filters { days_of_week: vec![6, 0], operators: vec!["Korzinka".into()], annotation_period_from: Some(202601), annotation_state: AnnotationState::Annotated, ..Default::default() }
        );

        // Applied, each is the table to the last setting, and the list marks the one that is on.
        act(&mut bench, Msg::Open(Tab::Saved));
        for (index, preset) in presets.iter().enumerate() {
            act(&mut bench, Msg::Apply(index));
            assert_eq!(bench.screen.table, preset.view, "{}", preset.name);
            assert_eq!(bench.screen.search, preset.view.filters.search);
            assert_eq!(bench.screen.columns, preset.view.visible());
            assert_eq!(bench.on_disk(), preset.view);
            assert_eq!(bench.count("применён"), 1, "{}", preset.name);
            // The filter sheet takes what the view brought, and has nothing of its own to say.
            bench.screen.open_filters();
            act(&mut bench, Msg::Open(Tab::Saved));
            assert!(!bench.screen.has_filter_draft(), "{}", preset.name);
        }
        // Written back by this build, the file reads as the same views.
        act(&mut bench, Msg::Default(1));
        let again = Transactions::load_presets(&bench.ctx);
        assert_eq!(again.len(), presets.len());
        for (index, (written, read)) in again.iter().zip(&presets).enumerate() {
            assert_eq!((&written.name, &written.view, written.default), (&read.name, &read.view, index == 1));
        }
    }

    #[test]
    fn a_default_view_starts_a_table_that_has_no_state_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::for_tests(dir.path());
        std::fs::write(ctx.state_path(PRESETS), PRESETS_FIXTURE).unwrap();
        let (fresh, _) = Transactions::new(&ctx);
        assert_eq!(fresh.table, fresh.presets[0].view);
        assert_eq!((fresh.table.sort, fresh.table.filters.source_channel.as_deref()), (NEWEST, Some("TELEGRAM")));
        // A table somebody arranged already is that person's, default view or not.
        let own = TableView { pinned: 1, ..Default::default() };
        tbs_core::config::save_json(&ctx.state_path(STATE), &own).unwrap();
        let (kept, _) = Transactions::new(&ctx);
        assert_eq!(kept.table, own);
    }

    #[test]
    fn escape_closes_the_question_first_and_then_the_sheet() {
        let mut bench = Bench::new(true);
        act(&mut bench, Msg::Open(Tab::Columns));
        act(&mut bench, Msg::Toggle(Column::Amount, false));
        act(&mut bench, Msg::AskReset);
        bench.escape();
        assert!(bench.screen.view_ask.is_none() && matches!(bench.screen.sheet, Sheet::View(_)));
        bench.escape();
        assert!(matches!(bench.screen.sheet, Sheet::None));
        // The sheet has no draft: what was pressed in it stays pressed.
        assert!(!bench.screen.columns.contains(&Column::Amount));
    }

    #[test]
    fn the_sheet_fits_the_smallest_window() {
        let mut bench = Bench::new(true);
        bench.resize(SMALL_WINDOW);
        let sheet = SMALL_WINDOW.width - WIDTH;
        let inside = |bench: &Bench, words: &str| {
            let at = bench.place(words).unwrap_or_else(|| panic!("«{words}» is out of sight"));
            assert!(at.x >= sheet && at.x + at.width <= SMALL_WINDOW.width && at.y >= 0.0 && at.y + at.height <= SMALL_WINDOW.height, "«{words}» sticks out of the sheet: {at:?}");
        };
        act(&mut bench, Msg::Open(Tab::Columns));
        for words in ["Вид таблицы", "Столбцы · 19", "Отображение", "Сохранённые виды", "Показать все", "Как в книге Excel", "Сбросить ширину", "Скрыть все", "Сбросить вид", "Готово", "№", "Сумма"] {
            inside(&bench, words);
        }
        inside(&bench, &shown_of_all(19));
        // The list scrolls under its own search; the head and the foot stay where they are.
        let (list, foot) = (bench.place("№").unwrap(), bench.place("Готово").unwrap());
        assert!(list.y > bench.place("Показать все").unwrap().y && list.y < foot.y, "list {list:?}, foot {foot:?}");

        act(&mut bench, Msg::Tab(Tab::Display));
        for words in ["Плотность строк", "Ультра-компактный", "Компактный", "Средний", "Крупный", "Строк на странице", "50", "Закреплённые столбцы", "Нет", "Порядок строк", "Готово"] {
            inside(&bench, words);
        }

        // A name far too long is cut in the list, and the buttons of its view keep their place.
        act(&mut bench, Msg::Tab(Tab::Saved));
        let long = "Отчёт для бухгалтерии за третий квартал две тысячи двадцать шестого года, только списания в сумах";
        act(&mut bench, Msg::Name(long.into()));
        act(&mut bench, Msg::Save(true));
        assert_eq!(bench.screen.presets[0].name, long);
        let cut = shorten(long, NAME_LENGTH);
        for words in [cut.as_str(), "по умолчанию", "Применить", "Сохранить вид", "Сохранить и по умолчанию", "Готово"] {
            inside(&bench, words);
        }
        act(&mut bench, Msg::AskDelete(0));
        assert!(bench.shows_whole(&format!("Удалить вид «{cut}»?")));
        assert!(bench.shows_whole("Удалить") && bench.shows_whole("Отмена"));
    }

    /// Saves pictures of the three tabs and of the table they arrange. Only when asked:
    /// `TBS_SHOTS=<folder> cargo test -p tbs-app --features devtools p4_shots`.
    #[test]
    fn p4_shots() {
        let Some(folder) = shots_folder() else { return };
        let today = Local::now().date_naive();
        let mut bench = Bench::with(true, recent_rows(today));
        tell(&mut bench, Screen::Year(today.year()));

        // The columns of a fresh table: the ledger's nineteen, three of them pinned.
        act(&mut bench, Msg::Open(Tab::Columns));
        bench.shoot(&folder, "p4-columns");
        act(&mut bench, Msg::Search("получ".into()));
        bench.shoot(&folder, "p4-columns-search");
        act(&mut bench, Msg::Search(String::new()));

        // Arranged by hand: a column shown, one hidden, «Сумма» moved up beside the date, one pinned less.
        act(&mut bench, Msg::Toggle(Column::Description, true));
        act(&mut bench, Msg::Toggle(Column::IsP2p, false));
        for _ in 0..5 {
            act(&mut bench, Msg::Move(Column::Amount, Step::Up));
        }
        bench.screen.table.widths.insert(Column::OperatorRaw, 180.0);
        act(&mut bench, Msg::Pinned(2));
        bench.shoot(&folder, "p4-columns-arranged");

        act(&mut bench, Msg::Tab(Tab::Display));
        act(&mut bench, Msg::Density(Density::Compact));
        tell(&mut bench, Screen::Newest);
        bench.shoot(&folder, "p4-display");

        // Saved views: the one that is on, a default one, and a name too long for its line.
        act(&mut bench, Msg::Tab(Tab::Saved));
        act(&mut bench, Msg::Name("Сверка с книгой".into()));
        act(&mut bench, Msg::Save(true));
        bench.screen.table.filters = Filters { currency: Some("USD".into()), amount_min: Some(10_000.0), ..Default::default() };
        bench.screen.refresh_rows();
        act(&mut bench, Msg::Density(Density::Comfortable));
        act(&mut bench, Msg::Name("Отчёт для бухгалтерии за третий квартал, только доллары и крупные суммы".into()));
        act(&mut bench, Msg::Save(false));
        act(&mut bench, Msg::Apply(0));
        act(&mut bench, Msg::Name("Сверка с книгой".into()));
        bench.ctx.toasts.clear();
        bench.shoot(&folder, "p4-saved");
        act(&mut bench, Msg::Name(String::new()));
        act(&mut bench, Msg::AskRename(1));
        act(&mut bench, Msg::AskText("Сверка с книгой".into()));
        bench.shoot(&folder, "p4-saved-rename");
        act(&mut bench, Msg::AskClose);

        // The question before the look is reset, and the table before and after: the filters stay on.
        bench.screen.table.filters = Filters { transaction_types: vec!["DEBIT".into()], amount_min: Some(10_000.0), ..Default::default() };
        bench.screen.refresh_rows();
        tell(&mut bench, Screen::Filters(Narrow::Remove(Facet::Currency)));
        act(&mut bench, Msg::Tab(Tab::Columns));
        act(&mut bench, Msg::AskReset);
        bench.shoot(&folder, "p4-reset-ask");
        act(&mut bench, Msg::AskClose);
        act(&mut bench, Msg::Close);
        bench.shoot(&folder, "p4-table-arranged");
        act(&mut bench, Msg::AskReset);
        act(&mut bench, Msg::AskConfirm);
        bench.ctx.toasts.clear();
        bench.shoot(&folder, "p4-table-reset");

        // The smallest window, and the dark theme.
        bench.resize(SMALL_WINDOW);
        act(&mut bench, Msg::Open(Tab::Columns));
        bench.shoot(&folder, "p4-small-columns");
        act(&mut bench, Msg::Tab(Tab::Saved));
        bench.shoot(&folder, "p4-small-saved");
        bench.ctx.dark = true;
        act(&mut bench, Msg::Tab(Tab::Display));
        bench.shoot(&folder, "p4-small-display-dark");
    }
}
