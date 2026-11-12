//! Общее состояние сервера.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{Duration, NaiveDateTime, Utc};
use tokio::sync::broadcast;

use crate::bot::BotStatus;
use crate::config::Config;
use crate::db::Db;
use crate::hub::Hub;
use crate::service::Engine;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub cfg: Arc<Config>,
    /// Уведомления интерфейсу об изменениях (SSE).
    pub changes: broadcast::Sender<&'static str>,
    pub sessions: Arc<Mutex<HashSet<String>>>,
    /// Сервер учётных записей; `None` — программа работает сама по себе.
    pub hub: Option<Arc<Hub>>,
    /// Растёт при каждом изменении данных: интерфейс, которому недоступен поток
    /// событий, узнаёт о переменах по этому числу.
    revision: Arc<AtomicU64>,
    bot: Arc<Mutex<BotStatus>>,
}

impl AppState {
    pub fn new(db: Db, cfg: Config) -> Self {
        AppState {
            db: Arc::new(db),
            hub: Hub::from_config(&cfg).map(Arc::new),
            cfg: Arc::new(cfg),
            changes: broadcast::channel(64).0,
            sessions: Arc::default(),
            revision: Arc::default(),
            bot: Arc::default(),
        }
    }

    /// Смещение местного времени от UTC в минутах (настройка `utc_offset_minutes`, по умолчанию Ташкент, +5:00).
    pub fn utc_offset(&self) -> i64 {
        self.db
            .setting("utc_offset_minutes")
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300)
    }

    /// Текущее местное время.
    pub fn now(&self) -> NaiveDateTime {
        Utc::now().naive_utc() + Duration::minutes(self.utc_offset())
    }

    /// Сообщает открытым интерфейсам, что данные изменились.
    pub fn notify(&self) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        let _ = self.changes.send("change");
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    /// Расчёт месяца по правилам этого запуска: с общими учётными записями
    /// сообщения бота засчитываются только по привязанному Telegram ID.
    pub fn engine(&self) -> anyhow::Result<Engine<'_>> {
        Ok(Engine::new(&self.db)?.strict_tg_identity(self.cfg.hub_mode()))
    }

    pub fn bot_status<R>(&self, f: impl FnOnce(&mut BotStatus) -> R) -> R {
        f(&mut self.bot.lock().unwrap_or_else(|e| e.into_inner()))
    }
}
