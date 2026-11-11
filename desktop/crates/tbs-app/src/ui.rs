//! Building blocks shared by every screen.

use std::collections::HashMap;
use std::sync::LazyLock;

use iced::widget::{
    Column, Id, Row, Space, button, center, column, container, mouse_area, opaque, pick_list, pin, row, rule, scrollable,
    space, stack, svg, text, text_input, tooltip,
};
use iced::{Alignment, Element, Fill, Length, Padding, Point, Rectangle, Size, Theme};

pub use crate::icon_set::Icon;
use crate::theme::{self, Tone, control, size, space as gap};

pub type El<'a, M> = Element<'a, M>;

static HANDLES: LazyLock<std::sync::Mutex<HashMap<Icon, svg::Handle>>> = LazyLock::new(Default::default);

pub fn icon_handle(icon: Icon) -> svg::Handle {
    HANDLES.lock().unwrap().entry(icon).or_insert_with(|| svg::Handle::from_memory(icon.bytes())).clone()
}

/// An icon that follows the surrounding text colour role.
pub fn icon<'a>(name: Icon, px: f32) -> svg::Svg<'a> {
    svg(icon_handle(name)).width(px).height(px).style(|theme: &Theme, _| svg::Style { color: Some(theme::p(theme).text2) })
}

pub fn icon_tone<'a>(name: Icon, px: f32, tone: Tone) -> svg::Svg<'a> {
    svg(icon_handle(name))
        .width(px)
        .height(px)
        .style(move |theme: &Theme, _| svg::Style { color: Some(theme::p(theme).tone(tone).0) })
}

pub fn label<'a>(value: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(value).size(size::BASE)
}

pub fn muted<'a>(value: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(value).size(size::SM).style(|theme: &Theme| text::Style { color: Some(theme::p(theme).text2) })
}

pub fn faint<'a>(value: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(value).size(size::SM).style(|theme: &Theme| text::Style { color: Some(theme::p(theme).text3) })
}

pub fn title<'a>(value: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(value).size(size::XL).font(theme::FONT_BOLD)
}

pub fn heading<'a>(value: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(value).size(size::MD).font(theme::FONT_BOLD)
}

pub fn strong<'a>(value: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(value).size(size::BASE).font(theme::FONT_MEDIUM)
}

pub fn toned<'a>(value: impl text::IntoFragment<'a>, tone: Tone) -> text::Text<'a> {
    text(value)
        .size(size::SM)
        .style(move |theme: &Theme| text::Style { color: Some(theme::p(theme).tone(tone).0) })
}

pub fn error_line<'a, M: 'a>(message: impl text::IntoFragment<'a>) -> El<'a, M> {
    row![icon_tone(Icon::Alert, 14.0, Tone::Danger), toned(message, Tone::Danger)]
        .spacing(gap::XS + 2.0)
        .align_y(Alignment::Center)
        .into()
}

/// Inline callout for states the user should read before acting.
pub fn notice<'a, M: 'a>(tone: Tone, message: impl text::IntoFragment<'a>) -> El<'a, M> {
    let glyph = match tone {
        Tone::Success => Icon::Success,
        Tone::Danger => Icon::Alert,
        Tone::Warning => Icon::Warning,
        _ => Icon::Info,
    };
    container(
        row![icon_tone(glyph, 16.0, tone), text(message).size(size::SM).width(Fill)]
            .spacing(gap::SM)
            .align_y(Alignment::Center),
    )
    .padding([gap::SM, gap::MD])
    .width(Fill)
    .style(theme::tinted(tone))
    .into()
}

pub fn badge<'a, M: 'a>(value: impl text::IntoFragment<'a>, tone: Tone) -> El<'a, M> {
    container(text(value).size(size::XS).font(theme::FONT_MEDIUM))
        .padding([1.0, gap::SM])
        .style(theme::pill(tone))
        .into()
}

pub fn status_dot<'a, M: 'a>(tone: Tone) -> El<'a, M> {
    container(Space::new()).width(8.0).height(8.0).style(theme::dot(tone)).into()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    Primary,
    #[default]
    Secondary,
    Ghost,
    Danger,
}

fn styled<'a, M: 'a>(b: button::Button<'a, M>, kind: Kind) -> button::Button<'a, M> {
    match kind {
        Kind::Primary => b.style(theme::btn_primary),
        Kind::Secondary => b.style(theme::btn_secondary),
        Kind::Ghost => b.style(theme::btn_ghost),
        Kind::Danger => b.style(theme::btn_danger),
    }
}

/// A text button. `None` for the message renders it disabled.
pub fn btn<'a, M: Clone + 'a>(value: impl text::IntoFragment<'a>, kind: Kind, on_press: Option<M>) -> button::Button<'a, M> {
    let body = container(text(value).size(size::BASE).font(theme::FONT_MEDIUM)).center_y(Fill);
    styled(button(body), kind).padding([0.0, gap::MD]).height(control::MD).on_press_maybe(on_press)
}

pub fn btn_icon<'a, M: Clone + 'a>(
    glyph: Icon,
    value: impl text::IntoFragment<'a>,
    kind: Kind,
    on_press: Option<M>,
) -> button::Button<'a, M> {
    let tint = match kind {
        Kind::Primary => None,
        _ => Some(()),
    };
    let image: El<'a, M> = if tint.is_some() {
        icon(glyph, 16.0).into()
    } else {
        svg(icon_handle(glyph))
            .width(16.0)
            .height(16.0)
            .style(|theme: &Theme, _| svg::Style { color: Some(theme::p(theme).on_accent) })
            .into()
    };
    let body = row![image, text(value).size(size::BASE).font(theme::FONT_MEDIUM)]
        .spacing(gap::XS + 2.0)
        .align_y(Alignment::Center);
    styled(button(container(body).center_y(Fill)), kind).padding([0.0, gap::MD - 2.0]).height(control::MD).on_press_maybe(on_press)
}

/// A square icon button with a tooltip, because the glyph alone may not be enough.
pub fn icon_btn<'a, M: Clone + 'a>(glyph: Icon, hint: &'a str, on_press: Option<M>) -> El<'a, M> {
    let b = button(center(icon(glyph, 16.0)))
        .width(control::MD)
        .height(control::MD)
        .padding(0)
        .style(theme::btn_ghost)
        .on_press_maybe(on_press);
    hint_on(b, hint)
}

pub fn hint_on<'a, M: 'a>(content: impl Into<El<'a, M>>, hint: &'a str) -> El<'a, M> {
    hint_at(content, hint, tooltip::Position::Bottom)
}

/// The same hint to the right, for what stands at the window's left edge.
pub fn hint_beside<'a, M: 'a>(content: impl Into<El<'a, M>>, hint: &'a str) -> El<'a, M> {
    hint_at(content, hint, tooltip::Position::Right)
}

fn hint_at<'a, M: 'a>(content: impl Into<El<'a, M>>, hint: &'a str, position: tooltip::Position) -> El<'a, M> {
    tooltip(content, container(text(hint).size(size::SM)).padding([gap::XS, gap::SM]).style(theme::popover), position).gap(4).into()
}

pub fn input<'a, M: Clone + 'a>(placeholder: &str, value: &str) -> text_input::TextInput<'a, M> {
    text_input(placeholder, value).size(size::BASE).padding([6.5, 10.0]).style(theme::input)
}

pub fn secret_input<'a, M: Clone + 'a>(placeholder: &str, value: &str) -> text_input::TextInput<'a, M> {
    input(placeholder, value).secure(true)
}

/// A labelled form row; the error, when present, sits under the control.
pub fn field<'a, M: 'a>(caption: &'a str, control: impl Into<El<'a, M>>) -> Column<'a, M> {
    column![muted(caption), control.into()].spacing(gap::XS)
}

pub fn select<'a, T, M>(
    options: impl std::borrow::Borrow<[T]> + 'a,
    selected: Option<T>,
    on_select: impl Fn(T) -> M + 'a,
) -> pick_list::PickList<'a, T, impl std::borrow::Borrow<[T]> + 'a, T, M>
where
    T: ToString + PartialEq + Clone + 'a,
    M: Clone + 'a,
{
    pick_list(options, selected, on_select)
        .text_size(size::BASE)
        .padding([6.0, 10.0])
        .style(theme::select)
        .menu_style(theme::select_menu)
}

pub fn check<'a, M: Clone + 'a>(caption: impl Into<String>, value: bool, on_toggle: impl Fn(bool) -> M + 'a) -> iced::widget::Checkbox<'a, M> {
    iced::widget::checkbox(value).label(caption.into()).on_toggle(on_toggle).size(16).text_size(size::BASE).style(theme::check)
}

pub fn switch<'a, M: Clone + 'a>(value: bool, on_toggle: impl Fn(bool) -> M + 'a) -> iced::widget::Toggler<'a, M> {
    iced::widget::toggler(value).on_toggle(on_toggle).size(18).style(theme::switch)
}

pub fn card<'a, M: 'a>(content: impl Into<El<'a, M>>) -> container::Container<'a, M> {
    container(content).padding(gap::LG).width(Fill).style(theme::card)
}

pub fn divider<'a, M: 'a>() -> El<'a, M> {
    rule::horizontal(1).style(theme::divider).into()
}

pub fn divider_v<'a, M: 'a>() -> El<'a, M> {
    rule::vertical(1).style(theme::divider).into()
}

pub fn scroll<'a, M: 'a>(content: impl Into<El<'a, M>>) -> scrollable::Scrollable<'a, M> {
    scrollable(content)
        .direction(scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(8).scroller_width(6).margin(2)))
        .style(theme::scroller)
}

pub fn fill<'a>() -> Space {
    space::horizontal()
}

/// Pushes what follows to the bottom of a column.
pub fn grow<'a>() -> Space {
    space::vertical()
}

/// A page header with a title on the left and actions on the right.
pub fn page_header<'a, M: 'a>(name: &'a str, hint: Option<&'a str>, actions: impl Into<El<'a, M>>) -> El<'a, M> {
    // The heading takes what the actions leave: in a narrow window its hint
    // wraps, where a heading sized first pushed the buttons out of the page.
    let mut heading_col = Column::new().push(title(name)).spacing(gap::XXS).width(Fill);
    if let Some(hint) = hint {
        heading_col = heading_col.push(muted(hint));
    }
    row![heading_col, actions.into()].align_y(Alignment::Center).spacing(gap::MD).into()
}

/// A segmented control over a small fixed set of choices.
pub fn segments<'a, T: Copy + PartialEq + 'a, M: Clone + 'a>(
    items: &[(T, &'a str)],
    active: T,
    on_pick: impl Fn(T) -> M,
) -> El<'a, M> {
    let mut line = Row::new().spacing(gap::XXS);
    for (value, caption) in items {
        line = line.push(
            button(container(text(*caption).size(size::SM).font(theme::FONT_MEDIUM)).center_y(Fill))
                .height(control::SM - 4.0)
                .padding([0.0, gap::SM + 2.0])
                .style(theme::btn_segment(*value == active))
                .on_press(on_pick(*value)),
        );
    }
    container(line).padding(gap::XXS).style(theme::inset).into()
}

/// A dialog over a dimmed page. Clicking the dimmed area sends `on_dismiss`.
pub fn modal<'a, M: Clone + 'a>(
    base: impl Into<El<'a, M>>,
    content: impl Into<El<'a, M>>,
    width: impl Into<Length>,
    on_dismiss: Option<M>,
) -> El<'a, M> {
    let dialog = container(content).padding(gap::XL).width(width).max_height(760.0).style(theme::dialog);
    let mut backdrop = mouse_area(container(center(opaque(dialog))).width(Fill).height(Fill).padding(gap::XL).style(theme::scrim));
    if let Some(message) = on_dismiss {
        backdrop = backdrop.on_press(message);
    }
    stack![base.into(), opaque(backdrop)].width(Fill).height(Fill).into()
}

/// A side sheet that slides over the right edge of the page.
pub fn sheet<'a, M: Clone + 'a>(
    base: impl Into<El<'a, M>>,
    content: impl Into<El<'a, M>>,
    width: f32,
    on_dismiss: Option<M>,
) -> El<'a, M> {
    let panel = container(content).width(width).height(Fill).style(theme::surface);
    let mut dim = mouse_area(container(Space::new()).width(Fill).height(Fill).style(theme::scrim));
    if let Some(message) = on_dismiss {
        dim = dim.on_press(message);
    }
    stack![base.into(), row![dim, opaque(panel)]].width(Fill).height(Fill).into()
}

/// The inside of a side sheet: a fixed head and foot around a scrolling body.
pub fn sheet_panel<'a, M: Clone + 'a>(
    eyebrow: impl text::IntoFragment<'a>,
    name: impl text::IntoFragment<'a>,
    hint: impl text::IntoFragment<'a>,
    on_close: M,
    body: impl Into<El<'a, M>>,
    footer: Option<El<'a, M>>,
) -> El<'a, M> {
    let head = column![faint(eyebrow), dialog_title(name, Some(on_close)), muted(hint)].spacing(gap::XS);
    let mut panel = column![
        container(head).padding([gap::LG, gap::XL]).width(Fill),
        divider(),
        scroll(container(body).padding([gap::LG, gap::XL]).width(Fill)).height(Fill),
    ];
    if let Some(footer) = footer {
        panel = panel.push(divider()).push(container(footer).padding([gap::MD, gap::XL]).width(Fill));
    }
    panel.height(Fill).into()
}

/// A titled group inside a sheet.
pub fn section<'a, M: 'a>(name: impl text::IntoFragment<'a>, content: impl Into<El<'a, M>>) -> El<'a, M> {
    column![strong(name), content.into()].spacing(gap::SM).width(Fill).into()
}

/// A small toggle in a row of choices.
pub fn chip<'a, M: Clone + 'a>(caption: impl text::IntoFragment<'a>, active: bool, on_press: M) -> El<'a, M> {
    button(text(caption).size(size::SM)).padding([gap::XS, gap::SM]).style(theme::btn_chip(active)).on_press(on_press).into()
}

pub fn dialog_title<'a, M: Clone + 'a>(name: impl text::IntoFragment<'a>, on_close: Option<M>) -> El<'a, M> {
    row![text(name).size(size::LG).font(theme::FONT_BOLD), fill(), icon_btn(Icon::Close, "Закрыть", on_close)]
        .align_y(Alignment::Center)
        .into()
}

/// Confirmation with a stated consequence, used before anything destructive.
pub fn confirm<'a, M: Clone + 'a>(
    base: impl Into<El<'a, M>>,
    name: &'a str,
    body: impl text::IntoFragment<'a>,
    action: &'a str,
    on_confirm: M,
    on_cancel: M,
) -> El<'a, M> {
    modal(
        base,
        column![
            text(name).size(size::LG).font(theme::FONT_BOLD),
            text(body).size(size::BASE),
            row![
                fill(),
                btn("Отмена", Kind::Secondary, Some(on_cancel.clone())),
                btn(action, Kind::Danger, Some(on_confirm)),
            ]
            .spacing(gap::SM),
        ]
        .spacing(gap::MD),
        420.0,
        Some(on_cancel),
    )
}

pub fn empty_state<'a, M: 'a>(glyph: Icon, head: &'a str, hint: &'a str) -> El<'a, M> {
    center(
        column![icon(glyph, 28.0), strong(head), muted(hint)]
            .spacing(gap::SM)
            .align_x(Alignment::Center),
    )
    .into()
}

pub fn loading<'a, M: 'a>(caption: &'a str) -> El<'a, M> {
    center(row![icon(Icon::Loader, 16.0), muted(caption)].spacing(gap::SM).align_y(Alignment::Center)).into()
}

/// Space above the page content, shared so every screen starts on the same line.
pub const PAGE_PADDING: Padding = Padding { top: gap::LG, right: gap::XL, bottom: gap::LG, left: gap::XL };

pub fn page<'a, M: 'a>(content: impl Into<El<'a, M>>) -> El<'a, M> {
    container(content).width(Fill).height(Fill).padding(PAGE_PADDING).style(theme::page).into()
}

pub const EDGE: f32 = 8.0;
const EDITOR_MIN_WIDTH: f32 = 180.0;

/// Bound to the window so a panel opened at the edge stays readable.
pub fn clamped(at: Point, panel_size: Size, window: Size) -> Point {
    Point::new(at.x.min(window.width - panel_size.width - EDGE).max(EDGE), at.y.min(window.height - panel_size.height - EDGE).max(EDGE))
}

/// A panel at a point; a press anywhere else closes it.
pub fn floating<'a, M: Clone + 'a>(panel: El<'a, M>, at: Point, panel_size: Size, window: Size, on_close: M) -> El<'a, M> {
    let Point { x, y } = clamped(at, panel_size, window);
    let catcher = mouse_area(Space::new().width(Fill).height(Fill)).on_press(on_close.clone()).on_right_press(on_close);
    stack![opaque(catcher), pin(opaque(panel)).x(x).y(y)].width(Fill).height(Fill).into()
}

pub fn menu_item<'a, M: Clone + 'a>(glyph: Icon, caption: &'a str, danger: bool, message: M) -> El<'a, M> {
    button(row![icon(glyph, 16.0), text(caption).size(size::BASE)].spacing(gap::SM).align_y(Alignment::Center))
        .width(Fill)
        .padding([gap::XS + 2.0, gap::SM])
        .style(theme::btn_menu(danger))
        .on_press(message)
        .into()
}

pub fn menu_panel<'a, M: 'a>(content: impl Into<El<'a, M>>, width: f32) -> El<'a, M> {
    container(content).padding(gap::SM).width(width).style(theme::popover).into()
}

/// A text box laid over one grid cell; `rect` is in window coordinates.
pub fn cell_editor<'a, M: Clone + 'a>(
    id: Id,
    value: &str,
    rect: Rectangle,
    window: Size,
    on_input: impl Fn(String) -> M + 'a,
    on_submit: M,
    on_dismiss: M,
) -> El<'a, M> {
    let catcher = opaque(mouse_area(container(Space::new()).width(Fill).height(Fill)).on_press(on_dismiss));
    let width = rect.width.max(EDITOR_MIN_WIDTH);
    let x = rect.x.min(window.width - gap::SM - width).max(0.0);
    let height = rect.height.max(26.0);
    let input = text_input("", value)
        .id(id)
        .size(size::SM)
        .padding([gap::XXS, gap::XS])
        .style(theme::input_cell)
        .on_input(on_input)
        .on_submit(on_submit);
    let body = container(input).width(width).height(height).center_y(height);
    stack![catcher, pin(body).x(x).y(rect.y)].width(Fill).height(Fill).into()
}

/// A number with what it counts, for the row above a table.
pub fn stat<'a, M: 'a>(caption: &'a str, value: impl text::IntoFragment<'a>, hint: impl text::IntoFragment<'a>) -> El<'a, M> {
    let body = column![faint(caption), text(value).size(size::XL).font(theme::FONT_BOLD), faint(hint)].spacing(gap::XXS);
    container(body).padding([gap::SM, gap::MD]).width(Fill).style(theme::card).into()
}

/// A quick filter that says how many rows it would leave: «Списания 1 154».
pub fn count_chip<'a, M: Clone + 'a>(caption: &'a str, count: usize, active: bool, on_press: M) -> El<'a, M> {
    let body = row![
        text(caption).size(size::SM).font(theme::FONT_MEDIUM),
        text(tbs_core::format::count(count)).size(size::SM),
    ]
    .spacing(gap::XS);
    button(body).padding([gap::XS + 1.0, gap::MD]).style(theme::btn_chip(active)).on_press(on_press).into()
}

/// Previous and next page around "N / M".
pub fn pager<'a, M: Clone + 'a>(page: i64, pages: i64, on_page: impl Fn(i64) -> M) -> El<'a, M> {
    row![
        icon_btn(Icon::ChevronLeft, "Назад", (page > 1).then(|| on_page(page - 1))),
        muted(format!("{page} / {pages}")),
        icon_btn(Icon::ChevronRight, "Вперёд", (page < pages).then(|| on_page(page + 1))),
    ]
    .spacing(gap::XS)
    .align_y(Alignment::Center)
    .into()
}

