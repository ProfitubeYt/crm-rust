//! The table. This widget exists because the React table froze on scroll.
//!
//! A wheel tick here changes one number in the widget's own state and asks for a
//! redraw. No message is published and `view()` does not run. Drawing touches
//! only the cells on screen, so the cost does not grow with the row count.

pub mod geometry;

use std::borrow::Cow;
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::svg::{self as adv_svg, Renderer as _};
use iced::advanced::text::{self, LineHeight, Paragraph as _, Renderer as _, Shaping, Text, Wrapping};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Renderer as _, Shell};
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::mouse::{self, Cursor};
use iced::{Background, Border, Color, Element, Event, Font, Length, Pixels, Point, Radians, Rectangle, Size, Theme, Vector, alignment};

pub use geometry::Selection;
use geometry::{BAR, Columns, MAX_COLUMN, MIN_COLUMN};

use crate::theme::{self, Palette, Tone};
use crate::ui::{self, Icon};

const WHEEL_LINE: f32 = 64.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const DRAG_START: f32 = 6.0;
const EDGE_SCROLL: f32 = 18.0;
const CELL_PAD: f32 = 8.0;
/// The line between two columns. iced snaps a quad to the pixel grid, so the
/// line stays sharp wherever the scroll puts it.
const SEPARATOR: f32 = 1.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// How one cell differs from its column.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CellStyle {
    pub background: Option<Color>,
    pub align: Option<Align>,
    pub color: Option<Color>,
    pub medium: bool,
    /// Text colour taken from the theme, so one style works in light and dark.
    pub tone: Option<Tone>,
    /// Draws the value as a small rounded label in the tone's soft fill.
    pub pill: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowTone {
    #[default]
    Plain,
    Warning,
    Highlight,
}

pub trait GridData {
    fn row_count(&self) -> usize;
    fn cell(&self, row: usize, col: usize) -> Cow<'_, str>;
    fn cell_style(&self, _row: usize, _col: usize) -> CellStyle {
        CellStyle::default()
    }
    fn row_tone(&self, _row: usize) -> RowTone {
        RowTone::Plain
    }
}

#[derive(Clone, Debug)]
pub struct GridColumn {
    pub title: String,
    pub width: f32,
    pub align: Align,
    pub mono: bool,
    pub medium: bool,
    pub sortable: bool,
    /// `Some(true)` means sorted descending.
    pub sorted: Option<bool>,
    pub background: Option<Color>,
}

impl GridColumn {
    pub fn new(title: impl Into<String>, width: f32) -> Self {
        Self { title: title.into(), width, align: Align::Left, mono: false, medium: false, sortable: false, sorted: None, background: None }
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    pub fn medium(mut self) -> Self {
        self.medium = true;
        self
    }

    pub fn sortable(mut self, sorted: Option<bool>) -> Self {
        self.sortable = true;
        self.sorted = sorted;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridKey {
    Delete,
    Copy,
    Paste,
    Undo,
    Redo,
    Escape,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GridEvent {
    Select(Selection),
    /// A plain click that ended where it started.
    Click { row: usize, col: usize, at: Point },
    /// Double click, Enter or F2. `rect` is the cell in window coordinates.
    Activate { row: usize, col: usize, rect: Rectangle },
    /// Typing on a selected cell starts an edit with that text.
    TypeChar { row: usize, col: usize, rect: Rectangle, text: String },
    Context { row: usize, col: usize, at: Point },
    HeaderContext { col: usize, at: Point },
    Sort(usize),
    Resize { col: usize, width: f32 },
    Move { from: usize, to: usize },
    Key(GridKey),
    /// A press in the grid while `locked` is set; the app commits its editor.
    Dismiss,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Drag {
    #[default]
    None,
    Select,
    Resize { col: usize, origin: f32, start: f32 },
    Header { col: usize, origin: f32, moved: bool },
    ThumbV { grab: f32 },
    ThumbH { grab: f32 },
}

#[derive(Debug, Default)]
struct State {
    scroll: Vector,
    hover: Option<(usize, usize)>,
    hover_header: Option<usize>,
    drag: Drag,
    focused: bool,
    modifiers: Modifiers,
    last_click: Option<(Instant, usize, usize)>,
    /// Selection while the mouse is held; published on release.
    live: Option<Selection>,
    resize_preview: Option<(usize, f32)>,
    /// Column widened for one clipped cell until the pointer moves on.
    expanded: Option<(usize, f32)>,
    drop_slot: Option<usize>,
    reveal_token: u64,
    moved_since_press: bool,
    press_cell: Option<(usize, usize)>,
    bench_down: bool,
}

/// Regions of the widget, in window coordinates.
#[derive(Clone, Copy, Debug)]
struct Frame {
    header: Rectangle,
    body: Rectangle,
    bar_v: Rectangle,
    bar_h: Rectangle,
}

/// A screen shows one grid at a time, so they can all answer to one name.
pub const ID: &str = "grid";

pub struct Grid<'a, D, Message> {
    id: iced::widget::Id,
    data: D,
    columns: &'a [GridColumn],
    selection: &'a Selection,
    pinned: usize,
    row_height: f32,
    header_height: f32,
    font_size: f32,
    locked: bool,
    reveal: Option<(u64, usize)>,
    on_event: Box<dyn Fn(GridEvent) -> Message + 'a>,
}

pub fn grid<'a, D: GridData, Message>(
    data: D,
    columns: &'a [GridColumn],
    selection: &'a Selection,
    on_event: impl Fn(GridEvent) -> Message + 'a,
) -> Grid<'a, D, Message> {
    Grid {
        id: iced::widget::Id::new(ID),
        data,
        columns,
        selection,
        pinned: 0,
        row_height: 28.0,
        header_height: 30.0,
        font_size: 12.0,
        locked: false,
        reveal: None,
        on_event: Box::new(on_event),
    }
}

impl<'a, D: GridData, Message> Grid<'a, D, Message> {
    pub fn pinned(mut self, count: usize) -> Self {
        self.pinned = count;
        self
    }

    pub fn row_height(mut self, height: f32) -> Self {
        self.row_height = height;
        self
    }

    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    /// Scrolls `row` into view once per change of `token`.
    pub fn reveal(mut self, reveal: Option<(u64, usize)>) -> Self {
        self.reveal = reveal;
        self
    }

    fn geometry(&self, state: &State) -> Columns {
        let mut widths: Vec<f32> = self.columns.iter().map(|c| c.width).collect();
        if let Some((col, width)) = state.expanded {
            if let Some(slot) = widths.get_mut(col) {
                *slot = width;
            }
        }
        if let Some((col, width)) = state.resize_preview {
            if let Some(slot) = widths.get_mut(col) {
                *slot = width;
            }
        }
        Columns::new(widths, self.pinned)
    }

    fn frame(&self, bounds: Rectangle) -> Frame {
        let body_h = (bounds.height - self.header_height - BAR).max(0.0);
        let body_w = (bounds.width - BAR).max(0.0);
        Frame {
            header: Rectangle { x: bounds.x, y: bounds.y, width: body_w, height: self.header_height },
            body: Rectangle { x: bounds.x, y: bounds.y + self.header_height, width: body_w, height: body_h },
            bar_v: Rectangle { x: bounds.x + body_w, y: bounds.y + self.header_height, width: BAR, height: body_h },
            bar_h: Rectangle { x: bounds.x, y: bounds.y + self.header_height + body_h, width: body_w, height: BAR },
        }
    }

    fn content_height(&self) -> f32 {
        self.data.row_count() as f32 * self.row_height
    }

    fn clamp_scroll(&self, state: &mut State, columns: &Columns, frame: &Frame) {
        state.scroll.x = state.scroll.x.clamp(0.0, columns.max_scroll(frame.body.width));
        state.scroll.y = state.scroll.y.clamp(0.0, geometry::max_scroll_y(self.data.row_count(), self.row_height, frame.body.height));
    }

    fn scroll_track(&self, columns: &Columns, frame: &Frame) -> f32 {
        (frame.body.width - columns.pinned_width).max(0.0)
    }

    fn cell_at(&self, state: &State, columns: &Columns, frame: &Frame, at: Point) -> Option<(usize, usize)> {
        if !frame.body.contains(at) {
            return None;
        }
        let row = ((at.y - frame.body.y + state.scroll.y) / self.row_height).floor() as usize;
        let col = columns.at(at.x - frame.body.x, state.scroll.x)?;
        (row < self.data.row_count()).then_some((row, col))
    }

    /// The nearest cell, for drags that run past the edge of the table.
    fn cell_near(&self, state: &State, columns: &Columns, frame: &Frame, at: Point) -> Option<(usize, usize)> {
        let rows = self.data.row_count();
        if rows == 0 || columns.len() == 0 {
            return None;
        }
        let y = at.y.clamp(frame.body.y, frame.body.y + frame.body.height - 1.0);
        let x = at.x.clamp(frame.body.x, frame.body.x + frame.body.width - 1.0);
        let row = (((y - frame.body.y + state.scroll.y) / self.row_height).floor() as usize).min(rows - 1);
        let col = columns.at(x - frame.body.x, state.scroll.x).unwrap_or(columns.len() - 1);
        Some((row, col))
    }

    fn cell_rect(&self, state: &State, columns: &Columns, frame: &Frame, row: usize, col: usize) -> Rectangle {
        Rectangle {
            x: frame.body.x + columns.screen_x(col, state.scroll.x),
            y: frame.body.y + row as f32 * self.row_height - state.scroll.y,
            width: columns.widths[col],
            height: self.row_height,
        }
    }

    fn reveal_cell(&self, state: &mut State, columns: &Columns, frame: &Frame, row: usize, col: usize) {
        state.scroll.y = geometry::reveal(state.scroll.y, row as f32 * self.row_height, self.row_height, frame.body.height);
        if !columns.is_pinned(col) {
            let viewport = (frame.body.width - columns.pinned_width).max(0.0);
            let start = columns.starts[col] - columns.pinned_width;
            state.scroll.x = geometry::reveal(state.scroll.x, start, columns.widths[col], viewport);
        }
        self.clamp_scroll(state, columns, frame);
    }

    fn current<'s>(&'s self, state: &'s State) -> &'s Selection {
        state.live.as_ref().unwrap_or(self.selection)
    }

    fn font(&self, column: &GridColumn, medium: bool) -> Font {
        if column.mono {
            theme::FONT_MONO
        } else if column.medium || medium {
            theme::FONT_MEDIUM
        } else {
            theme::FONT
        }
    }

    /// Text width of a clipped cell, or `None` when it already fits.
    fn overflow_width(&self, row: usize, col: usize, width: f32) -> Option<f32> {
        let column = self.columns.get(col)?;
        let value = self.data.cell(row, col);
        if value.is_empty() {
            return None;
        }
        let paragraph = <iced::Renderer as text::Renderer>::Paragraph::with_text(Text {
            content: value.as_ref(),
            bounds: Size::new(f32::INFINITY, self.row_height),
            size: Pixels(self.font_size),
            line_height: LineHeight::default(),
            font: self.font(column, false),
            align_x: text::Alignment::Left,
            align_y: alignment::Vertical::Center,
            shaping: Shaping::Advanced,
            wrapping: Wrapping::None,
        });
        let needed = paragraph.min_width().ceil() + CELL_PAD * 2.0 + 4.0;
        (needed > width).then(|| needed.min(MAX_COLUMN))
    }

    fn press_body(
        &self,
        state: &mut State,
        columns: &Columns,
        frame: &Frame,
        row: usize,
        col: usize,
        shell: &mut Shell<'_, Message>,
    ) {
        let now = Instant::now();
        let double = state
            .last_click
            .is_some_and(|(at, r, c)| r == row && c == col && now.duration_since(at) < DOUBLE_CLICK);
        state.last_click = Some((now, row, col));
        state.press_cell = Some((row, col));
        state.moved_since_press = false;

        if double {
            state.last_click = None;
            state.expanded = None;
            let rect = self.cell_rect(state, columns, frame, row, col);
            shell.publish((self.on_event)(GridEvent::Activate { row, col, rect }));
            return;
        }
        let mut next = self.selection.clone();
        if state.modifiers.control() {
            next.toggle(row, col);
            shell.publish((self.on_event)(GridEvent::Select(next)));
            return;
        }
        if state.modifiers.shift() && next.anchor.is_some() {
            next.extend_to((row, col));
        } else {
            next = Selection::single(row, col);
        }
        state.live = Some(next);
        state.drag = Drag::Select;
    }

    fn handle_key(
        &self,
        state: &mut State,
        columns: &Columns,
        frame: &Frame,
        key: &Key,
        modifiers: Modifiers,
        typed: Option<&str>,
        shell: &mut Shell<'_, Message>,
    ) -> bool {
        let rows = self.data.row_count();
        let cols = columns.len();
        let ctrl = modifiers.control();
        let publish = |shell: &mut Shell<'_, Message>, event: GridEvent| shell.publish((self.on_event)(event));

        if ctrl {
            if let Key::Character(c) = key {
                let event = match c.to_lowercase().as_str() {
                    "c" | "с" => Some(GridEvent::Key(GridKey::Copy)),
                    "v" | "м" => Some(GridEvent::Key(GridKey::Paste)),
                    "z" | "я" => Some(GridEvent::Key(GridKey::Undo)),
                    "y" | "н" => Some(GridEvent::Key(GridKey::Redo)),
                    "a" | "ф" if rows > 0 && cols > 0 => {
                        let mut all = Selection::single(0, 0);
                        all.extend_to((rows - 1, cols - 1));
                        Some(GridEvent::Select(all))
                    }
                    _ => None,
                };
                if let Some(event) = event {
                    publish(shell, event);
                    return true;
                }
            }
        }
        if rows == 0 || cols == 0 {
            return false;
        }
        let cursor = self.selection.cursor;
        let page = ((frame.body.height / self.row_height).floor() as isize - 1).max(1);
        let movement: Option<(isize, isize)> = match key {
            Key::Named(Named::ArrowUp) => Some((-1, 0)),
            Key::Named(Named::ArrowDown) => Some((1, 0)),
            Key::Named(Named::ArrowLeft) => Some((0, -1)),
            Key::Named(Named::ArrowRight) => Some((0, 1)),
            Key::Named(Named::Tab) if modifiers.shift() => Some((0, -1)),
            Key::Named(Named::Tab) => Some((0, 1)),
            Key::Named(Named::PageUp) => Some((-page, 0)),
            Key::Named(Named::PageDown) => Some((page, 0)),
            _ => None,
        };
        if let Some((dr, dc)) = movement {
            let (row, col) = cursor.unwrap_or((0, 0));
            let target = (
                (row as isize + dr).clamp(0, rows as isize - 1) as usize,
                (col as isize + dc).clamp(0, cols as isize - 1) as usize,
            );
            self.move_to(state, columns, frame, target, modifiers.shift(), shell);
            return true;
        }
        match key {
            Key::Named(Named::Home) => {
                let (row, _) = cursor.unwrap_or((0, 0));
                let target = if ctrl { (0, 0) } else { (row, 0) };
                self.move_to(state, columns, frame, target, modifiers.shift(), shell);
                true
            }
            Key::Named(Named::End) => {
                let (row, _) = cursor.unwrap_or((0, 0));
                let target = if ctrl { (rows - 1, cols - 1) } else { (row, cols - 1) };
                self.move_to(state, columns, frame, target, modifiers.shift(), shell);
                true
            }
            Key::Named(Named::Delete | Named::Backspace) => {
                publish(shell, GridEvent::Key(GridKey::Delete));
                true
            }
            Key::Named(Named::Escape) => {
                publish(shell, GridEvent::Key(GridKey::Escape));
                true
            }
            Key::Named(Named::Enter | Named::F2) => {
                if let Some((row, col)) = cursor {
                    self.reveal_cell(state, columns, frame, row, col);
                    let rect = self.cell_rect(state, columns, frame, row, col);
                    publish(shell, GridEvent::Activate { row, col, rect });
                }
                true
            }
            _ => {
                let printable = typed.filter(|t| !ctrl && !modifiers.alt() && t.chars().all(|c| !c.is_control()) && !t.is_empty());
                if let (Some(value), Some((row, col))) = (printable, cursor) {
                    self.reveal_cell(state, columns, frame, row, col);
                    let rect = self.cell_rect(state, columns, frame, row, col);
                    publish(shell, GridEvent::TypeChar { row, col, rect, text: value.to_string() });
                    return true;
                }
                false
            }
        }
    }

    fn move_to(
        &self,
        state: &mut State,
        columns: &Columns,
        frame: &Frame,
        target: (usize, usize),
        extend: bool,
        shell: &mut Shell<'_, Message>,
    ) {
        let mut next = self.selection.clone();
        if extend {
            next.extend_to(target);
        } else {
            next = Selection::single(target.0, target.1);
        }
        self.reveal_cell(state, columns, frame, target.0, target.1);
        shell.publish((self.on_event)(GridEvent::Select(next)));
    }

    fn draw_body(
        &self,
        renderer: &mut iced::Renderer,
        p: &Palette,
        state: &State,
        columns: &Columns,
        range: std::ops::Range<usize>,
        clip: Rectangle,
        body: Rectangle,
        rows: std::ops::Range<usize>,
    ) {
        let selection = self.current(state);
        let scroll = state.scroll;
        for row in rows.clone() {
            let y = body.y + row as f32 * self.row_height - scroll.y;
            let line = Rectangle { x: clip.x, y, width: clip.width, height: self.row_height };
            let tone = self.data.row_tone(row);
            let base = match tone {
                RowTone::Warning => Some(p.warning_soft),
                RowTone::Highlight => Some(p.accent_soft),
                RowTone::Plain => None,
            };
            let hovered = state.hover.is_some_and(|(r, _)| r == row);
            if let Some(fill) = if hovered { Some(p.row_hover) } else { base } {
                quad(renderer, line, fill);
            }
            quad(renderer, Rectangle { y: y + self.row_height - 1.0, height: 1.0, ..line }, p.border);

            for col in range.clone() {
                let column = &self.columns[col];
                let x = body.x + columns.screen_x(col, scroll.x);
                let width = columns.widths[col];
                let cell = Rectangle { x, y, width, height: self.row_height };
                let style = self.data.cell_style(row, col);
                if let Some(fill) = style.background.or(column.background) {
                    quad(renderer, cell, fill);
                }
                if selection.contains(row, col) {
                    quad(renderer, cell, p.selection);
                }
                let value = self.data.cell(row, col);
                if value.is_empty() {
                    continue;
                }
                let align = style.align.unwrap_or(column.align);
                if style.pill {
                    self.draw_pill(renderer, p, &value, style.tone.unwrap_or_default(), align, cell, clip);
                    continue;
                }
                let (anchor, align_x) = match align {
                    Align::Left => (x + CELL_PAD, text::Alignment::Left),
                    Align::Center => (x + width / 2.0, text::Alignment::Center),
                    Align::Right => (x + width - CELL_PAD, text::Alignment::Right),
                };
                let clip_cell = Rectangle { x: x + 2.0, width: (width - 4.0).max(0.0), ..cell };
                let Some(visible) = clip_cell.intersection(&clip) else { continue };
                renderer.fill_text(
                    Text {
                        content: value.into_owned(),
                        bounds: Size::new(f32::INFINITY, self.row_height),
                        size: Pixels(self.font_size),
                        line_height: LineHeight::default(),
                        font: self.font(column, style.medium),
                        align_x,
                        align_y: alignment::Vertical::Center,
                        shaping: Shaping::Advanced,
                        wrapping: Wrapping::None,
                    },
                    Point::new(anchor, y + self.row_height / 2.0),
                    style.color.or(style.tone.map(|tone| p.tone(tone).0)).unwrap_or(p.text),
                    visible,
                );
            }
        }

        // The lines between columns go over every fill, so a painted, hovered or
        // selected cell keeps its edge, and under the frame of the cursor. They
        // end with the last row.
        let foot = (body.y + self.content_height() - scroll.y).min(clip.y + clip.height);
        if foot > clip.y {
            for col in range.clone() {
                quad(renderer, separator(columns, col, scroll.x, body.x, clip.y, foot - clip.y), p.border_strong);
            }
        }
        if let Some((row, col)) = selection.cursor
            && rows.contains(&row)
            && range.contains(&col)
        {
            let cell = Rectangle {
                x: body.x + columns.screen_x(col, scroll.x),
                y: body.y + row as f32 * self.row_height - scroll.y,
                width: columns.widths[col],
                height: self.row_height,
            };
            renderer.fill_quad(
                Quad { bounds: cell, border: Border { color: p.selection_edge, width: 1.5, radius: 0.0.into() }, ..Quad::default() },
                Background::Color(Color::TRANSPARENT),
            );
        }
    }

    /// A status label: measured text in a rounded soft fill, placed like the column's text.
    #[allow(clippy::too_many_arguments)]
    fn draw_pill(
        &self,
        renderer: &mut iced::Renderer,
        p: &Palette,
        value: &str,
        tone: Tone,
        align: Align,
        cell: Rectangle,
        clip: Rectangle,
    ) {
        let size = (self.font_size - 1.0).max(10.0);
        let paragraph = <iced::Renderer as text::Renderer>::Paragraph::with_text(Text {
            content: value,
            bounds: Size::new(f32::INFINITY, cell.height),
            size: Pixels(size),
            line_height: LineHeight::default(),
            font: theme::FONT_MEDIUM,
            align_x: text::Alignment::Left,
            align_y: alignment::Vertical::Center,
            shaping: Shaping::Advanced,
            wrapping: Wrapping::None,
        });
        let height = (size + 9.0).min(cell.height - 4.0).max(0.0);
        let width = (paragraph.min_width().ceil() + 16.0).min((cell.width - CELL_PAD * 2.0).max(0.0));
        let x = match align {
            Align::Left => cell.x + CELL_PAD,
            Align::Center => cell.x + (cell.width - width) / 2.0,
            Align::Right => cell.x + cell.width - CELL_PAD - width,
        };
        let pill = Rectangle { x, y: cell.y + (cell.height - height) / 2.0, width, height };
        let Some(visible) = pill.intersection(&clip) else { return };
        let (fg, bg) = p.tone(tone);
        // Same layer as the text: a layer of its own would draw over it.
        // The body layer already clips to the grid, so the quad needs no clip.
        renderer.fill_quad(
            Quad { bounds: pill, border: Border { radius: 6.0.into(), ..Border::default() }, ..Quad::default() },
            Background::Color(bg),
        );
        renderer.fill_text(
            Text {
                content: value.to_owned(),
                bounds: Size::new(f32::INFINITY, height),
                size: Pixels(size),
                line_height: LineHeight::default(),
                font: theme::FONT_MEDIUM,
                align_x: text::Alignment::Center,
                align_y: alignment::Vertical::Center,
                shaping: Shaping::Advanced,
                wrapping: Wrapping::None,
            },
            Point::new(pill.x + pill.width / 2.0, pill.y + pill.height / 2.0),
            fg,
            visible,
        );
    }

    fn draw_header(
        &self,
        renderer: &mut iced::Renderer,
        p: &Palette,
        state: &State,
        columns: &Columns,
        range: std::ops::Range<usize>,
        clip: Rectangle,
        header: Rectangle,
    ) {
        quad(renderer, clip, p.surface);
        quad(renderer, Rectangle { y: clip.y + clip.height - 1.0, height: 1.0, ..clip }, p.border);
        for col in range {
            let column = &self.columns[col];
            let x = header.x + columns.screen_x(col, state.scroll.x);
            let width = columns.widths[col];
            let cell = Rectangle { x, y: header.y, width, height: header.height };
            if state.hover_header == Some(col) && column.sortable {
                quad(renderer, cell, p.hover);
            }
            // The same line as between the cells below, so the two meet.
            quad(renderer, separator(columns, col, state.scroll.x, header.x, header.y, header.height), p.border_strong);
            let arrow_room = if column.sorted.is_some() { 14.0 } else { 0.0 };
            let (anchor, align_x) = match column.align {
                Align::Left => (x + CELL_PAD, text::Alignment::Left),
                Align::Center => (x + (width - arrow_room) / 2.0, text::Alignment::Center),
                Align::Right => (x + width - CELL_PAD - arrow_room, text::Alignment::Right),
            };
            let clip_cell = Rectangle { x: x + 2.0, width: (width - 4.0 - arrow_room).max(0.0), ..cell };
            if let Some(visible) = clip_cell.intersection(&clip) {
                renderer.fill_text(
                    Text {
                        content: column.title.clone(),
                        bounds: Size::new(f32::INFINITY, header.height),
                        size: Pixels((self.font_size - 1.0).max(10.0)),
                        line_height: LineHeight::default(),
                        font: theme::FONT_MEDIUM,
                        align_x,
                        align_y: alignment::Vertical::Center,
                        shaping: Shaping::Advanced,
                        wrapping: Wrapping::None,
                    },
                    Point::new(anchor, header.y + header.height / 2.0),
                    p.text3,
                    visible,
                );
            }
            if let Some(descending) = column.sorted {
                let glyph = if descending { Icon::ChevronDown } else { Icon::ChevronUp };
                let at = Rectangle {
                    x: x + width - 16.0,
                    y: header.y + (header.height - 12.0) / 2.0,
                    width: 12.0,
                    height: 12.0,
                };
                if let Some(visible) = at.intersection(&clip) {
                    renderer.draw_svg(
                        adv_svg::Svg { handle: ui::icon_handle(glyph), color: Some(p.accent), rotation: Radians(0.0), opacity: 1.0 },
                        at,
                        visible,
                    );
                }
            }
        }
    }

    fn drop_target(&self, state: &State, columns: &Columns, frame: &Frame, x: f32) -> Option<usize> {
        let at = columns.at(x - frame.header.x, state.scroll.x)?;
        Some(at)
    }
}

fn quad(renderer: &mut iced::Renderer, bounds: Rectangle, color: Color) {
    renderer.fill_quad(Quad { bounds, ..Quad::default() }, Background::Color(color));
}

/// The line at a column's right edge, `height` tall from `top`. The header and
/// the rows both ask here, so their lines stand at the same place.
fn separator(columns: &Columns, col: usize, scroll_x: f32, origin_x: f32, top: f32, height: f32) -> Rectangle {
    Rectangle { x: origin_x + columns.edge_x(col, scroll_x) - SEPARATOR, y: top, width: SEPARATOR, height }
}

/// Where the pinned columns end. Darker than the lines between columns, so it
/// still reads as the place the scrolling starts from.
fn pinned_edge(area: Rectangle) -> Rectangle {
    Rectangle { x: area.x + area.width - SEPARATOR, width: SEPARATOR, ..area }
}

fn rounded(renderer: &mut iced::Renderer, bounds: Rectangle, color: Color) {
    renderer.fill_quad(
        Quad { bounds, border: Border { radius: (bounds.width.min(bounds.height) / 2.0).into(), ..Border::default() }, ..Quad::default() },
        Background::Color(color),
    );
}

impl<'a, D, Message> Widget<Message, Theme, iced::Renderer> for Grid<'a, D, Message>
where
    D: GridData,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            bench_down: true,
            ..State::default()
        })
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.width(Length::Fill).height(Length::Fill).max())
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        let frame = self.frame(bounds);
        let columns = self.geometry(state);

        if let Some((token, row)) = self.reveal {
            if token != state.reveal_token {
                state.reveal_token = token;
                if row < self.data.row_count() {
                    state.scroll.y = geometry::reveal(state.scroll.y, row as f32 * self.row_height, self.row_height, frame.body.height);
                }
                shell.request_redraw();
            }
        }
        self.clamp_scroll(state, &columns, &frame);

        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => state.modifiers = *modifiers,
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, text, .. }) if state.focused && !self.locked => {
                state.modifiers = *modifiers;
                if self.handle_key(state, &columns, &frame, key, *modifiers, text.as_deref(), shell) {
                    shell.capture_event();
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if self.locked || !cursor.is_over(bounds) {
                    return;
                }
                let (dx, dy) = match *delta {
                    mouse::ScrollDelta::Lines { x, y } => (x * WHEEL_LINE, y * WHEEL_LINE),
                    mouse::ScrollDelta::Pixels { x, y } => (x, y),
                };
                let (dx, dy) = if state.modifiers.shift() && dx == 0.0 { (dy, 0.0) } else { (dx, dy) };
                let before = state.scroll;
                state.scroll.x -= dx;
                state.scroll.y -= dy;
                self.clamp_scroll(state, &columns, &frame);
                if state.scroll != before {
                    state.hover = None;
                    state.expanded = None;
                    shell.request_redraw();
                }
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let Some(at) = cursor.position() else { return };
                match state.drag {
                    Drag::Select => {
                        let mut edge = 0.0;
                        if at.y < frame.body.y {
                            edge = -EDGE_SCROLL;
                        } else if at.y > frame.body.y + frame.body.height {
                            edge = EDGE_SCROLL;
                        }
                        state.scroll.y += edge;
                        self.clamp_scroll(state, &columns, &frame);
                        if let Some(target) = self.cell_near(state, &columns, &frame, at) {
                            if state.press_cell != Some(target) {
                                state.moved_since_press = true;
                            }
                            if let Some(live) = state.live.as_mut() {
                                live.extend_to(target);
                            }
                            shell.request_redraw();
                        }
                    }
                    Drag::Resize { col, origin, start } => {
                        let width = (start + at.x - origin).clamp(MIN_COLUMN, MAX_COLUMN);
                        state.resize_preview = Some((col, width));
                        shell.request_redraw();
                    }
                    Drag::Header { col, origin, moved } => {
                        if moved || (at.x - origin).abs() > DRAG_START {
                            state.drag = Drag::Header { col, origin, moved: true };
                            state.drop_slot = self.drop_target(state, &columns, &frame, at.x);
                            shell.request_redraw();
                        }
                    }
                    Drag::ThumbV { grab } => {
                        let track = frame.bar_v.height;
                        state.scroll.y =
                            geometry::scroll_from_thumb(track, self.content_height(), frame.body.height, at.y - frame.bar_v.y - grab);
                        shell.request_redraw();
                    }
                    Drag::ThumbH { grab } => {
                        let track = self.scroll_track(&columns, &frame);
                        let content = columns.total - columns.pinned_width;
                        state.scroll.x = geometry::scroll_from_thumb(
                            track,
                            content,
                            track,
                            at.x - frame.bar_h.x - columns.pinned_width - grab,
                        );
                        shell.request_redraw();
                    }
                    Drag::None => {
                        let hover = self.cell_at(state, &columns, &frame, at);
                        let hover_header = frame
                            .header
                            .contains(at)
                            .then(|| columns.at(at.x - frame.header.x, state.scroll.x))
                            .flatten();
                        if let Some((col, _)) = state.expanded {
                            if hover.map(|(_, c)| c) != Some(col) {
                                state.expanded = None;
                                shell.request_redraw();
                            }
                        }
                        let row_changed = hover.map(|h| h.0) != state.hover.map(|h| h.0);
                        if row_changed || hover_header != state.hover_header {
                            shell.request_redraw();
                        }
                        state.hover = hover;
                        state.hover_header = hover_header;
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(at) = cursor.position() else { return };
                if !bounds.contains(at) {
                    if state.focused {
                        state.focused = false;
                        shell.request_redraw();
                    }
                    return;
                }
                if self.locked {
                    shell.publish((self.on_event)(GridEvent::Dismiss));
                    shell.capture_event();
                    return;
                }
                state.focused = true;
                if frame.bar_v.contains(at) {
                    let track = frame.bar_v.height;
                    if let Some((offset, length)) = geometry::thumb(track, self.content_height(), frame.body.height, state.scroll.y) {
                        let y = at.y - frame.bar_v.y;
                        let grab = if (offset..offset + length).contains(&y) { y - offset } else { length / 2.0 };
                        state.drag = Drag::ThumbV { grab };
                        state.scroll.y = geometry::scroll_from_thumb(track, self.content_height(), frame.body.height, y - grab);
                    }
                } else if frame.bar_h.contains(at) {
                    let track = self.scroll_track(&columns, &frame);
                    let content = columns.total - columns.pinned_width;
                    if let Some((offset, length)) = geometry::thumb(track, content, track, state.scroll.x) {
                        let x = at.x - frame.bar_h.x - columns.pinned_width;
                        let grab = if (offset..offset + length).contains(&x) { x - offset } else { length / 2.0 };
                        state.drag = Drag::ThumbH { grab };
                        state.scroll.x = geometry::scroll_from_thumb(track, content, track, x - grab);
                    }
                } else if frame.header.contains(at) {
                    let x = at.x - frame.header.x;
                    if let Some(col) = columns.grip_at(x, state.scroll.x) {
                        state.drag = Drag::Resize { col, origin: at.x, start: columns.widths[col] };
                        state.resize_preview = Some((col, columns.widths[col]));
                    } else if let Some(col) = columns.at(x, state.scroll.x) {
                        state.drag = Drag::Header { col, origin: at.x, moved: false };
                    }
                } else if let Some((row, col)) = self.cell_at(state, &columns, &frame, at) {
                    self.press_body(state, &columns, &frame, row, col, shell);
                }
                shell.capture_event();
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let drag = std::mem::take(&mut state.drag);
                match drag {
                    Drag::None => {}
                    Drag::Select => {
                        if let Some(live) = state.live.take() {
                            shell.publish((self.on_event)(GridEvent::Select(live)));
                        }
                        if !state.moved_since_press {
                            if let Some((row, col)) = state.press_cell {
                                if let Some(width) = self.overflow_width(row, col, columns.widths[col]) {
                                    state.expanded = Some((col, width));
                                } else {
                                    state.expanded = None;
                                }
                                shell.publish((self.on_event)(GridEvent::Click { row, col, at: cursor.position().unwrap_or(Point::ORIGIN) }));
                            }
                        }
                    }
                    Drag::Resize { col, .. } => {
                        if let Some((_, width)) = state.resize_preview.take() {
                            shell.publish((self.on_event)(GridEvent::Resize { col, width: width.round() }));
                        }
                    }
                    Drag::Header { col, moved, .. } => {
                        let slot = state.drop_slot.take();
                        if moved {
                            if let Some(to) = slot.filter(|to| *to != col) {
                                shell.publish((self.on_event)(GridEvent::Move { from: col, to }));
                            }
                        } else if self.columns.get(col).is_some_and(|c| c.sortable) {
                            shell.publish((self.on_event)(GridEvent::Sort(col)));
                        }
                    }
                    Drag::ThumbV { .. } | Drag::ThumbH { .. } => {}
                }
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let Some(at) = cursor.position() else { return };
                if !bounds.contains(at) || self.locked {
                    return;
                }
                state.focused = true;
                if frame.header.contains(at) {
                    if let Some(col) = columns.at(at.x - frame.header.x, state.scroll.x) {
                        shell.publish((self.on_event)(GridEvent::HeaderContext { col, at }));
                    }
                } else if let Some((row, col)) = self.cell_at(state, &columns, &frame, at) {
                    if !self.selection.contains(row, col) {
                        shell.publish((self.on_event)(GridEvent::Select(Selection::single(row, col))));
                    }
                    shell.publish((self.on_event)(GridEvent::Context { row, col, at }));
                }
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                if state.hover.take().is_some() || state.hover_header.take().is_some() || state.expanded.take().is_some() {
                    shell.request_redraw();
                }
            }
            _ => {}
        }
        // The benchmark drives the same state the wheel does, nothing cheaper.
        if bench_enabled() && matches!(event, Event::Window(iced::window::Event::RedrawRequested(_))) {
            let limit = geometry::max_scroll_y(self.data.row_count(), self.row_height, frame.body.height);
            if limit > 0.0 {
                let step = self.row_height * 1.5;
                state.scroll.y += if state.bench_down { step } else { -step };
                if state.scroll.y >= limit {
                    state.bench_down = false;
                }
                if state.scroll.y <= 0.0 {
                    state.bench_down = true;
                }
                self.clamp_scroll(state, &columns, &frame);
            }
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: Cursor,
        _viewport: &Rectangle,
    ) {
        let started = perf_enabled().then(Instant::now);
        let state = tree.state.downcast_ref::<State>();
        let p = theme::p(theme);
        let bounds = layout.bounds();
        let frame = self.frame(bounds);
        let columns = self.geometry(state);
        let body = frame.body;
        let rows = geometry::visible_rows(state.scroll.y, body.height, self.row_height, self.data.row_count());
        let scrolling = columns.visible(state.scroll.x, body.width);
        let pinned_w = columns.pinned_width.min(body.width);
        let moving = Rectangle { x: body.x + pinned_w, width: (body.width - pinned_w).max(0.0), ..body };
        let fixed = Rectangle { width: pinned_w, ..body };

        // iced draws every quad of a layer before its text, so regions that
        // overlap on screen each need their own layer.
        quad(renderer, bounds, p.surface);
        if moving.width > 0.0 {
            renderer.with_layer(moving, |r| self.draw_body(r, p, state, &columns, scrolling.clone(), moving, body, rows.clone()));
        }
        if pinned_w > 0.0 {
            renderer.with_layer(fixed, |r| {
                quad(r, fixed, p.surface);
                self.draw_body(r, p, state, &columns, 0..columns.pinned, fixed, body, rows.clone());
                quad(r, pinned_edge(fixed), p.control_edge);
            });
        }
        let header_moving = Rectangle { x: frame.header.x + pinned_w, width: (frame.header.width - pinned_w).max(0.0), ..frame.header };
        let header_fixed = Rectangle { width: pinned_w, ..frame.header };
        if header_moving.width > 0.0 {
            renderer.with_layer(header_moving, |r| {
                self.draw_header(r, p, state, &columns, scrolling.clone(), header_moving, frame.header);
            });
        }
        if pinned_w > 0.0 {
            renderer.with_layer(header_fixed, |r| {
                self.draw_header(r, p, state, &columns, 0..columns.pinned, header_fixed, frame.header);
                quad(r, pinned_edge(header_fixed), p.control_edge);
            });
        }
        renderer.with_layer(bounds, |r| {
            quad(r, Rectangle { x: frame.header.x + frame.header.width, width: BAR, ..frame.header }, p.raised);
            quad(r, frame.bar_v, p.surface);
            quad(r, frame.bar_h, p.surface);
            quad(r, Rectangle { x: frame.bar_v.x, y: frame.bar_h.y, width: BAR, height: BAR }, p.surface);
            let hot = matches!(state.drag, Drag::ThumbV { .. } | Drag::ThumbH { .. });
            let tint = if hot { p.text3 } else { p.border_strong };
            if let Some((offset, length)) = geometry::thumb(frame.bar_v.height, self.content_height(), body.height, state.scroll.y) {
                rounded(r, Rectangle { x: frame.bar_v.x + 2.0, y: frame.bar_v.y + offset, width: BAR - 4.0, height: length }, tint);
            }
            let track = self.scroll_track(&columns, &frame);
            let content = columns.total - columns.pinned_width;
            if let Some((offset, length)) = geometry::thumb(track, content, track, state.scroll.x) {
                rounded(
                    r,
                    Rectangle {
                        x: frame.bar_h.x + columns.pinned_width + offset,
                        y: frame.bar_h.y + 2.0,
                        width: length,
                        height: BAR - 4.0,
                    },
                    tint,
                );
            }
            if let Some(slot) = state.drop_slot {
                let x = frame.header.x + columns.screen_x(slot, state.scroll.x);
                quad(r, Rectangle { x: x - 1.0, y: bounds.y, width: 2.0, height: bounds.height - BAR }, p.accent);
            }
        });
        if let Some(started) = started {
            record_draw(started.elapsed());
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        match state.drag {
            Drag::Resize { .. } => return mouse::Interaction::ResizingHorizontally,
            Drag::Header { moved: true, .. } => return mouse::Interaction::Grabbing,
            _ => {}
        }
        let Some(at) = cursor.position_over(layout.bounds()) else { return mouse::Interaction::None };
        let frame = self.frame(layout.bounds());
        if frame.header.contains(at) {
            let columns = self.geometry(state);
            let x = at.x - frame.header.x;
            if columns.grip_at(x, state.scroll.x).is_some() {
                return mouse::Interaction::ResizingHorizontally;
            }
            if columns.at(x, state.scroll.x).and_then(|c| self.columns.get(c)).is_some_and(|c| c.sortable) {
                return mouse::Interaction::Pointer;
            }
        }
        mouse::Interaction::None
    }

    fn operate(
        &mut self,
        _tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        operation.container(Some(&self.id), layout.bounds());
    }
}

impl<'a, D, Message> From<Grid<'a, D, Message>> for Element<'a, Message>
where
    D: GridData + 'a,
    Message: 'a,
{
    fn from(grid: Grid<'a, D, Message>) -> Self {
        Element::new(grid)
    }
}

// --- Measuring -------------------------------------------------------------
// The whole point of this migration is scroll cost, so it stays measurable in a
// real build: TBS_PERF=1 prints draw timings, TBS_BENCH_SCROLL=1 scrolls by itself.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

fn perf_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("TBS_PERF").is_some())
}

fn bench_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("TBS_BENCH_SCROLL").is_some())
}

static DRAWS: AtomicU64 = AtomicU64::new(0);
static DRAW_NANOS: AtomicU64 = AtomicU64::new(0);
static DRAW_WORST: AtomicU64 = AtomicU64::new(0);
static WINDOW_START: OnceLock<std::sync::Mutex<Instant>> = OnceLock::new();

fn record_draw(took: Duration) {
    let nanos = took.as_nanos() as u64;
    let count = DRAWS.fetch_add(1, Ordering::Relaxed) + 1;
    DRAW_NANOS.fetch_add(nanos, Ordering::Relaxed);
    DRAW_WORST.fetch_max(nanos, Ordering::Relaxed);
    let clock = WINDOW_START.get_or_init(|| std::sync::Mutex::new(Instant::now()));
    let mut start = clock.lock().unwrap();
    if start.elapsed() >= Duration::from_secs(2) {
        let seconds = start.elapsed().as_secs_f64();
        let total = DRAW_NANOS.swap(0, Ordering::Relaxed);
        let worst = DRAW_WORST.swap(0, Ordering::Relaxed);
        DRAWS.store(0, Ordering::Relaxed);
        eprintln!(
            "grid: {:.0} frames/s, draw avg {:.3} ms, worst {:.3} ms",
            count as f64 / seconds,
            total as f64 / count as f64 / 1e6,
            worst as f64 / 1e6
        );
        *start = Instant::now();
    }
}

/// Reads the lines back from a picture of the grid. The simulator draws through
/// the same code as the window, at twice the size. Needs `png`, so it runs with
/// `--features devtools`.
#[cfg(all(test, feature = "devtools"))]
mod tests {
    use iced_test::Simulator;
    use iced_test::core::Settings;

    use super::*;

    const WINDOW: Size = Size::new(420.0, 200.0);
    const HEADER: f32 = 30.0;
    const ROW: f32 = 28.0;

    /// Cells without text: a pixel is then a fill or a line and nothing else.
    struct Blank(usize);

    impl GridData for Blank {
        fn row_count(&self) -> usize {
            self.0
        }

        fn cell(&self, _row: usize, _col: usize) -> Cow<'_, str> {
            Cow::Borrowed("")
        }
    }

    struct Picture {
        width: usize,
        rgba: Vec<u8>,
    }

    impl Picture {
        /// The colour at a point of the layout; the picture is twice as large.
        fn at(&self, x: f32, y: f32) -> Color {
            let at = ((y * 2.0) as usize * self.width + (x * 2.0) as usize) * 4;
            Color::from_rgb8(self.rgba[at], self.rgba[at + 1], self.rgba[at + 2])
        }

        fn is(&self, x: f32, y: f32, wanted: Color) -> bool {
            let seen = self.at(x, y);
            let near = |a: f32, b: f32| (a - b).abs() <= 3.0 / 255.0;
            near(seen.r, wanted.r) && near(seen.g, wanted.g) && near(seen.b, wanted.b)
        }
    }

    fn widths(all: &[f32]) -> Vec<GridColumn> {
        all.iter().map(|width| GridColumn::new("", *width)).collect()
    }

    fn draw(columns: &[GridColumn], pinned: usize, rows: usize, selection: &Selection, dark: bool, scroll: f32) -> Picture {
        let table: Element<'_, ()> = grid(Blank(rows), columns, selection, |_| ()).pinned(pinned).into();
        let mut sim = Simulator::with_size(Settings::default(), WINDOW, table);
        if scroll != 0.0 {
            sim.point_at(Point::new(WINDOW.width / 2.0, WINDOW.height / 2.0));
            let _ = sim.simulate([Event::Mouse(mouse::Event::WheelScrolled { delta: mouse::ScrollDelta::Pixels { x: -scroll, y: 0.0 } })]);
        }
        picture(&mut sim, dark)
    }

    /// What the simulator has on screen right now.
    fn picture<Message>(sim: &mut Simulator<'_, Message>, dark: bool) -> Picture {
        let shot = sim.snapshot(&theme::iced_theme(dark)).expect("a snapshot");
        // The simulator names the file after its renderer.
        let folder = tempfile::tempdir().unwrap();
        assert!(shot.matches_image(folder.path().join("grid")).expect("a picture"));
        let file = std::fs::read_dir(folder.path()).unwrap().flatten().next().expect("a file").path();
        let mut reader = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(file).unwrap())).read_info().unwrap();
        let mut rgba = vec![0; reader.output_buffer_size().expect("a picture that fits in memory")];
        let info = reader.next_frame(&mut rgba).unwrap();
        rgba.truncate(info.buffer_size());
        Picture { width: info.width as usize, rgba }
    }

    fn palette(dark: bool) -> &'static Palette {
        if dark { &theme::DARK } else { &theme::LIGHT }
    }

    /// The middle of the header, and of a row.
    fn header_y() -> f32 {
        HEADER / 2.0
    }

    fn row_y(row: usize) -> f32 {
        HEADER + row as f32 * ROW + ROW / 2.0
    }

    #[test]
    fn a_line_parts_every_column_in_the_header_and_in_the_rows_of_both_themes() {
        let columns = widths(&[60.0, 90.0, 70.0, 120.0]);
        let edges = [60.0, 150.0, 220.0, 340.0];
        for dark in [false, true] {
            let p = palette(dark);
            assert_ne!(p.border_strong, p.surface);
            let picture = draw(&columns, 0, 4, &Selection::default(), dark, 0.0);
            for edge in edges {
                for y in [header_y(), row_y(0), row_y(3)] {
                    assert!(picture.is(edge - 0.5, y, p.border_strong), "no line at {edge}, {y} (dark: {dark}): {:?}", picture.at(edge - 0.5, y));
                    assert!(picture.is(edge - 2.5, y, p.surface), "the line at {edge}, {y} is too wide on the left (dark: {dark})");
                    assert!(picture.is(edge + 1.5, y, p.surface), "the line at {edge}, {y} is too wide on the right (dark: {dark})");
                }
            }
        }
    }

    #[test]
    fn the_lines_end_with_the_last_row() {
        let columns = widths(&[60.0, 90.0]);
        let picture = draw(&columns, 0, 2, &Selection::default(), false, 0.0);
        assert!(picture.is(59.5, row_y(1), theme::LIGHT.border_strong));
        assert!(picture.is(59.5, row_y(3), theme::LIGHT.surface));
    }

    #[test]
    fn the_lines_scroll_with_their_columns_and_the_pinned_ones_stay() {
        // 800 of columns in a 410 body: 37 to the right puts every scrolling edge off its old place.
        let columns = widths(&[60.0, 90.0, 110.0, 100.0, 130.0, 150.0, 160.0]);
        let scroll = 37.0;
        for dark in [false, true] {
            let p = palette(dark);
            let picture = draw(&columns, 2, 4, &Selection::default(), dark, scroll);
            for y in [header_y(), row_y(1)] {
                // Pinned: the line between the two, and the darker edge where they end.
                assert!(picture.is(59.5, y, p.border_strong), "the pinned line moved at {y} (dark: {dark})");
                assert!(picture.is(149.5, y, p.control_edge), "no pinned edge at {y} (dark: {dark}): {:?}", picture.at(149.5, y));
                // Scrolling: at their columns' new edges, in the header as in the rows.
                for edge in [260.0, 360.0] {
                    let x = edge - scroll - 0.5;
                    assert!(picture.is(x, y, p.border_strong), "no line at {x}, {y} (dark: {dark}): {:?}", picture.at(x, y));
                    assert!(picture.is(edge - 0.5, y, p.surface), "a line stayed behind at {edge}, {y} (dark: {dark})");
                }
            }
        }
    }

    #[test]
    fn the_lines_stand_between_the_columns_in_whatever_order_they_are() {
        let sizes = [60.0, 90.0, 70.0, 120.0];
        let p = &theme::LIGHT;
        for order in [[0, 1, 2, 3], [3, 2, 1, 0], [2, 0, 3, 1]] {
            let arranged: Vec<f32> = order.iter().map(|at| sizes[*at]).collect();
            let picture = draw(&widths(&arranged), 1, 3, &Selection::default(), false, 0.0);
            let mut edge = 0.0;
            for (col, width) in arranged.iter().enumerate() {
                edge += width;
                // Whichever column comes first is the pinned one, with the darker edge.
                let line = if col == 0 { p.control_edge } else { p.border_strong };
                for y in [header_y(), row_y(2)] {
                    assert!(picture.is(edge - 0.5, y, line), "order {order:?}: no line at {edge}, {y}: {:?}", picture.at(edge - 0.5, y));
                    assert!(picture.is(edge - 2.5, y, p.surface) && picture.is(edge + 1.5, y, p.surface), "order {order:?}: more than a line at {edge}, {y}");
                }
            }
        }
    }

    #[test]
    fn a_line_goes_with_its_column_while_the_column_is_resized() {
        let columns = widths(&[60.0, 90.0, 70.0]);
        let selection = Selection::default();
        let table: Element<'_, GridEvent> = grid(Blank(3), &columns, &selection, |event| event).into();
        let mut sim = Simulator::with_size(Settings::default(), WINDOW, table);
        // Taken by the line itself: the grip is where the line is drawn.
        sim.point_at(Point::new(149.5, header_y()));
        let _ = sim.simulate([Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))]);
        let to = Point::new(179.5, header_y());
        sim.point_at(to);
        let _ = sim.simulate([Event::Mouse(mouse::Event::CursorMoved { position: to })]);

        // Mid-drag the column is 120 wide: its line and the next one have moved, in the header and the rows.
        let p = &theme::LIGHT;
        let shown = picture(&mut sim, false);
        for y in [header_y(), row_y(1)] {
            for edge in [60.0, 180.0, 250.0] {
                assert!(shown.is(edge - 0.5, y, p.border_strong), "no line at {edge}, {y}: {:?}", shown.at(edge - 0.5, y));
            }
            for left_behind in [150.0, 220.0] {
                assert!(shown.is(left_behind - 0.5, y, p.surface), "a line stayed at {left_behind}, {y}");
            }
        }

        let _ = sim.simulate([Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))]);
        let said: Vec<GridEvent> = sim.into_messages().collect();
        assert_eq!(said, [GridEvent::Resize { col: 1, width: 120.0 }]);
    }

    #[test]
    fn a_selected_cell_keeps_its_line_and_the_cursor_keeps_its_frame() {
        let columns = widths(&[60.0, 90.0, 70.0]);
        let mut selection = Selection::single(0, 0);
        selection.extend_to((1, 1));
        let p = &theme::LIGHT;
        let picture = draw(&columns, 0, 4, &selection, false, 0.0);
        // Selected, not the cursor: the fill, and the line over it.
        assert!(picture.is(30.0, row_y(0), p.selection));
        assert!(picture.is(59.5, row_y(0), p.border_strong));
        // The cursor: its frame is whole, over the line of its own column.
        assert!(picture.is(149.5, row_y(1), p.selection_edge), "{:?}", picture.at(149.5, row_y(1)));
        assert!(picture.is(60.5, row_y(1), p.selection_edge));
        // The line before the cursor's cell belongs to the neighbour and stays.
        assert!(picture.is(59.5, row_y(1), p.border_strong));
    }
}
