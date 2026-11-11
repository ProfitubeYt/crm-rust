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

/// A quiet text action for a line that has its buttons already: «Очистить»,
/// «Сбросить всё». As tall as a chip, so it sits in a row of them.
pub fn link<'a, M: Clone + 'a>(value: impl text::IntoFragment<'a>, on_press: Option<M>) -> button::Button<'a, M> {
    button(text(value).size(size::SM).font(theme::FONT_MEDIUM)).padding([gap::XS, gap::SM]).style(theme::btn_link).on_press_maybe(on_press)
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

/// The same row for a value that may be wrong: what is wrong stands right
/// under the control, not at the end of the form.
pub fn field_checked<'a, M: 'a>(caption: &'a str, control: impl Into<El<'a, M>>, problem: Option<&'a str>) -> Column<'a, M> {
    let row = field(caption, control);
    match problem {
        Some(reason) => row.push(error_line(reason)),
        None => row,
    }
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
    sheet_frame(head, body, footer)
}

/// A side sheet from its parts, for one whose head is more than a name: tabs,
/// a mark beside the title. The head and the foot stay; the body scrolls.
pub fn sheet_frame<'a, M: 'a>(head: impl Into<El<'a, M>>, body: impl Into<El<'a, M>>, footer: Option<El<'a, M>>) -> El<'a, M> {
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

/// The head of a side sheet with room for a mark beside its name, such as «не применено».
pub fn sheet_head<'a, M: Clone + 'a>(
    eyebrow: impl text::IntoFragment<'a>,
    name: impl text::IntoFragment<'a>,
    mark: Option<El<'a, M>>,
    hint: impl text::IntoFragment<'a>,
    on_close: M,
) -> El<'a, M> {
    let mut line = row![text(name).size(size::LG).font(theme::FONT_BOLD)].spacing(gap::SM).align_y(Alignment::Center);
    if let Some(mark) = mark {
        line = line.push(mark);
    }
    line = line.push(fill()).push(icon_btn(Icon::Close, "Закрыть", Some(on_close)));
    column![faint(eyebrow), line, muted(hint)].spacing(gap::XS).into()
}

/// The foot of a side sheet: what is going on, above a line of actions with
/// the quiet ones on the left and the ones that close the sheet on the right.
pub fn sheet_foot<'a, M: 'a>(status: Option<El<'a, M>>, left: Vec<El<'a, M>>, right: Vec<El<'a, M>>) -> El<'a, M> {
    let actions = Row::with_children(left).push(fill()).extend(right).spacing(gap::SM).align_y(Alignment::Center);
    match status {
        Some(status) => column![status, actions].spacing(gap::SM).into(),
        None => actions.into(),
    }
}

/// A titled group inside a sheet.
pub fn section<'a, M: 'a>(name: impl text::IntoFragment<'a>, content: impl Into<El<'a, M>>) -> El<'a, M> {
    column![strong(name), content.into()].spacing(gap::SM).width(Fill).into()
}

/// A small toggle in a row of choices.
pub fn chip<'a, M: Clone + 'a>(caption: impl text::IntoFragment<'a>, active: bool, on_press: M) -> El<'a, M> {
    button(text(caption).size(size::SM)).padding([gap::XS, gap::SM]).style(theme::btn_chip(active)).on_press(on_press).into()
}

/// A line of chips that runs on to the next line instead of pushing the page wider.
pub fn chip_row<'a, M: 'a>(chips: Vec<El<'a, M>>) -> El<'a, M> {
    Row::with_children(chips).spacing(gap::XS).align_y(Alignment::Center).wrap().vertical_spacing(gap::XS).into()
}

/// Something that is on and can be taken off: its words open it, its cross removes it.
pub fn tag<'a, M: Clone + 'a>(caption: impl text::IntoFragment<'a>, on_press: Option<M>, on_remove: Option<M>) -> El<'a, M> {
    let words = button(text(caption).size(size::SM).wrapping(text::Wrapping::None))
        .padding([gap::XS - 1.0, gap::SM])
        .style(theme::btn_tag)
        .on_press_maybe(on_press);
    let cross = button(center(icon_tone(Icon::Close, 12.0, Tone::Accent)))
        .width(TAG_CROSS)
        .height(TAG_CROSS)
        .padding(0)
        .style(theme::btn_tag)
        .on_press_maybe(on_remove);
    // One pixel in, so a lit half does not paint over the outline.
    container(row![words, hint_on(cross, "Убрать")].align_y(Alignment::Center)).padding(1).style(theme::tag).into()
}

const TAG_CROSS: f32 = 22.0;
const FOLD_GLYPH: f32 = 16.0;

/// A group of a sheet that folds to one line: its name, what is chosen in it,
/// and a way to clear it. Built with [`fold`].
pub struct Fold<'a, M> {
    name: &'a str,
    open: bool,
    on_toggle: M,
    hint: &'a str,
    summary: Option<String>,
    count: usize,
    marks: Vec<(&'a str, Tone)>,
    on_clear: Option<M>,
    body: Option<El<'a, M>>,
}

pub fn fold<'a, M: Clone + 'a>(name: &'a str, open: bool, on_toggle: M) -> Fold<'a, M> {
    Fold { name, open, on_toggle, hint: "", summary: None, count: 0, marks: Vec::new(), on_clear: None, body: None }
}

impl<'a, M: Clone + 'a> Fold<'a, M> {
    /// What the folded line says while nothing is chosen.
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = hint;
        self
    }

    /// What is chosen, in one line. Shown while the group is folded.
    pub fn summary(mut self, summary: Option<String>) -> Self {
        self.summary = summary;
        self
    }

    /// How many things are chosen. A group with any is edged and says how many.
    pub fn count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }

    /// A word beside the name: «изменено», «ошибка».
    pub fn mark(mut self, word: &'a str, tone: Tone) -> Self {
        self.marks.push((word, tone));
        self
    }

    /// «Очистить» at the end of the line; `None` leaves it out.
    pub fn on_clear(mut self, on_clear: Option<M>) -> Self {
        self.on_clear = on_clear;
        self
    }

    /// Drawn only while the group is open.
    pub fn body(mut self, body: impl Into<El<'a, M>>) -> Self {
        self.body = Some(body.into());
        self
    }
}

impl<'a, M: Clone + 'a> From<Fold<'a, M>> for El<'a, M> {
    fn from(fold: Fold<'a, M>) -> Self {
        let glyph = if fold.open { Icon::ChevronDown } else { Icon::ChevronRight };
        let mut name = row![icon(glyph, FOLD_GLYPH), strong(fold.name)].spacing(gap::SM).align_y(Alignment::Center);
        if fold.count > 0 {
            name = name.push(badge(fold.count.to_string(), Tone::Accent));
        }
        for (word, tone) in fold.marks {
            name = name.push(badge(word, tone));
        }
        let toggle = button(name).width(Fill).padding(gap::SM).style(theme::btn_option(false)).on_press(fold.on_toggle);
        let mut head = row![toggle].spacing(gap::XS).align_y(Alignment::Center);
        if let Some(on_clear) = fold.on_clear {
            head = head.push(link("Очистить", Some(on_clear)));
        }
        let mut block = column![head];
        if !fold.open {
            let line: El<'a, M> = match fold.summary {
                Some(summary) => text(summary).size(size::SM).wrapping(text::Wrapping::None).into(),
                None => faint(fold.hint).into(),
            };
            // Under the name, not under the arrow.
            let inset = Padding { top: 0.0, right: gap::SM, bottom: gap::SM, left: gap::SM + FOLD_GLYPH + gap::SM };
            block = block.push(container(line).padding(inset).width(Fill).clip(true));
        } else if let Some(body) = fold.body {
            block = block.push(container(body).padding(Padding { top: gap::XS, right: gap::MD, bottom: gap::MD, left: gap::MD }).width(Fill));
        }
        container(block).padding(gap::XXS).width(Fill).style(theme::group(fold.count > 0)).into()
    }
}

/// How many unticked lines a searchable list draws at once. A longer list
/// says so and asks for a narrower search.
pub const PICK_LIMIT: usize = 100;
const PICK_HEIGHT: f32 = 196.0;
const PICK_TALLY: f32 = 24.0;

/// The lines of a searchable list of ticks, each a code and its name.
#[derive(Debug, PartialEq, Eq)]
pub struct Shortlist<'a> {
    /// The ticked ones: always there and always on top, whatever the search says.
    pub ticked: Vec<(&'a str, &'a str)>,
    /// What the search found among the rest, as many as are drawn.
    pub rest: Vec<(&'a str, &'a str)>,
    /// How many the search found among the rest, drawn or not.
    pub found: usize,
}

pub fn shortlist<'a>(items: &[(&'a str, &'a str)], chosen: &'a [String], query: &str, limit: usize) -> Shortlist<'a> {
    let needle = query.trim().to_lowercase();
    let is_chosen = |code: &str| chosen.iter().any(|c| c.as_str() == code);
    let mut ticked: Vec<(&str, &str)> = items.iter().copied().filter(|(code, _)| is_chosen(code)).collect();
    // A value saved long ago may have left the list. It stays here to be unticked.
    for code in chosen {
        if !items.iter().any(|(known, _)| *known == code.as_str()) {
            ticked.push((code.as_str(), code.as_str()));
        }
    }
    let mut rest = Vec::new();
    let mut found = 0;
    for (code, name) in items.iter().copied().filter(|(code, _)| !is_chosen(code)) {
        if needle.is_empty() || name.to_lowercase().contains(&needle) || code.to_lowercase().contains(&needle) {
            found += 1;
            if rest.len() < limit {
                rest.push((code, name));
            }
        }
    }
    Shortlist { ticked, rest, found }
}

/// A searchable list of ticks: a search box, how many are chosen, the chosen on top.
pub fn pick_many<'a, M: Clone + 'a>(
    items: &[(&str, &str)],
    chosen: &[String],
    placeholder: &str,
    query: &str,
    on_query: impl Fn(String) -> M + 'a,
    on_toggle: fn(String) -> M,
    on_clear: M,
) -> El<'a, M> {
    let list = shortlist(items, chosen, query, PICK_LIMIT);
    let tally: El<'a, M> = if chosen.is_empty() {
        faint("Ничего не выбрано: подходят все").into()
    } else {
        row![muted(format!("Выбрано: {}", chosen.len())), link("Снять выбор", Some(on_clear))].spacing(gap::XS).align_y(Alignment::Center).into()
    };
    // As tall with a choice as without, so a first tick moves nothing.
    let mut block = column![input(placeholder, query).on_input(on_query), container(tally).center_y(PICK_TALLY)].spacing(gap::XS);
    if list.ticked.is_empty() && list.rest.is_empty() {
        return block.push(faint(if items.is_empty() { "Нет значений" } else { "Ничего не найдено" })).into();
    }
    let line = |(code, name): (&str, &str), ticked: bool| {
        let code = code.to_string();
        check(name, ticked, move |_| on_toggle(code.clone()))
    };
    let mut lines = Column::new().spacing(gap::XS);
    for item in &list.ticked {
        lines = lines.push(line(*item, true));
    }
    if !list.ticked.is_empty() && !list.rest.is_empty() {
        lines = lines.push(divider());
    }
    for item in &list.rest {
        lines = lines.push(line(*item, false));
    }
    block = block.push(container(scroll(container(lines).padding(gap::SM).width(Fill))).max_height(PICK_HEIGHT).width(Fill).style(theme::inset));
    if list.found > list.rest.len() {
        let shown = tbs_core::format::count(list.rest.len());
        block = block.push(faint(format!("Показаны первые {shown} из {}. Уточните поиск, чтобы найти остальные.", tbs_core::format::count(list.found))));
    }
    block.into()
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


#[cfg(test)]
mod tests {
    use super::*;

    const SHOPS: [(&str, &str); 5] = [("apt", "Аптека Подорожник"), ("bak", "Булочная"), ("cof", "Кофейня Арбуз"), ("mkt", "Ромашка Маркет"), ("tax", "Такси Комета")];

    fn codes(chosen: &[&str]) -> Vec<String> {
        chosen.iter().map(|code| code.to_string()).collect()
    }

    #[test]
    fn the_ticked_stand_first_whatever_the_search_says() {
        let chosen = codes(&["mkt", "bak"]);
        let list = shortlist(&SHOPS, &chosen, "", PICK_LIMIT);
        // In the order of the list, not of the ticking: a tick does not shuffle its neighbours.
        assert_eq!(list.ticked, [("bak", "Булочная"), ("mkt", "Ромашка Маркет")]);
        assert_eq!(list.rest, [("apt", "Аптека Подорожник"), ("cof", "Кофейня Арбуз"), ("tax", "Такси Комета")]);
        assert_eq!(list.found, 3);
        // The search narrows the rest. What is ticked stays, to be seen and unticked.
        let list = shortlist(&SHOPS, &chosen, "кофе", PICK_LIMIT);
        assert_eq!(list.ticked.len(), 2);
        assert_eq!(list.rest, [("cof", "Кофейня Арбуз")]);
        assert_eq!(list.found, 1);
        // Nothing found among the rest leaves the ticked ones alone.
        let list = shortlist(&SHOPS, &chosen, "нет такого", PICK_LIMIT);
        assert_eq!((list.ticked.len(), list.rest.len(), list.found), (2, 0, 0));
    }

    #[test]
    fn the_search_reads_names_and_codes_in_any_case() {
        let none: Vec<String> = Vec::new();
        let found = |query: &str| shortlist(&SHOPS, &none, query, PICK_LIMIT).rest.iter().map(|(code, _)| *code).collect::<Vec<_>>();
        assert_eq!(found("АРБУЗ"), ["cof"]);
        assert_eq!(found("  ромашка  "), ["mkt"]);
        assert_eq!(found("TAX"), ["tax"]);
        assert_eq!(found("а"), ["apt", "bak", "cof", "mkt", "tax"]);
        assert_eq!(found(""), ["apt", "bak", "cof", "mkt", "tax"]);
        assert_eq!(found("   "), ["apt", "bak", "cof", "mkt", "tax"]);
    }

    #[test]
    fn a_value_that_left_the_list_stays_to_be_unticked() {
        let chosen = codes(&["Закрытый магазин", "cof"]);
        let list = shortlist(&SHOPS, &chosen, "такси", PICK_LIMIT);
        assert_eq!(list.ticked, [("cof", "Кофейня Арбуз"), ("Закрытый магазин", "Закрытый магазин")]);
        assert_eq!(list.rest, [("tax", "Такси Комета")]);
    }

    #[test]
    fn a_long_list_is_cut_and_says_how_many_it_found() {
        let names: Vec<String> = (0..250).map(|i| format!("Поставщик № {i:03}")).collect();
        let items: Vec<(&str, &str)> = names.iter().map(|name| (name.as_str(), name.as_str())).collect();
        let chosen = codes(&["Поставщик № 240", "Поставщик № 007"]);
        let list = shortlist(&items, &chosen, "", PICK_LIMIT);
        // The ticked ones are all there, even from beyond the cut.
        assert_eq!(list.ticked.iter().map(|(code, _)| *code).collect::<Vec<_>>(), ["Поставщик № 007", "Поставщик № 240"]);
        assert_eq!((list.rest.len(), list.found), (PICK_LIMIT, 248));
        assert_eq!(list.rest[0].0, "Поставщик № 000");
        // A search that narrows it enough shows all it found.
        let list = shortlist(&items, &chosen, "№ 24", PICK_LIMIT);
        assert_eq!((list.rest.len(), list.found), (9, 9));
        assert!(!list.rest.iter().any(|(code, _)| *code == "Поставщик № 240"));
        // Nothing to pick from is not a search that failed.
        let empty = shortlist(&[], &[], "что-нибудь", PICK_LIMIT);
        assert_eq!((empty.ticked.len(), empty.rest.len(), empty.found), (0, 0, 0));
    }
}
