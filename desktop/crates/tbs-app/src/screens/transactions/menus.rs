//! Small floating panels: the cell and header menus, the view, export and search help.

use chrono::{Local, NaiveDate};
use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Fill, Point, Size, Task};
use serde::{Deserialize, Serialize};
use tbs_core::columns::{self, Column};
use tbs_core::export::selected_rows_xlsx;
use tbs_core::query::{Filters, Row, Scope, export_params};

use super::table::{Density, FILLS, Justify, LOCKED, MAX_PINNED, Paint, TableView, operation_number};
use super::{Msg as Screen, Transactions};
use crate::ctx::{Ctx, text_error};
use crate::files::save_as;
use crate::grid::Selection;
use crate::theme::{self, Tone, size, space};
use crate::ui::{self, El, Icon, Kind};

const PRESETS: &str = "transactions-presets";
const CELL_WIDTH: f32 = 220.0;
const VIEW_WIDTH: f32 = 320.0;
const EXPORT_WIDTH: f32 = 290.0;
const HELP_WIDTH: f32 = 360.0;
/// Where the floating panels of the toolbar begin, under its second line.
const UNDER_TOOLBAR: f32 = 104.0;

const SEARCH_HELP: [(&str, &str); 6] = [
    ("amount:100000", "точная сумма"),
    ("date:2026-01-28", "дата"),
    ("status:ошибка", "статус"),
    ("seller:korzinka", "оператор"),
    ("-status:дубликат", "исключить"),
    ("\"фраза\"", "точное совпадение"),
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Popup {
    #[default]
    None,
    Cell {
        row: usize,
        col: usize,
        at: Point,
    },
    Header {
        col: usize,
        at: Point,
    },
    View,
    Export,
    Help,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Open(Popup),
    Close,
    Align(Justify),
    Fill(Option<u8>),
    HideColumn,
    DeleteRow,
    ToggleColumn(Column, bool),
    ShowAll,
    HideAll,
    Density(Density),
    Pinned(usize),
    Reset,
    PresetName(String),
    PresetSave(bool),
    PresetLoad(usize),
    PresetDefault(usize),
    PresetAskRename(usize),
    PresetAskDelete(usize),
    PresetAskText(String),
    PresetAskConfirm,
    PresetAskClose,
    ExportView,
    ExportAll,
    ExportFrom(String),
    ExportTo(String),
    ExportPeriod,
    ExportSelected,
    Exported(&'static str, Result<Option<String>, String>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub view: TableView,
    #[serde(default)]
    pub default: bool,
}

pub enum PresetAsk {
    Rename { index: usize, name: String },
    Delete { index: usize, title: String },
}

fn send(message: Msg) -> Screen {
    Screen::Menu(message)
}

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn period(from: &str, to: &str) -> Result<(NaiveDate, NaiveDate), &'static str> {
    let (from, to) = (from.trim(), to.trim());
    if from.is_empty() || to.is_empty() {
        return Err("Укажите период (дата начала и дата конца).");
    }
    let parse = |raw: &str| NaiveDate::parse_from_str(raw, "%d.%m.%Y").ok();
    match (parse(from), parse(to)) {
        (Some(from), Some(to)) if from <= to => Ok((from, to)),
        _ => Err("Период указан некорректно."),
    }
}

/// Keeps one preset of a name and at most one default.
pub fn store_preset(presets: &mut Vec<Preset>, name: &str, view: TableView, default: bool) {
    if default {
        presets.iter_mut().for_each(|p| p.default = false);
    }
    match presets.iter_mut().find(|p| p.name == name) {
        Some(preset) => {
            preset.view = view;
            preset.default |= default;
        }
        None => presets.push(Preset { name: name.to_string(), view, default }),
    }
}

fn floating<'a>(panel: El<'a, Screen>, at: Point, panel_size: Size, window: Size) -> El<'a, Screen> {
    ui::floating(panel, at, panel_size, window, send(Msg::Close))
}

fn item<'a>(glyph: Icon, caption: &'a str, danger: bool, message: Msg) -> El<'a, Screen> {
    ui::menu_item(glyph, caption, danger, send(message))
}

fn panel<'a>(content: impl Into<El<'a, Screen>>, width: f32) -> El<'a, Screen> {
    ui::menu_panel(content, width)
}

fn swatch<'a>(color: iced::Color, hint: &'a str, message: Msg) -> El<'a, Screen> {
    let tile = container(Space::new()).width(22).height(22).style(theme::swatch(color));
    ui::hint_on(button(tile).padding(0).style(theme::btn_ghost).on_press(send(message)), hint)
}

impl Transactions {
    pub(super) fn load_presets(ctx: &Ctx) -> Vec<Preset> {
        tbs_core::config::load_json(&ctx.state_path(PRESETS))
    }

    fn save_presets(&self, ctx: &Ctx) {
        if let Err(error) = tbs_core::config::save_json(&ctx.state_path(PRESETS), &self.presets) {
            tracing::warn!("presets not saved: {error}");
        }
    }

    fn paint(&mut self, change: impl Fn(&mut Paint), ctx: &Ctx) {
        match self.popup {
            Popup::Header { col, .. } => {
                let Some(&column) = self.columns.get(col) else { return };
                let paint = self.table.column_paint.entry(column).or_default();
                change(paint);
                if paint.is_empty() {
                    self.table.column_paint.remove(&column);
                }
                self.refresh_columns();
            }
            Popup::Cell { .. } => {
                for (row, col) in self.selection.cells() {
                    let (Some(item), Some(&column)) = (self.row_at(row), self.columns.get(col)) else { continue };
                    let key = (item.tx.id, column);
                    let paint = self.table.cell_paint.entry(key).or_default();
                    change(paint);
                    if paint.is_empty() {
                        self.table.cell_paint.remove(&key);
                    }
                }
            }
            _ => return,
        }
        self.save(ctx);
    }

    fn apply_view(&mut self, view: TableView, ctx: &mut Ctx) {
        self.table = view;
        self.table.repair();
        self.search = self.table.filters.search.clone();
        self.forget_filter_draft(ctx);
        self.refresh_columns();
        self.refresh_rows();
    }

    fn export_task(&self, ctx: &Ctx, params: Vec<(&'static str, String)>, fallback: String, done: &'static str) -> Task<Screen> {
        let api = ctx.api.clone();
        Task::perform(
            async move {
                let (name, bytes) = api.download("/api/transactions/export", &params).await.map_err(text_error)?;
                save_as(name.unwrap_or(fallback), bytes).await
            },
            move |result| send(Msg::Exported(done, result)),
        )
    }

    fn export_selected(&self, ctx: &mut Ctx) -> Task<Screen> {
        let positions = self.selection.rows();
        let rows: Vec<&Row> = positions.iter().filter_map(|&p| self.row_at(p)).collect();
        if rows.is_empty() {
            return Task::none();
        }
        let total = self.matched_len();
        let sort = self.table.sort;
        let numbering = |column: Column, y: usize| match column {
            Column::RowNumber => Some((self.position(positions[y]) + 1).to_string()),
            Column::OperationNumber => Some(operation_number(&sort, total, self.position(positions[y])).to_string()),
            _ => None,
        };
        match selected_rows_xlsx(&rows, &self.columns, numbering) {
            Ok(bytes) => {
                let name = format!("transactions_selected_{}.xlsx", today());
                Task::perform(save_as(name, bytes), |result| send(Msg::Exported("Экспорт выбранных строк готов", result)))
            }
            Err(error) => {
                ctx.toast(Tone::Danger, format!("Не удалось выполнить экспорт: {error}"));
                Task::none()
            }
        }
    }

    pub(super) fn menu_update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Screen> {
        match message {
            Msg::Open(popup) => self.popup = if self.popup == popup { Popup::None } else { popup },
            Msg::Close => self.popup = Popup::None,
            Msg::Align(justify) => {
                self.paint(|p| p.align = Some(justify), ctx);
                self.popup = Popup::None;
            }
            Msg::Fill(fill) => {
                self.paint(|p| p.fill = fill, ctx);
                self.popup = Popup::None;
            }
            Msg::HideColumn => {
                if let Popup::Header { col, .. } = self.popup
                    && let Some(&column) = self.columns.get(col)
                    && column != LOCKED
                {
                    self.table.hidden.push(column);
                    self.selection = Selection::default();
                    self.refresh_columns();
                    self.save(ctx);
                }
                self.popup = Popup::None;
            }
            Msg::DeleteRow => {
                self.popup = Popup::None;
                let ids = self.selected_ids();
                self.ask_delete(ids);
            }
            Msg::ToggleColumn(column, visible) => {
                if column == LOCKED {
                    return Task::none();
                }
                self.table.hidden.retain(|c| *c != column);
                if !visible {
                    self.table.hidden.push(column);
                }
                self.selection = Selection::default();
                self.refresh_columns();
                self.save(ctx);
            }
            Msg::ShowAll => {
                self.table.hidden.clear();
                self.refresh_columns();
                self.save(ctx);
            }
            Msg::HideAll => {
                self.table.hidden = columns::ALL.iter().copied().filter(|c| *c != LOCKED).collect();
                self.selection = Selection::default();
                self.refresh_columns();
                self.save(ctx);
            }
            Msg::Density(density) => {
                self.table.density = density;
                self.save(ctx);
            }
            Msg::Pinned(count) => {
                self.table.pinned = count.min(MAX_PINNED);
                self.save(ctx);
            }
            Msg::Reset => {
                self.popup = Popup::None;
                self.apply_view(TableView::default(), ctx);
                self.save(ctx);
                ctx.toast(Tone::Success, "Фильтры и вид сброшены для текущего пользователя");
            }
            Msg::PresetName(value) => self.preset_name = value,
            Msg::PresetSave(default) => {
                let name = self.preset_name.trim().to_string();
                if name.is_empty() {
                    ctx.toast(Tone::Warning, "Введите название вида");
                    return Task::none();
                }
                store_preset(&mut self.presets, &name, self.table.clone(), default);
                self.preset_name.clear();
                self.save_presets(ctx);
            }
            Msg::PresetLoad(index) => {
                if let Some(preset) = self.presets.get(index) {
                    let view = preset.view.clone();
                    self.apply_view(view, ctx);
                    self.save(ctx);
                }
            }
            Msg::PresetDefault(index) => {
                let chosen = self.presets.get(index).is_some_and(|p| !p.default);
                self.presets.iter_mut().for_each(|p| p.default = false);
                if chosen {
                    self.presets[index].default = true;
                }
                self.save_presets(ctx);
            }
            Msg::PresetAskRename(index) => {
                if let Some(preset) = self.presets.get(index) {
                    self.preset_ask = Some(PresetAsk::Rename { index, name: preset.name.clone() });
                }
            }
            Msg::PresetAskDelete(index) => {
                if let Some(preset) = self.presets.get(index) {
                    self.preset_ask = Some(PresetAsk::Delete { index, title: format!("Удалить вид \"{}\"?", preset.name) });
                }
            }
            Msg::PresetAskText(value) => {
                if let Some(PresetAsk::Rename { name, .. }) = &mut self.preset_ask {
                    *name = value;
                }
            }
            Msg::PresetAskConfirm => {
                match self.preset_ask.take() {
                    Some(PresetAsk::Rename { index, name }) => {
                        let name = name.trim().to_string();
                        if let (false, Some(preset)) = (name.is_empty(), self.presets.get_mut(index)) {
                            preset.name = name;
                        }
                    }
                    Some(PresetAsk::Delete { index, .. }) if index < self.presets.len() => {
                        self.presets.remove(index);
                    }
                    _ => {}
                }
                self.save_presets(ctx);
            }
            Msg::PresetAskClose => self.preset_ask = None,
            Msg::ExportView => {
                self.popup = Popup::None;
                let params = export_params(&self.table.filters, &self.scope(), &self.table.sort, &self.columns, None);
                return self.export_task(ctx, params, format!("transactions_{}.xlsx", today()), "Экспорт текущего вида готов");
            }
            Msg::ExportAll => {
                self.popup = Popup::None;
                let params = export_params(&Filters::default(), &Scope::default(), &self.table.sort, &self.columns, None);
                return self.export_task(ctx, params, format!("transactions_all_{}.xlsx", today()), "Экспорт всех транзакций готов");
            }
            Msg::ExportFrom(value) => self.export_from = value,
            Msg::ExportTo(value) => self.export_to = value,
            Msg::ExportPeriod => match period(&self.export_from, &self.export_to) {
                Ok((from, to)) => {
                    self.popup = Popup::None;
                    let params = export_params(&self.table.filters, &self.scope(), &self.table.sort, &self.columns, Some((from, to)));
                    return self.export_task(ctx, params, format!("transactions_{from}_{to}.xlsx"), "Экспорт за период готов");
                }
                Err(reason) => ctx.toast(Tone::Danger, reason),
            },
            Msg::ExportSelected => return self.export_selected(ctx),
            Msg::Exported(done, result) => match result {
                Ok(Some(_)) => ctx.toast(Tone::Success, done),
                Ok(None) => {}
                Err(reason) => ctx.toast(Tone::Danger, reason),
            },
        }
        Task::none()
    }

    fn paint_menu(&self, column: Option<Column>, cell: bool) -> El<'_, Screen> {
        let aligns = row![
            ui::icon_btn(Icon::AlignLeft, "По левому краю", Some(send(Msg::Align(Justify::Left)))),
            ui::icon_btn(Icon::AlignCenter, "По центру", Some(send(Msg::Align(Justify::Center)))),
            ui::icon_btn(Icon::AlignRight, "По правому краю", Some(send(Msg::Align(Justify::Right)))),
        ]
        .spacing(space::XS);

        let mut fills = row![ui::icon_btn(Icon::Close, "Нет", Some(send(Msg::Fill(None))))].spacing(space::XS).align_y(Alignment::Center);
        for (index, (name, color)) in FILLS.iter().enumerate() {
            fills = fills.push(swatch(*color, name, Msg::Fill(Some(index as u8))));
        }

        let mut body = column![ui::faint("Выравнивание"), aligns, ui::faint("Заливка"), fills].spacing(space::XS);
        if column.is_some_and(|c| c != LOCKED) && !cell {
            body = body.push(ui::divider()).push(item(Icon::EyeOff, "Скрыть колонку", false, Msg::HideColumn));
        }
        if cell {
            body = body.push(ui::divider()).push(item(Icon::Trash, "Удалить строку", true, Msg::DeleteRow));
        }
        panel(body, CELL_WIDTH)
    }

    fn view_menu(&self, window: Size) -> El<'_, Screen> {
        let mut list = column![].spacing(space::XXS);
        for &column in &self.table.order {
            let visible = !self.table.hidden.contains(&column);
            let check = if column == LOCKED {
                iced::widget::checkbox(true).label(column.title()).size(16).text_size(size::BASE).style(theme::check)
            } else {
                ui::check(column.title(), visible, move |on| send(Msg::ToggleColumn(column, on)))
            };
            list = list.push(check);
        }
        let columns_block = column![
            row![ui::btn("Показать все", Kind::Ghost, Some(send(Msg::ShowAll))), ui::btn("Скрыть все", Kind::Ghost, Some(send(Msg::HideAll)))].spacing(space::XS),
            list,
        ]
        .spacing(space::SM);

        let mut densities = column![].spacing(space::XXS);
        for density in Density::ALL {
            densities = densities.push(ui::chip(density.label(), self.table.density == density, send(Msg::Density(density))));
        }
        let pins: Vec<(usize, &str)> = vec![(0, "0"), (1, "1"), (2, "2"), (3, "3")];

        let mut saved = column![].spacing(space::XS);
        if self.presets.is_empty() {
            saved = saved.push(ui::muted("Пока нет сохранённых видов."));
        }
        for (index, preset) in self.presets.iter().enumerate() {
            let name = if preset.default { format!("{} · по умолчанию", preset.name) } else { preset.name.clone() };
            saved = saved.push(
                row![
                    container(ui::label(name)).width(Fill),
                    ui::icon_btn(Icon::Download, "Загрузить", Some(send(Msg::PresetLoad(index)))),
                    ui::icon_btn(Icon::Edit, "Имя", Some(send(Msg::PresetAskRename(index)))),
                    ui::icon_btn(Icon::Bookmark, "По умолчанию", Some(send(Msg::PresetDefault(index)))),
                    ui::icon_btn(Icon::Trash, "Удалить", Some(send(Msg::PresetAskDelete(index)))),
                ]
                .spacing(space::XXS)
                .align_y(Alignment::Center),
            );
        }
        let presets = column![
            ui::input("Название вида", &self.preset_name).on_input(|v| send(Msg::PresetName(v))).on_submit(send(Msg::PresetSave(false))),
            row![ui::btn("Сохранить", Kind::Secondary, Some(send(Msg::PresetSave(false)))), ui::btn("Сохранить и по умолчанию", Kind::Ghost, Some(send(Msg::PresetSave(true))))].spacing(space::XS),
            saved,
        ]
        .spacing(space::SM);

        let body = column![
            ui::section("Столбцы", columns_block),
            ui::section("Плотность", densities),
            ui::section("Закрепить столбцы", ui::segments(&pins, self.table.pinned, |n| send(Msg::Pinned(n)))),
            ui::section("Сохранённые виды", presets),
            ui::btn_icon(Icon::Refresh, "Сбросить вид", Kind::Ghost, Some(send(Msg::Reset))),
        ]
        .spacing(space::LG);
        let height = (window.height - UNDER_TOOLBAR - ui::EDGE * 2.0).max(200.0);
        container(ui::scroll(container(body).padding(space::MD)).height(height)).width(VIEW_WIDTH).style(theme::popover).into()
    }

    fn export_menu(&self) -> El<'_, Screen> {
        let body = column![
            item(Icon::Table, "Экспорт текущего вида", false, Msg::ExportView),
            item(Icon::Sheet, "Экспорт всех транзакций", false, Msg::ExportAll),
            ui::divider(),
            ui::faint("Экспорт за период"),
            row![
                ui::input("ДД.ММ.ГГГГ", &self.export_from).on_input(|v| send(Msg::ExportFrom(v))),
                ui::input("ДД.ММ.ГГГГ", &self.export_to).on_input(|v| send(Msg::ExportTo(v))).on_submit(send(Msg::ExportPeriod)),
            ]
            .spacing(space::XS),
            ui::btn_icon(Icon::Download, "Скачать", Kind::Secondary, Some(send(Msg::ExportPeriod))),
        ]
        .spacing(space::XS);
        panel(body, EXPORT_WIDTH)
    }

    fn help_menu(&self) -> El<'_, Screen> {
        let mut body = column![ui::strong("Поиск")].spacing(space::XS);
        for (query, meaning) in SEARCH_HELP {
            body = body.push(row![container(text(query).size(size::SM).font(theme::FONT_MONO)).width(160), ui::muted(meaning)].spacing(space::SM));
        }
        panel(container(body).padding(space::XS), HELP_WIDTH)
    }

    pub(super) fn popup_view(&self, ctx: &Ctx) -> Option<El<'_, Screen>> {
        let right = ctx.window.width - ui::PAGE_PADDING.right;
        // In a narrow page the tools take a line more, and the panels of the toolbar begin under it.
        let under = if self.tools_on_one_line(ctx) { UNDER_TOOLBAR } else { UNDER_TOOLBAR + theme::control::MD + space::SM };
        let at = |width: f32| Point::new(right - width, under);
        Some(match self.popup {
            Popup::None => return None,
            Popup::Cell { at, .. } => floating(self.paint_menu(None, true), at, Size::new(CELL_WIDTH, 170.0), ctx.window),
            Popup::Header { col, at } => floating(self.paint_menu(self.columns.get(col).copied(), false), at, Size::new(CELL_WIDTH, 170.0), ctx.window),
            Popup::View => floating(self.view_menu(ctx.window), at(VIEW_WIDTH), Size::new(VIEW_WIDTH, 200.0), ctx.window),
            Popup::Export => floating(self.export_menu(), at(EXPORT_WIDTH), Size::new(EXPORT_WIDTH, 220.0), ctx.window),
            Popup::Help => floating(self.help_menu(), Point::new(ctx.rail + ui::PAGE_PADDING.left, under), Size::new(HELP_WIDTH, 190.0), ctx.window),
        })
    }

    pub(super) fn preset_ask_view(&self) -> Option<El<'_, Screen>> {
        Some(match self.preset_ask.as_ref()? {
            PresetAsk::Delete { title, .. } => {
                ui::confirm(Space::new(), title, "Вид будет удалён без возможности восстановления.", "Удалить", send(Msg::PresetAskConfirm), send(Msg::PresetAskClose))
            }
            PresetAsk::Rename { name, .. } => {
                let body = column![
                    ui::dialog_title("Переименовать вид", Some(send(Msg::PresetAskClose))),
                    ui::field("Новое имя", ui::input("Новое имя", name).on_input(|v| send(Msg::PresetAskText(v))).on_submit(send(Msg::PresetAskConfirm))),
                    row![ui::fill(), ui::btn("Отмена", Kind::Ghost, Some(send(Msg::PresetAskClose))), ui::btn("Сохранить", Kind::Primary, Some(send(Msg::PresetAskConfirm)))].spacing(space::SM),
                ]
                .spacing(space::MD);
                ui::modal(Space::new(), body, 380, Some(send(Msg::PresetAskClose)))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_period_needs_both_ends_in_order() {
        assert_eq!(period("", "01.02.2026"), Err("Укажите период (дата начала и дата конца)."));
        assert_eq!(period("02.02.2026", "01.02.2026"), Err("Период указан некорректно."));
        assert_eq!(period("31.02.2026", "01.03.2026"), Err("Период указан некорректно."));
        assert!(period("01.02.2026", "01.02.2026").is_ok());
    }

    #[test]
    fn saving_a_name_again_replaces_the_view() {
        let mut presets = Vec::new();
        store_preset(&mut presets, "Отчёт", TableView::default(), false);
        let wide = TableView { pinned: 1, ..Default::default() };
        store_preset(&mut presets, "Отчёт", wide, false);
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].view.pinned, 1);
    }

    #[test]
    fn only_one_preset_stays_default() {
        let mut presets = Vec::new();
        store_preset(&mut presets, "А", TableView::default(), true);
        store_preset(&mut presets, "Б", TableView::default(), true);
        assert_eq!(presets.iter().filter(|p| p.default).map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Б"]);
    }

    #[test]
    fn panels_stay_inside_the_window() {
        let window = Size::new(1000.0, 700.0);
        let panel = Size::new(CELL_WIDTH, 170.0);
        let at = ui::clamped(Point::new(990.0, 690.0), panel, window);
        assert!(at.x + panel.width <= window.width && at.y + panel.height <= window.height);
        assert_eq!(ui::clamped(Point::new(-5.0, 0.0), panel, window).x, ui::EDGE);
    }
}
