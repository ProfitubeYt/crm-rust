#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod ctx;
#[cfg(feature = "devtools")]
mod devtools;
mod files;
mod focus;
mod grid;
mod icon_set;
mod screens;
mod theme;
mod ui;

fn main() -> iced::Result {
    app::run()
}
