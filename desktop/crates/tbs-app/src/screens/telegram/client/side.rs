//! The rest of what happens in the chats, beside reading and writing them:
//! the switches of a chat, its password, the period, the buttons under a
//! message and its files.

use std::time::Instant;

use iced::Task;
use iced::widget::image;
use iced::widget::operation::{self, RelativeOffset};
use tbs_core::api::{TgButtonSource, TgMenuButton};

use super::*;
use crate::ctx::{Ctx, text_error};
use crate::files;
use crate::theme::Tone;

impl Client {
    pub(super) fn ask_menu_button(&mut self, ctx: &Ctx) -> Task<Msg> {
        let Some(chat) = self.active_chat().filter(|chat| chat.chat_type == "bot").map(|chat| chat.chat_id) else { return Task::none() };
        if self.menu_buttons.get(&chat).is_some_and(|(asked, _)| asked.elapsed() < MENU_BUTTON_LIFE) {
            return Task::none();
        }
        self.menu_buttons.insert(chat, (Instant::now(), TgMenuButton::default()));
        let api = ctx.api.clone();
        Task::perform(async move { api.tg_menu_button(chat).await.map_err(|e| text_error(&e)) }, move |outcome| Msg::MenuButton(chat, outcome))
    }

    /// Photos of the open chat, the newest first and a few at a time.
    pub(super) fn want_photos(&mut self, ctx: &Ctx) -> Task<Msg> {
        let Some(chat) = self.active else { return Task::none() };
        let wanted: Vec<PhotoKey> = self.messages.iter().rev().filter_map(|message| Some((chat, message.id.filter(|id| *id != 0)?, message.photo.as_ref()?.file_id?))).collect();
        self.photos.retain(|key, _| wanted.contains(key));
        self.photo_queue.retain(|key| wanted.contains(key));
        for key in wanted {
            if !self.photos.contains_key(&key) {
                self.photos.insert(key, Photo::Loading);
                self.photo_queue.push_back(key);
            }
        }
        self.pump_photos(ctx)
    }

    pub(super) fn pump_photos(&mut self, ctx: &Ctx) -> Task<Msg> {
        let mut tasks = Vec::new();
        while self.photo_flights < PHOTOS_AT_ONCE
            && let Some(key) = self.photo_queue.pop_front()
        {
            self.photo_flights += 1;
            let api = ctx.api.clone();
            let name = self.messages.iter().find(|message| message.id == Some(key.1)).and_then(|message| message.photo.as_ref()).and_then(|photo| photo.file_name.clone());
            tasks.push(Task::perform(
                async move { api.tg_file(key.0, key.1, key.2, name.as_deref()).await.map(|file| image::Handle::from_bytes(file.bytes)).map_err(|e| text_error(&e)) },
                move |outcome| Msg::Photo(key, outcome),
            ));
        }
        Task::batch(tasks)
    }
    fn submit_password(&mut self, ctx: &Ctx) -> Task<Msg> {
        let Some(form) = self.password.as_mut().filter(|form| !form.busy) else { return Task::none() };
        let password = form.password.trim().to_owned();
        let problem = if password.is_empty() {
            Some("Введите пароль")
        } else if form.mode == PasswordMode::Set && password.chars().count() < 4 {
            Some("Минимум 4 символа")
        } else if form.mode == PasswordMode::Set && password != form.confirm.trim() {
            Some("Пароли не совпадают")
        } else {
            None
        };
        if let Some(problem) = problem {
            form.error = Some(problem.to_owned());
            return Task::none();
        }
        form.error = None;
        form.busy = true;
        let (api, chat, mode) = (ctx.api.clone(), form.chat, form.mode);
        Task::perform(
            async move {
                let outcome = match mode {
                    PasswordMode::Set => api.tg_set_chat_password(chat, &password).await,
                    PasswordMode::Remove => api.tg_remove_chat_password(chat, &password).await,
                };
                outcome.map(|_| ()).map_err(|e| text_error(&e))
            },
            move |outcome| Msg::PasswordDone(mode, outcome),
        )
    }

    fn apply_period(&mut self) {
        let Some(form) = &mut self.period else { return };
        let (Ok(from), Ok(to)) = (dates::read_day(&form.from), dates::read_day(&form.to)) else {
            form.error = Some("Дата в формате ДД.ММ.ГГГГ".to_owned());
            return;
        };
        if from.is_none() && to.is_none() {
            return;
        }
        if let (Some(from), Some(to)) = (from, to)
            && from > to
        {
            form.error = Some("Начало периода позже его конца".to_owned());
            return;
        }
        self.range = Range { from, to };
        self.period = None;
    }

    fn press_key(&mut self, message: i64, row: usize, col: usize) -> Task<Msg> {
        let Some(chat) = self.active else { return Task::none() };
        let Some(found) = self.messages.iter().find(|candidate| candidate.id.unwrap_or(0) == message) else { return Task::none() };
        let Some(markup) = &found.reply_markup else { return Task::none() };
        let Some(button) = markup.rows.get(row).and_then(|line| line.get(col)) else { return Task::none() };
        let url = button.url.clone().filter(|url| !url.is_empty());
        match (button.kind.as_str(), url) {
            ("web_app", Some(url)) => {
                let source = if markup.kind == "keyboard" { TgButtonSource::Keyboard } else { TgButtonSource::Inline };
                self.wants.push(Want::App(AppRequest { chat, bot: found.sender(), url, source, text: button.text.clone() }));
            }
            ("url" | "login_url", Some(url)) => self.wants.push(Want::Link(url)),
            ("copy_text", url) => {
                let text = url.unwrap_or_else(|| button.text.clone());
                self.copied = Some((message, row, col));
                return Task::batch([iced::clipboard::write(text), Task::perform(async { tokio::time::sleep(COPIED_FOR).await }, move |()| Msg::CopiedOff(message, row, col))]);
            }
            _ => {}
        }
        Task::none()
    }

    fn save_document(&mut self, message: i64, ctx: &Ctx) -> Task<Msg> {
        let Some(chat) = self.active else { return Task::none() };
        if self.saving.is_some() {
            return Task::none();
        }
        let Some(document) = self.messages.iter().find(|candidate| candidate.id == Some(message)).and_then(|found| found.document.as_ref()) else { return Task::none() };
        let Some(file) = document.file_id else { return Task::none() };
        let name = document.file_name.clone().filter(|name| !name.is_empty()).unwrap_or_else(|| "document".to_owned());
        self.saving = Some(message);
        let api = ctx.api.clone();
        Task::perform(
            async move {
                let download = api.tg_file(chat, message, file, Some(&name)).await.map_err(|e| text_error(&e))?;
                files::save_any(name, download.bytes).await
            },
            Msg::Saved,
        )
    }
    pub(super) fn update_side(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Monitor(chat, enabled) => {
                let api = ctx.api.clone();
                return Task::perform(async move { api.tg_set_monitor(chat, enabled).await.map(|_| ()).map_err(|e| text_error(&e)) }, move |outcome| Msg::Monitored(chat, enabled, outcome));
            }
            Msg::Monitored(chat, enabled, outcome) => match outcome {
                Ok(()) => {
                    if let Some(found) = self.chats.iter_mut().find(|candidate| candidate.chat_id == chat) {
                        found.monitor_enabled = enabled;
                        found.is_monitored |= enabled;
                    }
                    ctx.toast(Tone::Success, if enabled { "Мониторинг включён" } else { "Мониторинг выключен" });
                    return self.reload_chats(ctx);
                }
                Err(reason) => ctx.toast(Tone::Danger, reason),
            },
            Msg::Hide(chat, hidden) => {
                self.menu = None;
                // Shown at once; the server's word settles it either way.
                self.hiding.insert(chat, hidden);
                let api = ctx.api.clone();
                return Task::perform(async move { api.tg_set_hidden(chat, hidden).await.map(|_| ()).map_err(|e| text_error(&e)) }, move |outcome| Msg::Hidden(chat, hidden, outcome));
            }
            Msg::Hidden(chat, hidden, outcome) => {
                // A later choice for the same chat is still on its way.
                if self.hiding.get(&chat) == Some(&hidden) {
                    self.hiding.remove(&chat);
                }
                match outcome {
                    Ok(()) => {
                        if let Some(found) = self.chats.iter_mut().find(|candidate| candidate.chat_id == chat) {
                            found.is_hidden = hidden;
                        }
                        ctx.toast(Tone::Success, if hidden { "Чат скрыт" } else { "Чат показан" });
                    }
                    Err(reason) => ctx.toast(Tone::Danger, reason),
                }
            }
            Msg::Pointer(at) => self.pointer = at,
            Msg::Context(target) => self.menu = Some((target, self.pointer)),
            Msg::CloseMenu => self.menu = None,
            Msg::AskPassword(chat, mode) => {
                self.menu = None;
                let title = self.chat(chat).map(|found| found.title.clone()).unwrap_or_default();
                self.password = Some(PasswordForm { chat, title, mode, password: String::new(), confirm: String::new(), error: None, busy: false });
            }
            Msg::PasswordInput(value) => {
                if let Some(form) = self.password.as_mut().filter(|form| !form.busy) {
                    form.password = value;
                }
            }
            Msg::PasswordConfirm(value) => {
                if let Some(form) = self.password.as_mut().filter(|form| !form.busy) {
                    form.confirm = value;
                }
            }
            Msg::PasswordSubmit => return self.submit_password(ctx),
            Msg::PasswordDone(mode, outcome) => match outcome {
                Ok(()) => {
                    self.password = None;
                    ctx.toast(Tone::Success, if mode == PasswordMode::Set { "Пароль установлен" } else { "Пароль снят" });
                    return self.reload_chats(ctx);
                }
                Err(reason) => {
                    if let Some(form) = &mut self.password {
                        form.busy = false;
                        form.error = Some(reason);
                    }
                }
            },
            Msg::ClosePassword => {
                if self.password.as_ref().is_some_and(|form| !form.busy) {
                    self.password = None;
                }
            }
            Msg::LockInput(value) => {
                if let Some(form) = self.lock.as_mut().filter(|form| !form.busy) {
                    form.password = value;
                }
            }
            Msg::Unlock => {
                let Some(chat) = self.active else { return Task::none() };
                let Some(form) = self.lock.as_mut().filter(|form| !form.busy && !form.password.trim().is_empty()) else { return Task::none() };
                form.busy = true;
                form.error = None;
                let (api, password) = (ctx.api.clone(), form.password.trim().to_owned());
                return Task::perform(async move { api.tg_unlock_chat(chat, &password).await.map_err(|e| text_error(&e)) }, move |outcome| Msg::Unlocked(chat, outcome));
            }
            Msg::Unlocked(chat, outcome) => {
                if self.active != Some(chat) {
                    return Task::none();
                }
                let Some(form) = &mut self.lock else { return Task::none() };
                form.busy = false;
                match outcome {
                    Ok(answer) if answer.verified => {
                        self.lock = None;
                        self.messages_loaded = false;
                        return Task::batch([self.reload_messages(chat, ctx), self.ask_menu_button(ctx)]);
                    }
                    Ok(answer) if answer.locked => form.error = Some("Слишком много неверных попыток. Чат временно заблокирован.".to_owned()),
                    Ok(answer) => {
                        form.password.clear();
                        form.error = Some(match answer.attempts_left {
                            Some(left) => format!("Неверный пароль (осталось попыток: {left})"),
                            None => "Неверный пароль".to_owned(),
                        });
                    }
                    Err(reason) => form.error = Some(reason),
                }
            }
            Msg::MenuButton(chat, outcome) => {
                // A refusal is remembered as "none" until it ages, not asked again at once.
                if let Ok(button) = outcome {
                    self.menu_buttons.insert(chat, (Instant::now(), button));
                }
            }
            Msg::MenuApp => {
                if let Some(chat) = self.active
                    && let Some(button) = self.menu_app(chat)
                    && let Some(url) = button.url.clone()
                {
                    let text = button.text.clone().filter(|text| !text.is_empty()).unwrap_or_else(|| "Открыть приложение".to_owned());
                    let bot = button.bot_user_id;
                    self.wants.push(Want::App(AppRequest { chat, bot, url, source: TgButtonSource::Menu, text }));
                }
            }
            Msg::CopyText(message) => {
                self.menu = None;
                // A message drawn by the client cannot be selected with the mouse; this is its way out.
                if let Some(words) = self.messages.iter().find(|candidate| candidate.id == Some(message)).and_then(|found| found.text.clone()).filter(|words| !words.is_empty()) {
                    ctx.toast(Tone::Success, "Текст скопирован");
                    return iced::clipboard::write(words);
                }
            }
            Msg::Key(message, row, col) => return self.press_key(message, row, col),
            Msg::CopiedOff(message, row, col) => {
                if self.copied == Some((message, row, col)) {
                    self.copied = None;
                }
            }
            Msg::TogglePeriod => {
                self.period = match self.period {
                    Some(_) => None,
                    None => {
                        let today = dates::now().date();
                        let custom = self.range.is_set() && !Preset::ALL.into_iter().any(|preset| preset.range(today) == self.range);
                        Some(PeriodForm { custom, from: dates::typed(self.range.from), to: dates::typed(self.range.to), error: None })
                    }
                };
            }
            Msg::Preset(preset) => {
                self.range = preset.range(dates::now().date());
                self.period = None;
            }
            Msg::PeriodCustom => {
                if let Some(form) = &mut self.period {
                    form.custom = !form.custom;
                }
            }
            Msg::PeriodFrom(value) => {
                if let Some(form) = &mut self.period {
                    form.from = value;
                    form.error = None;
                }
            }
            Msg::PeriodTo(value) => {
                if let Some(form) = &mut self.period {
                    form.to = value;
                    form.error = None;
                }
            }
            Msg::PeriodApply => self.apply_period(),
            Msg::ClearRange => {
                self.range = Range::default();
                self.period = None;
            }
            Msg::Photo(key, outcome) => {
                self.photo_flights = self.photo_flights.saturating_sub(1);
                if let Some(slot) = self.photos.get_mut(&key) {
                    *slot = outcome.map_or(Photo::Failed, Photo::Ready);
                }
                return self.pump_photos(ctx);
            }
            Msg::ViewPhoto(key) => {
                if self.selecting || self.command {
                    self.toggle(key.1);
                } else if matches!(self.photos.get(&key), Some(Photo::Ready(_))) {
                    self.viewer = Some(key);
                }
            }
            Msg::CloseViewer => self.viewer = None,
            Msg::SavePhoto => {
                if let Some(key) = self.viewer
                    && let Some(Photo::Ready(image::Handle::Bytes(_, bytes))) = self.photos.get(&key)
                {
                    let name = self.messages.iter().find(|message| message.id == Some(key.1)).and_then(|message| message.photo.as_ref()).and_then(|photo| photo.file_name.clone());
                    return Task::perform(files::save_any(name.unwrap_or_else(|| "photo.jpg".to_owned()), bytes.to_vec()), Msg::Saved);
                }
            }
            Msg::Document(message) => {
                if self.selecting || self.command {
                    self.toggle(message);
                } else {
                    return self.save_document(message, ctx);
                }
            }
            Msg::Saved(outcome) => {
                self.saving = None;
                match outcome {
                    Ok(Some(path)) => ctx.toast(Tone::Success, format!("Файл сохранён: {path}")),
                    Ok(None) => {}
                    Err(reason) => ctx.toast(Tone::Danger, format!("Не удалось сохранить файл: {reason}")),
                }
            }
            Msg::Scrolled(distance) => self.away = distance > NEAR_BOTTOM && !self.messages.is_empty(),
            Msg::ToBottom => {
                self.away = false;
                return operation::snap_to(STREAM, RelativeOffset::START);
            }
            Msg::Command(held) => self.command = held,
            // Reading and writing the chats is the other half, in `update`.
            _ => {}
        }
        Task::none()
    }
}
