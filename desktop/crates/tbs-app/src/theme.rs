//! Design tokens and the style functions built on them.
//!
//! Every colour on screen comes from a [`Palette`]. Widgets read it through
//! [`p`], so a theme switch needs nothing but a new `iced::Theme`.

use iced::widget::{button, checkbox, container, pick_list, progress_bar, rule, scrollable, text_input, toggler};
use iced::{Background, Border, Color, Font, Shadow, Theme, Vector, border, color, font};

pub mod size {
    pub const XS: f32 = 11.0;
    pub const SM: f32 = 12.0;
    pub const BASE: f32 = 13.0;
    pub const MD: f32 = 14.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 20.0;
    pub const XXL: f32 = 26.0;
}

pub mod space {
    pub const XXS: f32 = 2.0;
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
}

pub mod radius {
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 6.0;
    pub const LG: f32 = 10.0;
    pub const XL: f32 = 14.0;
    pub const PILL: f32 = 999.0;
}

/// Control heights keep toolbars aligned without per-screen tuning.
pub mod control {
    pub const SM: f32 = 28.0;
    pub const MD: f32 = 32.0;
    /// The sign-in window: one form with room to spare, not a toolbar.
    pub const LG: f32 = 38.0;
}

pub const FONT: Font = Font::with_name("Segoe UI");
pub const FONT_MEDIUM: Font = Font { weight: font::Weight::Medium, ..FONT };
pub const FONT_BOLD: Font = Font { weight: font::Weight::Semibold, ..FONT };
pub const FONT_MONO: Font = Font::with_name("Consolas");

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub bg: Color,
    /// The brand half of the sign-in window: the navy of the TBS inform logo.
    pub pane: Color,
    /// Text and marks on that navy, and the quieter of the two.
    pub on_pane: Color,
    pub on_pane2: Color,
    pub surface: Color,
    pub raised: Color,
    pub hover: Color,
    pub pressed: Color,
    pub border: Color,
    pub border_strong: Color,
    /// The outline of a field or a checkbox, which is how one is recognised at
    /// all, so it holds 3:1 against the surface. Cards keep the quieter borders.
    pub control_edge: Color,
    pub text: Color,
    pub text2: Color,
    pub text3: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_soft: Color,
    pub on_accent: Color,
    pub success: Color,
    pub success_soft: Color,
    pub warning: Color,
    pub warning_soft: Color,
    pub danger: Color,
    pub danger_soft: Color,
    pub row_alt: Color,
    pub row_hover: Color,
    pub selection: Color,
    pub selection_edge: Color,
    pub scrim: Color,
    pub shadow: Color,
}

pub const LIGHT: Palette = Palette {
    bg: color!(0xF5F6F8),
    pane: color!(0x243C72),
    on_pane: color!(0xFFFFFF),
    on_pane2: color!(0xC3CEE6),
    surface: color!(0xFFFFFF),
    raised: color!(0xF0F2F5),
    hover: color!(0xE9ECF1),
    pressed: color!(0xDFE3EA),
    border: color!(0xE1E5EB),
    border_strong: color!(0xC6CDD7),
    control_edge: color!(0x7F8A99),
    text: color!(0x141A23),
    text2: color!(0x4A5565),
    text3: color!(0x5C6776),
    accent: color!(0x2458D3),
    accent_hover: color!(0x1D49B3),
    accent_soft: color!(0xE7EDFC),
    on_accent: color!(0xFFFFFF),
    success: color!(0x0F7A57),
    success_soft: color!(0xE1F3EC),
    warning: color!(0x9A5B08),
    warning_soft: color!(0xFBF0DC),
    danger: color!(0xC12F2C),
    danger_soft: color!(0xFBE8E7),
    row_alt: color!(0xFAFBFC),
    row_hover: color!(0xEFF3F9),
    selection: color!(0xDCE6FB),
    selection_edge: color!(0x2458D3),
    scrim: Color { r: 0.07, g: 0.09, b: 0.13, a: 0.42 },
    shadow: Color { r: 0.07, g: 0.09, b: 0.13, a: 0.16 },
};

pub const DARK: Palette = Palette {
    bg: color!(0x0F1216),
    pane: color!(0x1A2B52),
    on_pane: color!(0xFFFFFF),
    on_pane2: color!(0xC3CEE6),
    surface: color!(0x161A20),
    raised: color!(0x1D2229),
    hover: color!(0x252B34),
    pressed: color!(0x2D343F),
    border: color!(0x2A313B),
    border_strong: color!(0x3C4552),
    control_edge: color!(0x67727F),
    text: color!(0xE8ECF1),
    text2: color!(0xAAB4C1),
    text3: color!(0x8792A0),
    accent: color!(0x7198FF),
    accent_hover: color!(0x8DADFF),
    accent_soft: color!(0x1E2A45),
    on_accent: color!(0x0D1117),
    success: color!(0x45C49A),
    success_soft: color!(0x13302A),
    warning: color!(0xE5A94F),
    warning_soft: color!(0x33280F),
    danger: color!(0xF2756F),
    danger_soft: color!(0x3A1D1E),
    row_alt: color!(0x13171C),
    row_hover: color!(0x1E242C),
    selection: color!(0x22345A),
    selection_edge: color!(0x7198FF),
    scrim: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.55 },
    shadow: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.45 },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
}

impl Palette {
    pub fn tone(&self, tone: Tone) -> (Color, Color) {
        match tone {
            Tone::Neutral => (self.text2, self.raised),
            Tone::Accent => (self.accent, self.accent_soft),
            Tone::Success => (self.success, self.success_soft),
            Tone::Warning => (self.warning, self.warning_soft),
            Tone::Danger => (self.danger, self.danger_soft),
        }
    }
}

pub fn p(theme: &Theme) -> &'static Palette {
    if theme.extended_palette().is_dark { &DARK } else { &LIGHT }
}

pub fn iced_theme(dark: bool) -> Theme {
    let tokens = if dark { &DARK } else { &LIGHT };
    Theme::custom(
        if dark { "TBS тёмная" } else { "TBS светлая" },
        iced::theme::Palette {
            background: tokens.bg,
            text: tokens.text,
            primary: tokens.accent,
            success: tokens.success,
            warning: tokens.warning,
            danger: tokens.danger,
        },
    )
}

fn soft_shadow(tokens: &Palette, y: f32, blur: f32) -> Shadow {
    Shadow { color: tokens.shadow, offset: Vector::new(0.0, y), blur_radius: blur }
}

// ---- containers ----------------------------------------------------------

pub fn page(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style { background: Some(c.bg.into()), text_color: Some(c.text), ..Default::default() }
}

pub fn surface(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style { background: Some(c.surface.into()), text_color: Some(c.text), ..Default::default() }
}

pub fn card(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style {
        background: Some(c.surface.into()),
        text_color: Some(c.text),
        border: Border { color: c.border, width: 1.0, radius: radius::LG.into() },
        ..Default::default()
    }
}

pub fn inset(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style {
        background: Some(c.raised.into()),
        text_color: Some(c.text),
        border: border::rounded(radius::MD),
        ..Default::default()
    }
}

/// A group of a sheet that folds. One that holds something is edged in the
/// accent, so it is found without opening it.
pub fn group(marked: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let c = p(theme);
        container::Style {
            background: Some(c.surface.into()),
            text_color: Some(c.text),
            border: Border { color: if marked { c.accent } else { c.border }, width: 1.0, radius: radius::LG.into() },
            ..Default::default()
        }
    }
}

pub fn dialog(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style {
        background: Some(c.surface.into()),
        text_color: Some(c.text),
        border: Border { color: c.border, width: 1.0, radius: radius::XL.into() },
        shadow: soft_shadow(c, 12.0, 40.0),
        ..Default::default()
    }
}

pub fn popover(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style {
        background: Some(c.surface.into()),
        text_color: Some(c.text),
        border: Border { color: c.border_strong, width: 1.0, radius: radius::LG.into() },
        shadow: soft_shadow(c, 6.0, 20.0),
        ..Default::default()
    }
}

/// The brand half of the sign-in window, out to the window's own edges.
pub fn sign_in_pane(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style { background: Some(c.pane.into()), text_color: Some(c.on_pane), ..Default::default() }
}

/// The pane's own ink, thinned: fills and lines on the navy are steps of it.
fn pane_ink(theme: &Theme, alpha: f32) -> Color {
    Color { a: alpha, ..p(theme).on_pane }
}

/// The square behind a workspace's glyph.
pub fn pane_tile(theme: &Theme) -> container::Style {
    container::Style { background: Some(pane_ink(theme, 0.14).into()), border: border::rounded(radius::MD + 2.0), ..Default::default() }
}

/// The hairline between the logo and the product's name.
pub fn pane_line(theme: &Theme) -> container::Style {
    container::Style { background: Some(pane_ink(theme, 0.4).into()), ..Default::default() }
}

/// The server's state on the navy, where the usual tones are too dark to read.
pub fn pane_dot(tone: Tone) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let fill = if tone == Tone::Neutral { p(theme).on_pane2 } else { DARK.tone(tone).0 };
        container::Style { background: Some(fill.into()), border: border::rounded(radius::PILL), ..Default::default() }
    }
}

pub fn scrim(theme: &Theme) -> container::Style {
    container::Style { background: Some(p(theme).scrim.into()), ..Default::default() }
}

pub fn rail(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style {
        background: Some(c.surface.into()),
        text_color: Some(c.text),
        border: Border { color: c.border, width: 0.0, radius: 0.0.into() },
        ..Default::default()
    }
}

pub fn tinted(tone: Tone) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let (fg, bg) = p(theme).tone(tone);
        container::Style {
            background: Some(bg.into()),
            text_color: Some(fg),
            border: border::rounded(radius::MD),
            ..Default::default()
        }
    }
}

pub fn pill(tone: Tone) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let (fg, bg) = p(theme).tone(tone);
        container::Style {
            background: Some(bg.into()),
            text_color: Some(fg),
            border: border::rounded(radius::PILL),
            ..Default::default()
        }
    }
}

pub fn toast(tone: Tone) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let c = p(theme);
        let (fg, _) = c.tone(tone);
        container::Style {
            background: Some(c.surface.into()),
            text_color: Some(c.text),
            border: Border { color: fg, width: 1.0, radius: radius::LG.into() },
            shadow: soft_shadow(c, 6.0, 22.0),
            ..Default::default()
        }
    }
}

pub fn dot(tone: Tone) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let (fg, _) = p(theme).tone(tone);
        container::Style { background: Some(fg.into()), border: border::rounded(radius::PILL), ..Default::default() }
    }
}

pub fn swatch(fill: Color) -> impl Fn(&Theme) -> container::Style {
    move |theme| container::Style {
        background: Some(fill.into()),
        border: Border { color: p(theme).border_strong, width: 1.0, radius: radius::SM.into() },
        ..Default::default()
    }
}

// ---- buttons -------------------------------------------------------------

fn button_base(radius: f32) -> button::Style {
    button::Style {
        background: None,
        text_color: Color::BLACK,
        border: border::rounded(radius),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn btn_primary(theme: &Theme, status: button::Status) -> button::Style {
    let c = p(theme);
    let fill = match status {
        button::Status::Hovered | button::Status::Pressed => c.accent_hover,
        button::Status::Disabled => c.pressed,
        button::Status::Active => c.accent,
    };
    button::Style {
        background: Some(fill.into()),
        text_color: if status == button::Status::Disabled { c.text3 } else { c.on_accent },
        ..button_base(radius::MD)
    }
}

pub fn btn_secondary(theme: &Theme, status: button::Status) -> button::Style {
    let c = p(theme);
    let fill = match status {
        button::Status::Hovered => c.hover,
        button::Status::Pressed => c.pressed,
        _ => c.surface,
    };
    button::Style {
        background: Some(fill.into()),
        text_color: if status == button::Status::Disabled { c.text3 } else { c.text },
        border: Border { color: c.border_strong, width: 1.0, radius: radius::MD.into() },
        ..button_base(radius::MD)
    }
}

pub fn btn_ghost(theme: &Theme, status: button::Status) -> button::Style {
    let c = p(theme);
    let fill = match status {
        button::Status::Hovered => Some(c.hover),
        button::Status::Pressed => Some(c.pressed),
        _ => None,
    };
    button::Style {
        background: fill.map(Background::from),
        text_color: if status == button::Status::Disabled { c.text3 } else { c.text2 },
        ..button_base(radius::MD)
    }
}

pub fn btn_danger(theme: &Theme, status: button::Status) -> button::Style {
    let c = p(theme);
    let fill = match status {
        button::Status::Hovered | button::Status::Pressed => c.danger,
        button::Status::Disabled => c.raised,
        button::Status::Active => c.danger_soft,
    };
    let text = match status {
        button::Status::Hovered | button::Status::Pressed => c.on_accent,
        button::Status::Disabled => c.text3,
        button::Status::Active => c.danger,
    };
    button::Style { background: Some(fill.into()), text_color: text, ..button_base(radius::MD) }
}

/// Navigation rail items and selectable rows.
pub fn btn_nav(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let c = p(theme);
        let (fill, text) = if active {
            (Some(c.accent_soft), c.accent)
        } else {
            match status {
                button::Status::Hovered => (Some(c.hover), c.text),
                button::Status::Pressed => (Some(c.pressed), c.text),
                _ => (None, c.text2),
            }
        };
        button::Style { background: fill.map(Background::from), text_color: text, ..button_base(radius::MD) }
    }
}

/// A workspace in the sign-in window: the chosen one is lit on the navy.
pub fn btn_system(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let fill = match (active, status) {
            (true, _) => Some(0.12),
            (false, button::Status::Hovered) => Some(0.07),
            (false, button::Status::Pressed) => Some(0.1),
            _ => None,
        };
        button::Style {
            background: fill.map(|alpha| pane_ink(theme, alpha).into()),
            text_color: p(theme).on_pane,
            border: Border { color: if active { pane_ink(theme, 0.3) } else { Color::TRANSPARENT }, width: 1.0, radius: radius::LG.into() },
            ..button_base(radius::LG)
        }
    }
}

/// A quiet button on the navy pane.
pub fn btn_pane(theme: &Theme, status: button::Status) -> button::Style {
    let fill = match status {
        button::Status::Hovered => Some(0.1),
        button::Status::Pressed => Some(0.16),
        _ => None,
    };
    button::Style {
        background: fill.map(|alpha| pane_ink(theme, alpha).into()),
        text_color: p(theme).on_pane,
        ..button_base(radius::MD)
    }
}

/// A line of a list that keyboard arrows walk: the marked line stays lit.
pub fn btn_option(marked: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let c = p(theme);
        let lit = marked || matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style { background: lit.then(|| c.hover.into()), text_color: c.text, ..button_base(radius::SM) }
    }
}

/// A segment of a segmented control.
/// A filter chip: a chosen one is tinted so it reads at a glance.
pub fn btn_chip(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let c = p(theme);
        let (fill, text, edge) = if active {
            (Some(c.accent_soft), c.accent, c.accent)
        } else {
            match status {
                button::Status::Hovered => (Some(c.hover), c.text, c.border_strong),
                _ => (None, c.text2, c.border),
            }
        };
        let mut style = button::Style { background: fill.map(Background::from), text_color: text, ..button_base(radius::SM) };
        style.border = Border { color: edge, width: 1.0, radius: radius::SM.into() };
        style
    }
}

/// A filter that is on, above the table: a chosen chip with a cross of its own.
pub fn tag(theme: &Theme) -> container::Style {
    let c = p(theme);
    container::Style {
        background: Some(c.accent_soft.into()),
        text_color: Some(c.accent),
        border: Border { color: c.accent, width: 1.0, radius: radius::SM.into() },
        ..Default::default()
    }
}

/// Either half of a tag: its words or its cross.
pub fn btn_tag(theme: &Theme, status: button::Status) -> button::Style {
    let c = p(theme);
    let lit = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style { background: lit.then(|| c.selection.into()), text_color: c.accent, ..button_base(radius::SM) }
}

pub fn btn_segment(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let c = p(theme);
        let (fill, text) = if active {
            (Some(c.surface), c.text)
        } else {
            match status {
                button::Status::Hovered => (Some(c.hover), c.text),
                _ => (None, c.text2),
            }
        };
        let mut style = button::Style {
            background: fill.map(Background::from),
            text_color: text,
            ..button_base(radius::SM)
        };
        if active {
            style.shadow = soft_shadow(c, 1.0, 3.0);
        }
        style
    }
}

/// A menu line: flat until hovered, full width.
pub fn btn_menu(danger: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let c = p(theme);
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: hovered.then(|| if danger { c.danger_soft } else { c.hover }.into()),
            text_color: match (danger, status) {
                (_, button::Status::Disabled) => c.text3,
                (true, _) => c.danger,
                (false, _) => c.text,
            },
            ..button_base(radius::SM)
        }
    }
}

pub fn btn_link(theme: &Theme, status: button::Status) -> button::Style {
    let c = p(theme);
    button::Style {
        text_color: if status == button::Status::Hovered { c.accent_hover } else { c.accent },
        ..button_base(radius::SM)
    }
}

// ---- inputs --------------------------------------------------------------

pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let c = p(theme);
    let edge = match status {
        text_input::Status::Focused { .. } => c.accent,
        text_input::Status::Hovered => c.text3,
        _ => c.control_edge,
    };
    text_input::Style {
        background: if matches!(status, text_input::Status::Disabled) { c.raised.into() } else { c.surface.into() },
        border: Border { color: edge, width: 1.0, radius: radius::MD.into() },
        icon: c.text3,
        placeholder: c.text3,
        value: c.text,
        selection: c.selection,
    }
}

pub fn input_error(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let c = p(theme);
    text_input::Style {
        border: Border { color: c.danger, width: 1.0, radius: radius::MD.into() },
        ..input(theme, status)
    }
}

/// The in-table editor: sits exactly on the cell, so it has no rounding.
pub fn input_cell(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let c = p(theme);
    text_input::Style {
        border: Border { color: c.accent, width: 2.0, radius: 0.0.into() },
        ..input(theme, status)
    }
}

pub fn select(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let c = p(theme);
    let edge = match status {
        pick_list::Status::Opened { .. } => c.accent,
        pick_list::Status::Hovered => c.text3,
        pick_list::Status::Active => c.control_edge,
    };
    pick_list::Style {
        text_color: c.text,
        placeholder_color: c.text3,
        handle_color: c.text3,
        background: c.surface.into(),
        border: Border { color: edge, width: 1.0, radius: radius::MD.into() },
    }
}

pub fn select_menu(theme: &Theme) -> iced::overlay::menu::Style {
    let c = p(theme);
    iced::overlay::menu::Style {
        background: c.surface.into(),
        border: Border { color: c.border_strong, width: 1.0, radius: radius::MD.into() },
        text_color: c.text,
        selected_text_color: c.on_accent,
        selected_background: c.accent.into(),
        shadow: soft_shadow(c, 6.0, 20.0),
    }
}

pub fn check(theme: &Theme, status: checkbox::Status) -> checkbox::Style {
    let c = p(theme);
    let (checked, hovered, disabled) = match status {
        checkbox::Status::Active { is_checked } => (is_checked, false, false),
        checkbox::Status::Hovered { is_checked } => (is_checked, true, false),
        checkbox::Status::Disabled { is_checked } => (is_checked, false, true),
    };
    checkbox::Style {
        background: if checked && !disabled { c.accent.into() } else { c.surface.into() },
        icon_color: if disabled { c.text3 } else { c.on_accent },
        border: Border {
            color: if checked && !disabled {
                c.accent
            } else if hovered {
                c.text3
            } else {
                c.control_edge
            },
            width: 1.0,
            radius: radius::SM.into(),
        },
        text_color: Some(if disabled { c.text3 } else { c.text }),
    }
}

pub fn switch(theme: &Theme, status: toggler::Status) -> toggler::Style {
    let c = p(theme);
    let on = matches!(
        status,
        toggler::Status::Active { is_toggled: true }
            | toggler::Status::Hovered { is_toggled: true }
            | toggler::Status::Disabled { is_toggled: true }
    );
    toggler::Style {
        background: if on { c.accent.into() } else { c.control_edge.into() },
        background_border_width: 0.0,
        background_border_color: Color::TRANSPARENT,
        foreground: c.surface.into(),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        text_color: Some(c.text),
        border_radius: None,
        padding_ratio: 0.12,
    }
}

pub fn scroller(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let c = p(theme);
    let hot = matches!(status, scrollable::Status::Hovered { .. } | scrollable::Status::Dragged { .. });
    let rail = scrollable::Rail {
        background: None,
        border: border::rounded(radius::PILL),
        scroller: scrollable::Scroller {
            background: if hot { c.text3.into() } else { c.border_strong.into() },
            border: border::rounded(radius::PILL),
        },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: c.surface.into(),
            border: border::rounded(radius::PILL),
            shadow: Shadow::default(),
            icon: c.text2,
        },
    }
}

pub fn divider(theme: &Theme) -> rule::Style {
    rule::Style {
        color: p(theme).border,
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

pub fn progress(theme: &Theme) -> progress_bar::Style {
    let c = p(theme);
    progress_bar::Style {
        background: c.raised.into(),
        bar: c.accent.into(),
        border: border::rounded(radius::PILL),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Color) -> f32 {
        let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn body_text_meets_aa_on_every_surface() {
        for tokens in [&LIGHT, &DARK] {
            for surface in [tokens.bg, tokens.surface, tokens.raised, tokens.row_alt, tokens.row_hover] {
                assert!(contrast(tokens.text, surface) >= 7.0);
                assert!(contrast(tokens.text2, surface) >= 4.5);
            }
            assert!(contrast(tokens.text, tokens.selection) >= 4.5);
            assert!(contrast(tokens.text3, tokens.surface) >= 4.5);
            assert!(contrast(tokens.on_pane, tokens.pane) >= 7.0);
            assert!(contrast(tokens.on_pane2, tokens.pane) >= 4.5);
        }
    }

    #[test]
    fn tones_stay_readable_on_their_soft_fill() {
        for tokens in [&LIGHT, &DARK] {
            for tone in [Tone::Accent, Tone::Success, Tone::Warning, Tone::Danger] {
                let (fg, bg) = tokens.tone(tone);
                assert!(contrast(fg, bg) >= 4.5, "{tone:?}");
            }
            assert!(contrast(tokens.on_accent, tokens.accent) >= 4.5);
        }
    }

    #[test]
    fn theme_lookup_follows_the_iced_theme() {
        assert_eq!(p(&iced_theme(true)), &DARK);
        assert_eq!(p(&iced_theme(false)), &LIGHT);
    }
}
