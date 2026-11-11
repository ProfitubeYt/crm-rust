//! Small floating panels: the cell and header menus, export and search help.
//! The look of the table has a sheet of its own in `view`.

use chrono::{Local, NaiveDate};
use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Point, Size, Task};
use tbs_core::columns::Column;
use tbs_core::export::selected_rows_xlsx;
use tbs_core::query::{Filters, Row, Scope, export_params};

use super::table::{FILLS, Justify, LOCKED, Paint, operation_number};
use super::{Msg as Screen, Transactions};
use crate::ctx::{Ctx, text_error};
use crate::files::save_as;
use crate::grid::Selection;
use crate::theme::{self, Tone, size, space};
use crate::ui::{self, El, Icon, Kind};

const CELL_WIDTH: f32 = 220.0;
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
    ExportView,
    ExportAll,
    ExportFrom(String),
    ExportTo(String),
    ExportPeriod,
    ExportSelected,
    Exported(&'static str, Result<Option<String>, String>),
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
            Popup::Export => floating(self.export_menu(), at(EXPORT_WIDTH), Size::new(EXPORT_WIDTH, 220.0), ctx.window),
            Popup::Help => floating(self.help_menu(), Point::new(ctx.rail + ui::PAGE_PADDING.left, under), Size::new(HELP_WIDTH, 190.0), ctx.window),
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
    fn panels_stay_inside_the_window() {
        let window = Size::new(1000.0, 700.0);
        let panel = Size::new(CELL_WIDTH, 170.0);
        let at = ui::clamped(Point::new(990.0, 690.0), panel, window);
        assert!(at.x + panel.width <= window.width && at.y + panel.height <= window.height);
        assert_eq!(ui::clamped(Point::new(-5.0, 0.0), panel, window).x, ui::EDGE);
    }
}
