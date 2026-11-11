//! The reference section: operators, descriptions and the annotation dictionaries.

mod descriptions;
mod dictionary;
mod operators;

use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Stack, column, container, opaque, row};
use iced::{Alignment, Event, Fill, Subscription, Task, event, window};

use crate::ctx::Ctx;
use crate::theme::{self, space};
use crate::ui::{self, El};

use descriptions::Descriptions;
use dictionary::Dictionary;
use operators::Operators;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Operators,
    Descriptions,
    Annotations,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Section(Section),
    Escape,
    FocusSearch,
    Add,
    Operators(operators::Msg),
    Descriptions(descriptions::Msg),
    Dictionary(dictionary::Msg),
}

pub struct Reference {
    section: Section,
    operators: Operators,
    descriptions: Descriptions,
    dictionary: Dictionary,
    described: bool,
    annotated: bool,
}

fn hotkey(event: Event, _status: event::Status, _window: window::Id) -> Option<Msg> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, physical_key, .. }) = event else { return None };
    if key == Key::Named(Named::Escape) {
        return Some(Msg::Escape);
    }
    if !modifiers.command() {
        return None;
    }
    match key.to_latin(physical_key) {
        Some('k') => Some(Msg::FocusSearch),
        Some('n') => Some(Msg::Add),
        _ => None,
    }
}

impl Reference {
    pub fn new(ctx: &Ctx) -> (Self, Task<Msg>) {
        let operators = Operators::new(ctx);
        let load = operators.load_all(ctx).map(Msg::Operators);
        let screen = Self { section: Section::Operators, operators, descriptions: Descriptions::new(ctx), dictionary: Dictionary::new(ctx), described: false, annotated: false };
        (screen, load)
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        event::listen_with(hotkey)
    }

    #[cfg(feature = "devtools")]
    pub fn probe(name: &str) -> Option<Msg> {
        Some(match name {
            "descriptions" => Msg::Section(Section::Descriptions),
            "annotations" => Msg::Section(Section::Annotations),
            "op-add" => Msg::Add,
            _ => return None,
        })
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Section(section) => {
                self.section = section;
                match section {
                    Section::Descriptions if !self.described => {
                        self.described = true;
                        return self.descriptions.load_all(ctx).map(Msg::Descriptions);
                    }
                    Section::Annotations if !self.annotated => {
                        self.annotated = true;
                        return self.dictionary.load(ctx).map(Msg::Dictionary);
                    }
                    _ => {}
                }
            }
            Msg::Escape => match self.section {
                Section::Operators => return self.operators.update(operators::Msg::Escape, ctx).map(Msg::Operators),
                Section::Descriptions => return self.descriptions.update(descriptions::Msg::Escape, ctx).map(Msg::Descriptions),
                Section::Annotations => return self.dictionary.update(dictionary::Msg::Escape, ctx).map(Msg::Dictionary),
            },
            Msg::FocusSearch => {
                return match self.section {
                    Section::Operators => operators::focus_search(),
                    Section::Descriptions => descriptions::focus_search(),
                    Section::Annotations => dictionary::focus_search(),
                };
            }
            Msg::Add => match self.section {
                Section::Operators => return self.operators.update(operators::Msg::OpenAdd, ctx).map(Msg::Operators),
                Section::Descriptions => return self.descriptions.update(descriptions::Msg::OpenAdd, ctx).map(Msg::Descriptions),
                Section::Annotations => return self.dictionary.update(dictionary::Msg::OpenAdd, ctx).map(Msg::Dictionary),
            },
            Msg::Operators(inner) => return self.operators.update(inner, ctx).map(Msg::Operators),
            Msg::Descriptions(inner) => return self.descriptions.update(inner, ctx).map(Msg::Descriptions),
            Msg::Dictionary(inner) => return self.dictionary.update(inner, ctx).map(Msg::Dictionary),
        }
        Task::none()
    }

    fn tabs(&self) -> El<'_, Msg> {
        let operators = match self.operators.total_records() {
            Some(count) => format!("Операторы · {}", tbs_core::format::count(count)),
            None => "Операторы".to_owned(),
        };
        let tab = |caption: String, section: Section| ui::chip(caption, self.section == section, Msg::Section(section));
        row![tab(operators, Section::Operators), tab("Описания".to_owned(), Section::Descriptions), tab("Разметка".to_owned(), Section::Annotations)]
            .spacing(space::XS)
            .align_y(Alignment::Center)
            .into()
    }

    pub fn view(&self, ctx: &Ctx) -> El<'_, Msg> {
        let body = match self.section {
            Section::Operators => self.operators.view(ctx).map(Msg::Operators),
            Section::Descriptions => self.descriptions.view(ctx).map(Msg::Descriptions),
            Section::Annotations => self.dictionary.view(ctx).map(Msg::Dictionary),
        };
        let page = column![self.tabs(), body].spacing(space::MD);
        container(page).padding(ui::PAGE_PADDING).width(Fill).height(Fill).style(theme::page).into()
    }

    pub fn overlay(&self, ctx: &Ctx) -> Option<El<'_, Msg>> {
        let layers = match self.section {
            Section::Operators => self.operators.overlay(ctx).into_iter().map(|e| e.map(Msg::Operators)).collect::<Vec<_>>(),
            Section::Descriptions => self.descriptions.overlay(ctx).into_iter().map(|e| e.map(Msg::Descriptions)).collect(),
            Section::Annotations => self.dictionary.overlay(ctx).into_iter().map(|e| e.map(Msg::Dictionary)).collect(),
        };
        if layers.is_empty() {
            return None;
        }
        Some(opaque(Stack::with_children(layers).width(Fill).height(Fill)))
    }
}
