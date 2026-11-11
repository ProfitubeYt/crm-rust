//! Drives the real screens against a local server: every step is a click or a
//! keystroke on the same view the window shows, and the outcome is read back
//! from the server. Writes, so it runs only with `TBS_E2E_WRITES=1` and
//! `TBS_E2E_LOGIN`, and refuses any address that is not this computer.
//!
//! Needs `ICED_TEST_BACKEND=tiny-skia`. Subscriptions do not run here, so
//! window-wide hotkeys are out of scope; anything the grid handles itself is in.

use std::sync::{Arc as Shared, Mutex};
use std::time::{Duration, Instant};

use iced::futures::StreamExt;
use iced::futures::stream::{BoxStream, SelectAll};
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::{Event, Point, Rectangle, Settings, Size, Vector, mouse};
use iced_runtime::Action;
use iced_test::Simulator;
use iced_test::selector::Candidate;
use iced_test::simulator::{click, press_key};
use serde_json::Value;
use tbs_core::columns::Column;
use tbs_core::secret::MemoryStore;

use super::*;
use crate::screens::transactions;

const SCREEN: Size = Size::new(2800.0, 1000.0);
const PATIENCE: Duration = Duration::from_secs(25);
/// Room a button takes at the right of a field.
const BUTTON_IN_FIELD: f32 = 44.0;

type Sim<'a> = Simulator<'a, Message>;

struct Creds {
    url: String,
    username: String,
    password: String,
}

fn creds() -> Option<Creds> {
    if std::env::var("TBS_E2E_WRITES").ok()?.as_str() != "1" {
        return None;
    }
    let path = std::env::var("TBS_E2E_LOGIN").ok()?;
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let url = std::env::var("TBS_E2E_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".into());
    assert!(url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost"), "writes run on a local server only, not {url}");
    Some(Creds { url, username: v["username"].as_str()?.into(), password: v["password"].as_str()?.into() })
}

fn stamp() -> String {
    Utc::now().format("%H%M%S%3f").to_string()
}

fn with_modifiers(event: Event, modifiers: Modifiers) -> Event {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed { key, modified_key, physical_key, location, repeat, text, .. }) => {
            Event::Keyboard(keyboard::Event::KeyPressed { key, modified_key, physical_key, location, modifiers, repeat, text })
        }
        other => other,
    }
}

fn ctrl(letter: &str) -> Event {
    with_modifiers(press_key(Key::Character(letter.into()), None), Modifiers::CTRL)
}

/// Every rectangle `pick` accepts, in drawing order, so the last one is on top.
fn collect(sim: &mut Sim<'_>, pick: impl Fn(&Candidate<'_>) -> Option<Rectangle> + Send) -> Vec<Rectangle> {
    let found = Shared::new(Mutex::new(Vec::new()));
    let sink = found.clone();
    let _ = sim.find(move |candidate: Candidate<'_>| -> Option<()> {
        if let Some(rect) = pick(&candidate) {
            sink.lock().unwrap().push(rect);
        }
        None
    });
    let rects = found.lock().unwrap().clone();
    rects
}

fn texts(sim: &mut Sim<'_>, label: &str) -> Vec<Rectangle> {
    let label = label.to_owned();
    collect(sim, move |candidate| match candidate {
        Candidate::Text { content, visible_bounds: Some(rect), .. } if *content == label => Some(*rect),
        _ => None,
    })
}

fn inputs(sim: &mut Sim<'_>) -> Vec<Rectangle> {
    collect(sim, |candidate| match candidate {
        Candidate::TextInput { visible_bounds: Some(rect), .. } => Some(*rect),
        _ => None,
    })
}

fn press_at(sim: &mut Sim<'_>, at: Point) {
    sim.point_at(at);
    sim.simulate(click());
}

fn right_press_at(sim: &mut Sim<'_>, at: Point) {
    sim.point_at(at);
    let button = mouse::Button::Right;
    sim.simulate([Event::Mouse(mouse::Event::ButtonPressed(button)), Event::Mouse(mouse::Event::ButtonReleased(button))]);
}

fn grid_bounds(sim: &mut Sim<'_>) -> Rectangle {
    let found = collect(sim, |candidate| match candidate {
        Candidate::Container { id: Some(id), bounds, .. } if **id == iced::widget::Id::new(crate::grid::ID) => Some(*bounds),
        _ => None,
    });
    *found.first().expect("a grid on the screen")
}

/// Somewhere in the first row of a grid, whatever its row height.
fn first_row(sim: &mut Sim<'_>) -> Point {
    let grid = grid_bounds(sim);
    Point::new(grid.x + 150.0, grid.y + 44.0)
}

/// The end of a field, where a click leaves the cursor after the text.
fn end_of(field: Rectangle) -> Point {
    Point::new(field.x + field.width - 6.0, field.y + field.height / 2.0)
}

fn field_under(sim: &mut Sim<'_>, caption: &str) -> Rectangle {
    let head = *texts(sim, caption).last().unwrap_or_else(|| panic!("no «{caption}» on the screen"));
    let mut below: Vec<Rectangle> = inputs(sim).into_iter().filter(|f| f.y >= head.y && f.x <= head.x + 4.0 && f.x + f.width >= head.x).collect();
    below.sort_by(|a, b| a.y.total_cmp(&b.y));
    *below.first().unwrap_or_else(|| panic!("no field under «{caption}»"))
}

struct Drive {
    app: App,
    running: SelectAll<BoxStream<'static, Action<Message>>>,
    _dir: tempfile::TempDir,
}

impl Drive {
    fn new(url: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = Ctx::for_tests(dir.path());
        ctx.api = Api::new(url.to_owned(), None, Arc::new(MemoryStore::default()));
        ctx.window = SCREEN;
        let mut drive = Self { app: App::with(ctx), running: SelectAll::new(), _dir: dir };
        drive.send(Message::Boot(Ok(None)));
        drive
    }

    fn api(&self) -> Api {
        self.app.ctx.api.clone()
    }

    fn tx(&self) -> &transactions::Transactions {
        self.app.tx.as_ref().expect("the transactions screen")
    }

    fn spawn(&mut self, task: Task<Message>) {
        if let Some(stream) = iced_runtime::task::into_stream(task) {
            self.running.push(stream);
        }
    }

    fn send(&mut self, message: Message) {
        let task = self.app.update(message);
        self.spawn(task);
    }

    /// Lets finished work come back for a moment.
    async fn pump(&mut self, for_how_long: Duration) {
        let end = Instant::now() + for_how_long;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            match tokio::time::timeout(left, self.running.next()).await {
                Ok(Some(Action::Output(message))) => self.send(message),
                Ok(Some(_)) => {}
                Ok(None) => tokio::time::sleep(left.min(Duration::from_millis(20))).await,
                Err(_) => return,
            }
        }
    }

    fn toasts(&self) -> Vec<String> {
        self.app.toasts.iter().map(|toast| toast.message.clone()).collect()
    }

    async fn until(&mut self, what: &str, done: impl Fn(&App) -> bool) {
        let end = Instant::now() + PATIENCE;
        while !done(&self.app) {
            if Instant::now() >= end {
                panic!("timed out waiting for {what}; toasts: {:?}; on screen: {:?}", self.toasts(), self.screen_text());
            }
            self.pump(Duration::from_millis(30)).await;
        }
    }

    /// Waits for the server to agree, feeding the app in the meantime.
    async fn until_server<T: std::fmt::Debug>(&mut self, what: &str, read: impl AsyncFn(Api) -> T, done: impl Fn(&T) -> bool) -> T {
        let end = Instant::now() + PATIENCE;
        loop {
            let seen = read(self.api()).await;
            if done(&seen) {
                return seen;
            }
            assert!(Instant::now() < end, "timed out waiting for {what}; last {seen:?}; toasts: {:?}", self.toasts());
            self.pump(Duration::from_millis(250)).await;
        }
    }

    fn ui<T>(&mut self, act: impl FnOnce(&mut Sim<'_>) -> T) -> T {
        let (out, messages) = {
            let mut sim = Simulator::with_size(Settings::default(), SCREEN, self.app.view());
            let out = act(&mut sim);
            (out, sim.into_messages().collect::<Vec<_>>())
        };
        for message in messages {
            self.send(message);
        }
        out
    }

    /// What the screen says, for failures.
    fn screen_text(&mut self) -> Vec<String> {
        let found = Shared::new(Mutex::new(Vec::new()));
        let sink = found.clone();
        self.ui(move |sim| {
            let _ = sim.find(move |candidate: Candidate<'_>| -> Option<()> {
                match candidate {
                    Candidate::Text { content, visible_bounds: Some(_), .. } => sink.lock().unwrap().push(content.to_owned()),
                    Candidate::TextInput { state, visible_bounds: Some(_), .. } => sink.lock().unwrap().push(format!("[{}]", state.text())),
                    _ => {}
                }
                None
            });
        });
        let all = found.lock().unwrap().clone();
        all
    }

    fn sees(&mut self, label: &str) -> bool {
        self.ui(|sim| !texts(sim, label).is_empty())
    }

    fn click(&mut self, label: &str) {
        self.ui(|sim| {
            let rect = *texts(sim, label).last().unwrap_or_else(|| panic!("no «{label}» on the screen"));
            press_at(sim, rect.center());
        });
    }

    /// For a caption that carries a count, such as «Фильтры · 2».
    fn click_starting(&mut self, start: &str) {
        let start = start.to_owned();
        self.ui(move |sim| {
            let wanted = start.clone();
            let found = collect(sim, move |candidate| match candidate {
                Candidate::Text { content, visible_bounds: Some(rect), .. } if content.starts_with(&wanted) => Some(*rect),
                _ => None,
            });
            let rect = *found.last().unwrap_or_else(|| panic!("nothing starts with «{start}»"));
            press_at(sim, rect.center());
        });
    }

    fn click_first_row(&mut self) {
        self.ui(|sim| {
            let at = first_row(sim);
            press_at(sim, at);
        });
    }

    fn right_click_first_row(&mut self) {
        self.ui(|sim| {
            let at = first_row(sim);
            right_press_at(sim, at);
        });
    }

    /// Opens the 160 wide drop-down left of `anchor` and picks a line, in one
    /// go: its list lives only as long as this view does. A drop-down keeps its
    /// words to itself, so it is found by position and the server says whether
    /// the right line was hit.
    fn choose_left_of(&mut self, anchor: &str, line: usize) {
        const ROW: f32 = 13.0 * 1.3 + 12.0;
        self.ui(|sim| {
            let rect = *texts(sim, anchor).last().unwrap_or_else(|| panic!("no «{anchor}» on the screen"));
            let open = Point::new(rect.x - 8.0 - 80.0, rect.center_y());
            press_at(sim, open);
            let at = Point::new(open.x, open.y + ROW / 2.0 + ROW * (line as f32 + 0.5));
            sim.point_at(at);
            sim.simulate([Event::Mouse(mouse::Event::CursorMoved { position: at })]);
            press_at(sim, at);
        });
    }

    /// The field `back` places before the last one drawn.
    fn fill_from_end(&mut self, back: usize, text: &str) {
        self.type_in(
            move |sim| {
                let all = inputs(sim);
                all[all.len() - 1 - back]
            },
            text,
        );
    }

    /// Presses what sits `by` pixels to the right of a caption, for controls
    /// without words of their own: a switch, an arrow.
    fn click_beside(&mut self, label: &str, by: f32) {
        self.ui(|sim| {
            let rect = *texts(sim, label).first().unwrap_or_else(|| panic!("no «{label}» on the screen"));
            press_at(sim, Point::new(rect.x + rect.width + by, rect.center_y()));
        });
    }

    async fn until_seen(&mut self, label: &str) {
        let end = Instant::now() + PATIENCE;
        while !self.sees(label) {
            if Instant::now() >= end {
                panic!("timed out waiting for «{label}»; toasts: {:?}; on screen: {:?}", self.toasts(), self.screen_text());
            }
            self.pump(Duration::from_millis(30)).await;
        }
    }

    async fn until_toast(&mut self, part: &str) -> String {
        let end = Instant::now() + PATIENCE;
        loop {
            if let Some(found) = self.toasts().into_iter().find(|toast| toast.contains(part)) {
                return found;
            }
            assert!(Instant::now() < end, "no toast with «{part}»; toasts: {:?}", self.toasts());
            self.pump(Duration::from_millis(30)).await;
        }
    }

    /// Types one character at a time: a field shows what the app last told it,
    /// so it has to be rebuilt between keystrokes just as the window does.
    fn type_in(&mut self, field: impl Fn(&mut Sim<'_>) -> Rectangle, text: &str) {
        self.ui(|sim| {
            let rect = field(sim);
            press_at(sim, end_of(rect));
            // A text field goes by the modifiers it was told about, not the key's own.
            sim.simulate([Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::CTRL)), ctrl("a")]);
            sim.simulate([Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::empty()))]);
            sim.tap_key(Named::Backspace);
        });
        for letter in text.chars() {
            self.ui(|sim| {
                let rect = field(sim);
                press_at(sim, end_of(rect));
                sim.typewrite(&letter.to_string());
            });
        }
    }

    /// Fills the field that sits under `caption`.
    fn fill(&mut self, caption: &str, text: &str) {
        let caption = caption.to_owned();
        self.type_in(move |sim| field_under(sim, &caption), text);
    }

    /// The same for a field with a button inside: a press at its far end would
    /// land on the button.
    fn fill_before_button(&mut self, caption: &str, text: &str) {
        let caption = caption.to_owned();
        self.type_in(
            move |sim| {
                let field = field_under(sim, &caption);
                Rectangle { width: field.width - BUTTON_IN_FIELD, ..field }
            },
            text,
        );
    }

    fn fill_top_field(&mut self, text: &str) {
        self.type_in(
            |sim| {
                let mut all = inputs(sim);
                all.sort_by(|a, b| a.y.total_cmp(&b.y));
                *all.first().expect("a field")
            },
            text,
        );
    }

    /// Fills the field drawn last: the one in the cell editor or a dialog.
    fn fill_last_field(&mut self, text: &str) {
        self.type_in(|sim| *inputs(sim).last().expect("a field"), text);
    }

    fn submit_last_field(&mut self) {
        self.ui(|sim| {
            let rect = *inputs(sim).last().expect("a field");
            press_at(sim, end_of(rect));
            sim.tap_key(Named::Enter);
        });
    }

    fn with_cell(&mut self, row: usize, column: Column, act: impl FnOnce(&mut Sim<'_>, Point)) {
        let (mut messages, app) = (Vec::new(), &self.app);
        let grid_of = self.tx();
        {
            let mut sim = Simulator::with_size(Settings::default(), SCREEN, app.view());
            let grid = grid_bounds(&mut sim);
            let offset: Vector = grid_of.cell_offset(row, column).expect("a visible column");
            let at = Point::new(grid.x + offset.x, grid.y + offset.y);
            assert!(grid.contains(at), "{at:?} is outside the grid {grid:?}");
            act(&mut sim, at);
            messages.extend(sim.into_messages());
        }
        for message in messages {
            self.send(message);
        }
    }

    fn double_click(&mut self, row: usize, column: Column) {
        self.with_cell(row, column, |sim, at| {
            press_at(sim, at);
            press_at(sim, at);
        });
    }

    fn keys_on(&mut self, row: usize, column: Column, keys: impl IntoIterator<Item = Event>) {
        self.with_cell(row, column, |sim, at| {
            press_at(sim, at);
            sim.simulate(keys);
        });
    }

    /// Selects `rows` as a shift-click would, then presses `key`.
    fn range_then(&mut self, rows: std::ops::RangeInclusive<usize>, column: Column, key: Event) {
        let (first, last) = (*rows.start(), *rows.end());
        self.with_cell(first, column, |sim, at| press_at(sim, at));
        let height = self.tx().cell_offset(1, column).unwrap().y - self.tx().cell_offset(0, column).unwrap().y;
        self.with_cell(first, column, |sim, at| {
            press_at(sim, at);
            sim.simulate([Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::SHIFT))]);
            press_at(sim, Point::new(at.x, at.y + height * (last - first) as f32));
            sim.simulate([Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::empty()))]);
            sim.simulate([key]);
        });
    }
}

async fn sign_in(d: &mut Drive, c: &Creds) {
    d.fill("Пользователь", &c.username);
    d.fill_before_button("Пароль", &c.password);
    d.click("Войти");
    d.until("the main screen", |app| matches!(app.phase, Phase::Main)).await;
    d.until("the rows", |app| app.tx.as_ref().is_some_and(|tx| tx.peek().loaded && tx.peek().total > 0)).await;
}

/// Out to the start window and back in: the session is the same one, and
/// nobody is asked to sign in again.
async fn start_window_and_back(d: &mut Drive) {
    let api = d.api();
    d.click(HUB);
    d.until("the start window", |app| matches!(app.phase, Phase::Login(_))).await;
    assert!(d.sees("Вход выполнен") && !d.sees("Войти"), "the start window asks a signed-in user to sign in");
    assert!(api.me().await.is_ok(), "going back to the start window ended the session");
    d.click("Открыть Чеки");
    d.until("the main screen again", |app| matches!(app.phase, Phase::Main)).await;
    d.until("the rows again", |app| app.tx.as_ref().is_some_and(|tx| tx.peek().loaded && tx.peek().total > 0)).await;
    println!("ok    start window and back: {:?}", d.tx().peek());
}

async fn add_row(d: &mut Drive, operator: &str, amount: &str) {
    d.click("Добавить");
    assert!(d.sees("Создать"), "the add form did not open: {:?}", d.screen_text());
    // A row from another year would land outside the year on screen.
    let year = d.tx().peek().year.unwrap_or(2024);
    d.fill("Дата и время", &format!("15.03.{year} 10:20"));
    d.fill("Оператор", operator);
    d.fill("Сумма", amount);
    d.fill("Последние 4 цифры", "1234");
    d.click("Создать");
    let wanted = operator.to_owned();
    d.until("the new row", move |app| {
        let tx = app.tx.as_ref().unwrap();
        (0..tx.peek().on_page).any(|row| tx.cell_text(row, Column::OperatorRaw).as_deref() == Some(wanted.as_str()))
    })
    .await;
    assert!(!d.sees("Создать"), "the add form stayed open");
}

async fn details_of(api: Api, id: i64) -> Option<String> {
    api.transaction(id).await.expect("the row").annotation_details
}

/// Add, search, edit in place, undo, redo, paste and delete, end to end.
async fn table(d: &mut Drive) {
    let mark = format!("E2EUI{}", stamp());
    let before = d.tx().peek().total;

    add_row(d, &format!("{mark} A"), "1111.50").await;
    add_row(d, &format!("{mark} B"), "2222.25").await;
    assert_eq!(d.tx().peek().total, before + 2);

    d.fill_top_field(&mark);
    d.until("the search", |app| app.tx.as_ref().unwrap().peek().shown == 2).await;
    let (first, second) = (d.tx().row_id(0).unwrap(), d.tx().row_id(1).unwrap());
    println!("ok    add and search: rows {first}, {second}");

    d.double_click(0, Column::AnnotationDetails);
    d.fill_last_field("из таблицы");
    d.submit_last_field();
    let seen = d.until_server("the edit", async |api| details_of(api, first).await, |v| v.as_deref() == Some("из таблицы")).await;
    assert_eq!(d.tx().cell_text(0, Column::AnnotationDetails).as_deref(), seen.as_deref());
    println!("ok    edit in a cell");

    d.keys_on(0, Column::AnnotationDetails, [ctrl("z")]);
    d.until_server("the undo", async |api| details_of(api, first).await, |v| v.as_deref().unwrap_or_default().is_empty()).await;
    d.keys_on(0, Column::AnnotationDetails, [ctrl("y")]);
    d.until_server("the redo", async |api| details_of(api, first).await, |v| v.as_deref() == Some("из таблицы")).await;
    println!("ok    undo and redo");

    d.keys_on(0, Column::AnnotationDetails, []);
    d.send(Message::Tx(transactions::Msg::Edit(transactions::edit::Msg::Pasted(Some("первая\nвторая".into())))));
    d.until_server("the paste", async |api| details_of(api, second).await, |v| v.as_deref() == Some("вторая")).await;
    assert_eq!(details_of(d.api(), first).await.as_deref(), Some("первая"));
    println!("ok    paste over two rows");

    d.range_then(0..=1, Column::AnnotationDetails, press_key(Named::Delete, None));
    assert!(d.sees("Удалить"), "no confirmation before deleting");
    d.click("Удалить");
    d.until_server("the delete", async |api| api.transaction(second).await.is_err() && api.transaction(first).await.is_err(), |gone| *gone).await;
    d.until("the rows to leave", |app| app.tx.as_ref().unwrap().peek().shown == 0).await;
    assert_eq!(d.tx().peek().total, before);
    d.fill_top_field("");
    d.until("the search to clear", move |app| app.tx.as_ref().unwrap().peek().shown == before).await;
    println!("ok    delete with confirmation");
}

fn xlsx_at(path: &std::path::Path) -> usize {
    let bytes = std::fs::read(path).unwrap_or_default();
    assert!(bytes.starts_with(b"PK"), "{} is not a workbook", path.display());
    bytes.len()
}

async fn saved(d: &mut Drive, path: &std::path::Path) -> usize {
    let end = Instant::now() + PATIENCE;
    while !path.exists() {
        assert!(Instant::now() < end, "nothing saved to {}; toasts: {:?}", path.display(), d.toasts());
        d.pump(Duration::from_millis(30)).await;
    }
    // The file appears before the write ends.
    d.pump(Duration::from_millis(300)).await;
    xlsx_at(path)
}

fn choose_file(path: Option<std::path::PathBuf>) {
    *crate::files::CHOSEN.lock().unwrap() = path;
}

/// A date filter narrows the table and the reset brings every row back.
async fn filters(d: &mut Drive) {
    let all = d.tx().peek().shown;
    let date = d.tx().cell_text(0, Column::TransactionDate).expect("a date");
    let day = date.split_whitespace().next().unwrap().to_owned();

    d.click("Фильтры");
    d.fill("с", &day);
    d.fill("по", &day);
    d.click("Применить");
    d.until("the filter", move |app| app.tx.as_ref().unwrap().peek().shown < all).await;
    let shown = d.tx().peek().shown;
    assert!(shown > 0, "the day of a row found no rows");
    for row in 0..d.tx().peek().on_page {
        let date = d.tx().cell_text(row, Column::TransactionDate).unwrap();
        assert!(date.starts_with(&day), "{date} slipped through the filter {day}");
    }
    println!("ok    filter by date: {shown} of {all} on {day}");

    d.click_starting("Фильтры");
    d.click("Сбросить всё");
    d.click("Применить");
    d.until("the reset", move |app| app.tx.as_ref().unwrap().peek().shown == all).await;
    println!("ok    filters reset");
}

/// Each way out of the table lands a real workbook where the person pointed.
async fn export(d: &mut Drive) {
    let folder = tempfile::tempdir().unwrap();

    let view = folder.path().join("view.xlsx");
    choose_file(Some(view.clone()));
    d.click("Экспорт");
    d.click("Экспорт текущего вида");
    let view = saved(d, &view).await;

    let all = folder.path().join("all.xlsx");
    choose_file(Some(all.clone()));
    d.click("Экспорт");
    d.click("Экспорт всех транзакций");
    let all = saved(d, &all).await;

    let year = d.tx().peek().year.unwrap_or(2024);
    let period = folder.path().join("period.xlsx");
    choose_file(Some(period.clone()));
    d.click("Экспорт");
    d.fill_from_end(1, &format!("01.01.{year}"));
    d.fill_from_end(0, &format!("31.03.{year}"));
    d.click("Скачать");
    let period = saved(d, &period).await;
    println!("ok    export: view {view} B, all {all} B, period {period} B");

    let picked = folder.path().join("picked.xlsx");
    choose_file(Some(picked.clone()));
    d.range_then(0..=1, Column::AnnotationDetails, press_key(Named::F13, None));
    assert!(d.sees("Снять"), "the selection bar did not show");
    d.click("Экспорт");
    let picked = saved(d, &picked).await;
    assert!(picked < all, "two rows came out as big as the whole table");
    d.click("Снять");
    println!("ok    export of selected rows: {picked} B");
    choose_file(None);
}

/// A workbook made in the panel is on the server, its file is saved, and it
/// can be taken away again.
async fn excel(d: &mut Drive) {
    let name = format!("E2EUI {}", stamp());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.xlsm");
    choose_file(Some(path.clone()));
    d.click("Excel-синхронизация");
    d.until_seen("Название книги").await;
    d.fill("Название книги", &name);
    d.fill("Пароль книги", "e2e-Book-12345");
    d.pump(Duration::from_millis(800)).await; // the button waits for the list
    d.click("Создать и скачать книгу");
    let wanted = name.clone();
    let made = d.until_server("the workbook", async |api| api.excel_activations().await.unwrap(), move |all| all.iter().any(|a| a.name.as_deref() == Some(wanted.as_str()))).await;
    let id = made.iter().find(|a| a.name.as_deref() == Some(name.as_str())).unwrap().workbook_id.clone();
    saved(d, &path).await;
    d.until_seen(&name).await;
    println!("ok    excel workbook made and saved");

    d.pump(Duration::from_millis(500)).await; // the panel is busy until the file is written
    // The newest workbook is the first in the list.
    d.click("Убрать пароль");
    let gone = id.clone();
    d.until_server("the removal", async |api| api.excel_activations().await.unwrap(), move |all| all.iter().all(|a| a.workbook_id != gone)).await;
    d.pump(Duration::from_millis(500)).await; // closing waits for the reply
    d.click("Готово");
    choose_file(None);
    println!("ok    excel workbook removed");
}

fn ledger_book(operator: &str, details: &str, year: i32) -> Vec<u8> {
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet();
    for (col, head) in ["Дата и время", "Оператор", "Сумма", "ПК", "Подробности"].into_iter().enumerate() {
        sheet.write_string(1, col as u16, head).unwrap();
    }
    let date = format!("{year}-03-15 10:20:00");
    for (col, value) in [date.as_str(), operator, "1111.50", "1234", details].into_iter().enumerate() {
        sheet.write_string(3, col as u16, value).unwrap();
    }
    book.save_to_buffer().unwrap()
}

async fn operators(d: &mut Drive) {
    let name = format!("E2EUI {}", stamp());
    d.click("Добавить");
    d.until_seen("Новая запись").await;
    d.fill("Оператор · продавец", &name);
    d.fill("Приложение", "e2eapp");
    d.click("Добавить");
    let wanted = name.clone();
    let read = async |api: Api| api.all_operators().await.unwrap();
    let all = d.until_server("the operator", read, move |all| all.iter().any(|o| o.operator_name == wanted)).await;
    let id = all.iter().find(|o| o.operator_name == name).unwrap().id;
    println!("ok    operator added");

    d.fill_top_field(&name);
    d.pump(Duration::from_millis(700)).await;
    d.right_click_first_row();
    d.until_seen("Изменить").await;
    d.click("Изменить");
    d.until_seen("Редактирование").await;
    d.fill("Приложение", "e2eother");
    d.click("Сохранить");
    d.until_server("the change", read, move |all| all.iter().any(|o| o.id == id && o.application_name == "e2eother")).await;
    println!("ok    operator changed");

    let folder = tempfile::tempdir().unwrap();
    let book = folder.path().join("operators.xlsx");
    choose_file(Some(book.clone()));
    d.click("Экспорт");
    saved(d, &book).await;
    d.click("Импорт");
    d.until_toast("Импорт").await;
    choose_file(None);
    println!("ok    operators exported and read back: {:?}", d.toasts().last());

    d.right_click_first_row();
    d.until_seen("Удалить").await;
    d.click("Удалить");
    d.until_seen("Отмена").await;
    d.click("Удалить");
    d.until_server("the removal", read, move |all| !all.iter().any(|o| o.id == id)).await;
    d.fill_top_field("");
    println!("ok    operator removed");
}

async fn descriptions(d: &mut Drive) {
    let text = format!("E2EUI описание {}", stamp());
    d.click("Описания");
    d.until_seen("Справочник описаний").await;
    d.click("Добавить описание");
    d.fill("Описание", &text);
    d.click("Добавить");
    let wanted = text.clone();
    let read = async |api: Api| api.descriptions(&[("search", "E2EUI".to_owned())]).await.unwrap().items;
    let found = d.until_server("the description", read, move |all| all.iter().any(|x| x.text == wanted)).await;
    let id = found.iter().find(|x| x.text == text).unwrap().id;
    println!("ok    description added");

    d.fill_top_field(&text);
    d.pump(Duration::from_millis(700)).await;
    d.right_click_first_row();
    d.until_seen("Удалить описание?").await;
    d.until_seen("Отмена").await;
    d.click("Удалить");
    d.until_server("the removal", read, move |all| !all.iter().any(|x| x.id == id)).await;
    d.fill_top_field("");
    println!("ok    description removed");
}

async fn dictionary(d: &mut Drive) {
    let code = format!("Е{}", stamp()); // the server spells codes in Cyrillic
    d.click("Разметка");
    d.until_seen("Что значат короткие коды из книги учёта: статьи и лица.").await;
    d.click("Добавить");
    d.fill("Код", &code);
    d.fill("Расшифровка", "из теста");
    d.click("Сохранить");
    let wanted = code.clone();
    let read = async |api: Api| api.annotation_articles(true).await.unwrap();
    let all = d.until_server("the code", read, move |all| all.iter().any(|a| a.code.eq_ignore_ascii_case(&wanted))).await;
    let id = all.iter().find(|a| a.code.eq_ignore_ascii_case(&code)).unwrap().id;
    println!("ok    dictionary code added");

    d.fill_top_field(&code);
    d.pump(Duration::from_millis(300)).await;
    d.right_click_first_row();
    d.until_seen("Изменить").await;
    d.click("Изменить");
    d.fill("Расшифровка", "изменено");
    d.click("Сохранить");
    d.until_server("the change", read, move |all| all.iter().any(|a| a.id == id && a.label == "изменено")).await;
    println!("ok    dictionary code changed");

    d.right_click_first_row();
    d.until_seen("Удалить").await;
    d.click("Удалить");
    d.until_seen("Удалить код?").await;
    d.click("Удалить");
    d.until_server("the removal", read, move |all| !all.iter().any(|a| a.id == id && a.is_active)).await;
    d.fill_top_field("");
    println!("ok    dictionary code removed");
}

/// Everything in the reference section that writes.
async fn reference(d: &mut Drive) {
    d.click("Справочник");
    d.until_seen("Справочник операторов").await;
    operators(d).await;
    descriptions(d).await;
    dictionary(d).await;
}

async fn logs(d: &mut Drive) {
    d.click("Логи");
    d.until("the log", |app| app.logs.as_ref().is_some_and(|logs| logs.peek().1 > 0)).await;
    let (shown, total, _) = d.app.logs.as_ref().unwrap().peek();
    println!("ok    log loaded: {shown} of {total}");

    let pages = (total + 49) / 50;
    if pages > 1 {
        d.click_beside(&format!("1 / {pages}"), 22.0);
        d.until("the next page", |app| app.logs.as_ref().unwrap().peek().2 == 2).await;
        println!("ok    log next page");
    }

    d.choose_left_of("Только дубликаты", 2);
    d.until("the status filter", move |app| app.logs.as_ref().unwrap().peek().1 < total).await;
    let (_, failed, page) = d.app.logs.as_ref().unwrap().peek();
    assert_eq!(page, 1, "a filter must start from the first page");
    println!("ok    log filter by status: {failed} of {total}");
    d.choose_left_of("Только дубликаты", 0);
    d.until("the filter off", move |app| app.logs.as_ref().unwrap().peek().1 == total).await;

    d.click("Только дубликаты");
    d.until("the duplicates", move |app| app.logs.as_ref().unwrap().peek().1 < total).await;
    d.click("Только дубликаты");
    d.until("the duplicates off", move |app| app.logs.as_ref().unwrap().peek().1 == total).await;
    println!("ok    log filter by duplicates");

    d.click_first_row();
    d.until_seen("Закрыть").await;
    d.click("Закрыть");
    assert!(!d.sees("Закрыть"), "the record stayed open");
    println!("ok    log record opens and closes");
}

async fn audit(d: &mut Drive) {
    d.click("Аудит");
    d.until("the audit", |app| app.audit.as_ref().is_some_and(|audit| !audit.actions().is_empty())).await;
    let rows = d.app.audit.as_ref().unwrap().actions();
    let action = rows[0].clone();
    let others = rows.iter().filter(|name| **name != action).count();
    d.fill_top_field(&action);
    let wanted = action.clone();
    d.until("the action filter", move |app| {
        let now = app.audit.as_ref().unwrap().actions();
        !now.is_empty() && now.iter().all(|name| *name == wanted)
    })
    .await;
    println!("ok    audit filter by action: {action} (hid {others})");
    d.click_first_row();
    d.until_seen("Закрыть").await;
    d.click("Закрыть");
    d.fill_top_field("");
    println!("ok    audit record opens and closes");
}

async fn users(d: &mut Drive) {
    let login = format!("e2eui{}", stamp());
    let (first, second) = ("E2e-Pass-12345!", "E2e-Pass-67890!");
    d.click("Пользователи");
    d.until_seen("Клик по строке открывает карточку, правый клик или «···» — действия.").await;
    d.click("Добавить");
    d.until_seen("Новый пользователь").await;
    d.fill("Логин", &login);
    d.fill("Пароль", first);
    d.fill("Отображаемое имя", "Тест Интерфейса");
    d.click("Создать");
    let wanted = login.clone();
    let read = async |api: Api| api.admin_users().await.unwrap();
    let all = d.until_server("the user", read, move |all| all.iter().any(|u| u.username == wanted)).await;
    let made = all.iter().find(|u| u.username == login).unwrap();
    let id = made.id;
    assert_eq!((made.display_name.as_str(), made.role.as_str(), made.is_active), ("Тест Интерфейса", "operator", true));
    d.until("the form to close", |app| matches!(app.phase, Phase::Main)).await;
    println!("ok    user created");

    d.pump(Duration::from_millis(300)).await;
    d.fill_top_field(&login);
    d.pump(Duration::from_millis(200)).await;
    d.click_first_row();
    d.until_seen("Сохранить").await;
    d.fill("Отображаемое имя", "Новое Имя");
    d.click("Сохранить");
    d.until_server("the new name", read, move |all| all.iter().any(|u| u.id == id && u.display_name == "Новое Имя")).await;
    println!("ok    user changed");

    d.pump(Duration::from_millis(300)).await;
    d.right_click_first_row();
    d.until_seen("Сбросить пароль").await;
    d.click("Сбросить пароль");
    d.fill_last_field(second);
    d.submit_last_field();
    d.until_toast("арол").await;
    let fresh = Api::new(d.api().base(), None, Arc::new(MemoryStore::default()));
    assert!(!fresh.login(&login, first).await.expect("old password call").ok, "the old password still works");
    assert!(fresh.login(&login, second).await.expect("new password call").ok, "the new password does not work");
    fresh.logout().await;
    println!("ok    password reset");

    d.right_click_first_row();
    d.until_seen("Деактивировать").await;
    d.click("Деактивировать");
    d.until_seen("Деактивировать пользователя?").await;
    d.click("Деактивировать");
    d.until_server("the deactivation", read, move |all| all.iter().any(|u| u.id == id && !u.is_active)).await;
    d.fill_top_field("");
    println!("ok    user deactivated");
}

async fn periods(d: &mut Drive) {
    let reason = format!("E2EUI {}", stamp());
    d.click("Заблокир. периоды");
    d.until_seen("Дата начала").await;
    d.fill("Дата начала", "01.01.2001");
    d.fill("Дата конца", "02.01.2001");
    d.fill("Причина", &reason);
    d.click("Добавить");
    let wanted = reason.clone();
    let read = async |api: Api| api.periods().await.unwrap();
    let all = d.until_server("the period", read, move |all| all.iter().any(|p| p.reason.as_deref() == Some(wanted.as_str()) && p.is_active)).await;
    let id = all.iter().find(|p| p.reason.as_deref() == Some(reason.as_str())).unwrap().id;
    d.until_seen(&reason).await;
    println!("ok    period locked");

    d.click("Снять");
    d.until_server("the unlock", read, move |all| !all.iter().any(|p| p.id == id && p.is_active)).await;
    println!("ok    period released");
}

async fn sessions(d: &mut Drive) {
    d.click("Сессии");
    d.pump(Duration::from_millis(800)).await;
    let read = async |api: Api| api.sessions().await.unwrap();
    let before = read(d.api()).await;
    d.pump(Duration::from_millis(500)).await;
    if before.len() > 1 {
        d.click_starting("Завершить все");
        d.until_server("the others to end", read, |all| all.len() == 1).await;
    }
    assert!(d.api().me().await.is_ok(), "ending the others ended this session too");
    println!("ok    other sessions ended: {} were open", before.len());
}

async fn system(d: &mut Drive, row: i64, details: &str, book: &std::path::Path) {
    d.click("Системные");
    d.until_seen("OTP по scope").await;
    d.until_seen("Выбрать файл").await;

    let read = async |api: Api| api.system_settings().await.unwrap().otp_enabled;
    let was = read(d.api()).await;
    let word = if was { "Включено" } else { "Выключено" };
    d.click_beside(word, 26.0);
    d.until_server("the switch", read, move |on| *on != was).await;
    d.pump(Duration::from_millis(300)).await;
    let word = if was { "Выключено" } else { "Включено" };
    d.click_beside(word, 26.0);
    d.until_server("the switch back", read, move |on| *on == was).await;
    println!("ok    system switch both ways");

    choose_file(Some(book.to_path_buf()));
    d.click("Выбрать файл");
    d.until_seen(book.file_name().unwrap().to_str().unwrap()).await;
    d.pump(Duration::from_millis(200)).await;
    d.click("Загрузить сейчас");
    let wanted = details.to_owned();
    d.until_server("the book", async |api| details_of(api, row).await, move |v| v.as_deref() == Some(wanted.as_str())).await;
    choose_file(None);
    println!("ok    ledger book imported");

    d.click("Предпросмотр");
    d.until_seen("Дозаполнить").await;
    d.click("Пересчитать правила");
    d.pump(Duration::from_millis(500)).await;
    println!("ok    rules preview and recount: {:?}", d.toasts().last());
}

async fn appearance(d: &mut Drive) {
    d.click("Внешний вид");
    d.until_seen("Тёмная").await;
    d.click("Тёмная");
    d.until("the dark theme", |app| app.ctx.dark).await;
    d.click("Светлая");
    d.until("the light theme", |app| !app.ctx.dark).await;
    println!("ok    theme both ways");
}

/// The sections that only show things have to open and fill.
async fn the_rest_of_settings(d: &mut Drive) {
    d.click("Безопасность");
    d.until_seen("Двухфакторная аутентификация").await;
    d.click("Доступы и папки");
    d.pump(Duration::from_millis(500)).await;
    assert!(!d.toasts().iter().any(|toast| toast.contains("Не удалось")), "{:?}", d.toasts());
    println!("ok    security and access open");
}

async fn sign_out(d: &mut Drive) {
    let api = d.api();
    d.click("Профиль");
    d.until_seen("Отображаемое имя").await;
    d.click("Выйти");
    d.until_seen("Остаться").await;
    d.click("Выйти");
    d.until("the login form", |app| matches!(app.phase, Phase::Login(_))).await;
    assert!(d.sees("Войти"), "no login form after signing out");
    assert!(api.me().await.is_err(), "the session outlived the sign-out");
    println!("ok    sign out");
}

#[tokio::test(flavor = "current_thread")]
async fn the_screens_do_what_their_buttons_say() {
    let Some(c) = creds() else {
        eprintln!("skipped: TBS_E2E_WRITES and TBS_E2E_LOGIN are not set");
        return;
    };
    let mut d = Drive::new(&c.url);
    sign_in(&mut d, &c).await;
    println!("ok    sign in: {:?}", d.tx().peek());
    start_window_and_back(&mut d).await;
    table(&mut d).await;
    filters(&mut d).await;
    export(&mut d).await;
    excel(&mut d).await;

    // A row for the ledger book to find, made before leaving the table.
    let mark = format!("E2EUI{} BOOK", stamp());
    add_row(&mut d, &mark, "1111.50").await;
    let year = d.tx().peek().year.unwrap_or(2024);
    let row = (0..d.tx().peek().on_page).find(|&at| d.tx().cell_text(at, Column::OperatorRaw).as_deref() == Some(mark.as_str())).unwrap();
    let row = d.tx().row_id(row).unwrap();
    let details = "из книги через экран";
    let folder = tempfile::tempdir().unwrap();
    let book = folder.path().join("ledger.xlsx");
    std::fs::write(&book, ledger_book(&mark, details, year)).unwrap();

    reference(&mut d).await;
    logs(&mut d).await;
    audit(&mut d).await;
    d.click("Настройки");
    d.until_seen("Безопасность").await;
    users(&mut d).await;
    periods(&mut d).await;
    sessions(&mut d).await;
    system(&mut d, row, details, &book).await;
    appearance(&mut d).await;
    the_rest_of_settings(&mut d).await;

    d.api().delete_transaction(row).await.expect("cleanup");
    sign_out(&mut d).await;
}
