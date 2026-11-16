//! How the chats look: bubbles, avatars and the buttons under a message.

use iced::widget::{button, container, text_editor};
use iced::{Background, Border, Color, Shadow, Theme, Vector, border};

use crate::theme::{p, radius};

/// Eight fills for chats that have no picture, chosen by the chat's number.
const AVATARS: [Color; 8] = [
    Color::from_rgb8(0xE1, 0x70, 0x76),
    Color::from_rgb8(0xF2, 0x98, 0x4A),
    Color::from_rgb8(0xA6, 0x95, 0xE7),
    Color::from_rgb8(0x7B, 0xC8, 0x62),
    Color::from_rgb8(0x6E, 0xC9, 0xCB),
    Color::from_rgb8(0x65, 0xAA, 0xDD),
    Color::from_rgb8(0xEE, 0x7A, 0xAE),
    Color::from_rgb8(0x8C, 0x9A, 0xA9),
];

pub fn avatar(seed: i64) -> impl Fn(&Theme) -> container::Style {
    let fill = AVATARS[(seed.unsigned_abs() % 8) as usize];
    move |_| container::Style { background: Some(fill.into()), text_color: Some(Color::WHITE), border: border::rounded(radius::PILL), ..Default::default() }
}

pub fn list_pane(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style { background: Some(c.surface.into()), text_color: Some(c.text), ..Default::default() }
}

/// The bar over the messages and the one under them.
pub fn bar(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style { background: Some(c.surface.into()), text_color: Some(c.text), ..Default::default() }
}

pub fn bubble(outgoing: bool, selected: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let c = p(theme);
        let fill = if outgoing { c.accent_soft } else { c.surface };
        let edge = if selected { c.accent } else if outgoing { c.selection_edge } else { c.border };
        container::Style {
            background: Some(fill.into()),
            text_color: Some(c.text),
            border: Border { color: edge, width: if selected { 2.0 } else { 1.0 }, radius: radius::LG.into() },
            ..Default::default()
        }
    }
}

/// A row of the stream that is ticked in selection mode.
pub fn row_mark(selected: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let c = p(theme);
        container::Style { background: selected.then(|| c.selection.into()), border: border::rounded(radius::MD), ..Default::default() }
    }
}

pub fn day_mark(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style { background: Some(c.raised.into()), text_color: Some(c.text2), border: border::rounded(radius::PILL), ..Default::default() }
}

/// The square a file's glyph sits in.
pub fn file_mark(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style { background: Some(c.raised.into()), text_color: Some(c.text2), border: border::rounded(radius::MD), ..Default::default() }
}

pub fn photo_wait(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style { background: Some(c.raised.into()), text_color: Some(c.text3), border: border::rounded(radius::MD), ..Default::default() }
}

pub fn tick(checked: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let c = p(theme);
        container::Style {
            background: Some(if checked { c.accent } else { c.surface }.into()),
            text_color: Some(c.on_accent),
            border: Border { color: if checked { c.accent } else { c.control_edge }, width: 1.0, radius: radius::PILL.into() },
            ..Default::default()
        }
    }
}

fn flat(radius: f32) -> button::Style {
    button::Style { background: None, text_color: Color::BLACK, border: border::rounded(radius), shadow: Shadow::default(), snap: true }
}

/// A chat in the list: lit while it is open or under the pointer.
pub fn chat(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let c = p(theme);
        let fill = if active {
            Some(c.selection)
        } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            Some(c.hover)
        } else {
            None
        };
        button::Style { background: fill.map(Background::from), text_color: c.text, ..flat(radius::MD) }
    }
}

/// Something inside a bubble that is pressed as a whole: a photo, a file.
pub fn plain(theme: &Theme, status: button::Status) -> button::Style {
    let c = p(theme);
    let lit = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style { background: lit.then(|| c.hover.into()), text_color: c.text, ..flat(radius::MD) }
}

/// A button of a message's keyboard. One that opens a mini app is tinted.
pub fn key(app: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let c = p(theme);
        let (fill, text, edge) = match (status, app) {
            (button::Status::Disabled, _) => (c.raised, c.text3, c.border),
            (button::Status::Hovered | button::Status::Pressed, true) => (c.selection, c.accent, c.accent),
            (button::Status::Hovered | button::Status::Pressed, false) => (c.hover, c.text, c.border_strong),
            (_, true) => (c.accent_soft, c.accent, c.selection_edge),
            (_, false) => (c.surface, c.text, c.border_strong),
        };
        button::Style { background: Some(fill.into()), text_color: text, border: Border { color: edge, width: 1.0, radius: radius::MD.into() }, ..flat(radius::MD) }
    }
}

/// «Обработать» beside a message, and what it turns into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Work {
    Idle,
    Busy,
    Done,
    Failed,
}

pub fn work(state: Work) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let c = p(theme);
        let (text, edge, soft) = match state {
            Work::Idle | Work::Busy => (c.text2, c.border_strong, c.hover),
            Work::Done => (c.success, c.success, c.success_soft),
            Work::Failed => (c.danger, c.danger, c.danger_soft),
        };
        let lit = matches!(status, button::Status::Hovered | button::Status::Pressed) || state == Work::Done || state == Work::Failed;
        button::Style { background: lit.then(|| soft.into()), text_color: text, border: Border { color: edge, width: 1.0, radius: radius::PILL.into() }, ..flat(radius::PILL) }
    }
}

/// The round button that returns to the newest message.
pub fn fab(theme: &Theme, status: button::Status) -> button::Style {
    let c = p(theme);
    let fill = if matches!(status, button::Status::Hovered | button::Status::Pressed) { c.hover } else { c.surface };
    button::Style {
        background: Some(fill.into()),
        text_color: c.text,
        border: Border { color: c.border_strong, width: 1.0, radius: radius::PILL.into() },
        shadow: Shadow { color: c.shadow, offset: Vector::new(0.0, 4.0), blur_radius: 12.0 },
        snap: true,
    }
}

/// A mini app's own main button, in the colours the app asked for.
pub fn app_button(fill: Option<Color>, ink: Option<Color>) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let c = p(theme);
        let base = match status {
            button::Status::Disabled => c.pressed,
            button::Status::Hovered | button::Status::Pressed => fill.unwrap_or(c.accent_hover),
            button::Status::Active => fill.unwrap_or(c.accent),
        };
        let text = if status == button::Status::Disabled { c.text3 } else { ink.unwrap_or(c.on_accent) };
        button::Style { background: Some(base.into()), text_color: text, ..flat(radius::MD) }
    }
}

pub fn composer(theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let c = p(theme);
    let edge = match status {
        text_editor::Status::Focused { .. } => c.accent,
        text_editor::Status::Hovered => c.text3,
        _ => c.control_edge,
    };
    text_editor::Style {
        background: if matches!(status, text_editor::Status::Disabled) { c.raised.into() } else { c.surface.into() },
        border: Border { color: edge, width: 1.0, radius: radius::MD.into() },
        placeholder: c.text3,
        value: c.text,
        selection: c.selection,
    }
}

/// The frame of a mini app: square inside, so the page it holds meets the edge.
pub fn app_panel(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style {
        background: Some(c.surface.into()),
        text_color: Some(c.text),
        border: Border { color: c.border_strong, width: 1.0, radius: 0.0.into() },
        shadow: Shadow { color: c.shadow, offset: Vector::new(0.0, 12.0), blur_radius: 40.0 },
        ..Default::default()
    }
}

pub fn app_body(dark: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let c = p(theme);
        let (r, g, b, _) = super::host::background(dark);
        container::Style { background: Some(Color::from_rgb8(r, g, b).into()), text_color: Some(c.text2), ..Default::default() }
    }
}
