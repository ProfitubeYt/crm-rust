//! Test hooks. Compiled only with `--features devtools`; a shipped client has
//! none of this.
//!
//! - `TBS_AUTOLOGIN=<json with username, password>` signs in by itself.
//! - `TBS_GOTO=<route>` opens that section.
//! - `TBS_OPEN=<panel>` opens a panel of the table before the screenshot.
//!   In Telegram: `tg-chat:<id>` opens that chat, `tg-select` the selection
//!   mode, `tg-period` the period menu, `tg-all` the tab of all chats,
//!   `tg-app` the mini app on the open bot's menu button.
//! - `TBS_SHOT=<png>` saves a screenshot once data is on screen, then quits.
//! - `TBS_CMD=<file>` runs the lines put in that file, one per tick, through the
//!   app's own update: `goto <route>`, `open <panel>`, `theme <dark|light>`,
//!   `toast <text>`, `hub`, `logout`. It reaches the user's desktop in no way.

use std::time::Duration;

use iced::{Subscription, Task, window};

use crate::app::{App, Message, Route};

#[derive(Clone, Debug)]
pub enum Msg {
    Tick,
    Shot(window::Screenshot),
}

#[derive(Default)]
pub struct Dev {
    login: Option<(String, String)>,
    goto: Option<String>,
    open: Option<String>,
    shot: Option<String>,
    commands: Option<String>,
    ready_ticks: u32,
    taken: bool,
    logged: bool,
}

impl Dev {
    pub fn from_env() -> Self {
        let login = std::env::var("TBS_AUTOLOGIN").ok().and_then(|path| {
            let text = std::fs::read_to_string(path).ok()?;
            let value: serde_json::Value = serde_json::from_str(&text).ok()?;
            Some((value["username"].as_str()?.to_owned(), value["password"].as_str()?.to_owned()))
        });
        Self { login, goto: std::env::var("TBS_GOTO").ok(), open: std::env::var("TBS_OPEN").ok(), shot: std::env::var("TBS_SHOT").ok(), commands: std::env::var("TBS_CMD").ok(), ..Default::default() }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_millis(500)).map(|_| Message::Dev(Msg::Tick))
    }
}

fn route(name: &str) -> Option<Route> {
    Some(match name {
        "transactions" => Route::Transactions,
        "reference" => Route::Reference,
        "telegram" => Route::Telegram,
        "logs" => Route::Logs,
        "settings" => Route::Settings,
        "audit" => Route::Audit,
        _ => return None,
    })
}

fn probe(name: &str) -> Option<Message> {
    use crate::screens::{audit::Audit, login::Login, logs::Logs, reference::Reference, settings::Settings, telegram::Telegram, transactions::Transactions};
    Login::probe(name)
        .map(Message::Login)
        .or_else(|| Transactions::probe(name).map(Message::Tx))
        .or_else(|| Reference::probe(name).map(Message::Reference))
        .or_else(|| Logs::probe(name).map(Message::Logs))
        .or_else(|| Audit::probe(name).map(Message::Audit))
        .or_else(|| Settings::probe(name).map(Message::Settings))
        .or_else(|| Telegram::probe(name).map(Message::Telegram))
}

/// Takes the first line out of the command file.
fn next_command(path: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let (line, rest) = text.split_once('\n').unwrap_or((text.as_str(), ""));
    let line = line.trim().to_owned();
    if line.is_empty() {
        return None;
    }
    std::fs::write(path, rest).ok()?;
    Some(line)
}

fn run_command(app: &mut App, line: &str) -> Task<Message> {
    let (verb, value) = line.split_once(' ').unwrap_or((line, ""));
    match verb {
        "goto" => route(value).map_or_else(Task::none, |target| app.go_to(target)),
        "open" => probe(value).map_or_else(Task::none, Task::done),
        "hub" => Task::done(Message::Hub),
        "logout" => Task::done(Message::Logout),
        "theme" => {
            app.ctx_for_test().prefs.theme = Some(value.to_owned());
            app.ctx_for_test().request(crate::ctx::Request::ThemeChanged);
            Task::none()
        }
        "toast" => {
            app.ctx_for_test().toast(crate::theme::Tone::Success, value);
            Task::none()
        }
        _ => {
            eprintln!("unknown command: {line}");
            Task::none()
        }
    }
}

pub fn update(app: &mut App, message: Msg) -> Task<Message> {
    match message {
        Msg::Tick => {
            if !app.dev.logged
                && let Some((username, password)) = app.dev.login.clone()
                && let Some(task) = app.sign_in_for_test(username, password)
            {
                app.dev.logged = true;
                return task;
            }
            // The start window has no data to wait for, and nothing to go to.
            if app.on_login()
                && let Some(line) = app.dev.commands.as_deref().and_then(next_command)
            {
                return run_command(app, &line);
            }
            if app.is_ready() {
                if let Some(target) = app.dev.goto.take().as_deref().and_then(route) {
                    return app.go_to(target);
                }
                if let Some(line) = app.dev.commands.as_deref().and_then(next_command) {
                    return run_command(app, &line);
                }
                app.dev.ready_ticks += 1;
                if app.dev.ready_ticks == 2
                    && let Some(name) = app.dev.open.take()
                    && let Some(message) = probe(&name)
                {
                    return Task::done(message);
                }
                if app.dev.shot.is_some() && !app.dev.taken && app.dev.ready_ticks >= 5 {
                    app.dev.taken = true;
                    return window::latest().and_then(window::screenshot).map(|shot| Message::Dev(Msg::Shot(shot)));
                }
            }
        }
        Msg::Shot(shot) => {
            if let Some(path) = &app.dev.shot {
                if let Err(error) = save_png(path, &shot) {
                    eprintln!("screenshot failed: {error}");
                }
            }
            return iced::exit();
        }
    }
    Task::none()
}

fn save_png(path: &str, shot: &window::Screenshot) -> anyhow::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, shot.size.width, shot.size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&shot.rgba)?;
    Ok(())
}
