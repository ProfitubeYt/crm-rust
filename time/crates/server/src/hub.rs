//! Общие учётные записи (Hub).
//!
//! Сам «Тайм-лист» учётных записей не ведёт. Когда заданы `TIMELIST_HUB_URL` и
//! `TIMELIST_HUB_KEY`, вход, роли и доступ принадлежат серверу учётных записей:
//! интерфейс и API доступны только через его шлюз (по `TIMELIST_API_TOKEN`), а бот
//! перед каждой отметкой спрашивает у него, привязан ли отправитель к действующей
//! учётной записи с доступом к учёту времени. Ответ не запоминается: отключённая
//! запись перестаёт приниматься со следующего же сообщения.

use std::fmt;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;

use crate::config::Config;

/// Почему отправителю отказано. Причину бот сообщает ему самому в ответ на `/me`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Denial {
    /// Telegram ID не указан ни в одной учётной записи.
    NotLinked,
    /// Учётная запись отключена.
    Inactive,
    /// У учётной записи нет доступа к учёту времени.
    NoTimeAccess,
    /// Срок действия учётной записи истёк.
    Expired,
    /// Причина, которой эта версия не знает: отказ остаётся отказом.
    Other,
}

impl Denial {
    fn from_reason(reason: &str) -> Self {
        match reason {
            "not_linked" => Denial::NotLinked,
            "inactive" => Denial::Inactive,
            "no_time_access" => Denial::NoTimeAccess,
            "access_expired" => Denial::Expired,
            _ => Denial::Other,
        }
    }

    /// Коротко, для журнала и строки состояния бота.
    pub fn label(&self) -> &'static str {
        match self {
            Denial::NotLinked => "Telegram не привязан к учётной записи",
            Denial::Inactive => "учётная запись отключена",
            Denial::NoTimeAccess => "нет доступа к учёту времени",
            Denial::Expired => "срок действия учётной записи истёк",
            Denial::Other => "доступ не подтверждён",
        }
    }

    /// Ответ самому отправителю.
    pub fn explain(&self, tg_user_id: i64) -> String {
        match self {
            Denial::NotLinked => format!(
                "Ваш Telegram ID {tg_user_id} не привязан к учётной записи. Передайте его администратору: \
                 он укажет ID в вашей учётной записи и откроет доступ к учёту времени."
            ),
            Denial::Inactive => {
                "Ваша учётная запись отключена, отметки не принимаются. Обратитесь к администратору.".into()
            }
            Denial::NoTimeAccess => {
                "У вашей учётной записи нет доступа к учёту времени. Обратитесь к администратору.".into()
            }
            Denial::Expired => {
                "Срок действия вашей учётной записи истёк. Обратитесь к администратору.".into()
            }
            Denial::Other => "Доступ к учёту времени не подтверждён. Обратитесь к администратору.".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    Denied(Denial),
}

/// Сервер учётных записей не ответил или ответил непонятно. Это не отказ и не
/// разрешение: сообщение остаётся у Telegram и будет прочитано снова.
#[derive(Debug)]
pub struct HubUnavailable(pub String);

impl fmt::Display for HubUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "сервер учётных записей недоступен: {}", self.0)
    }
}

impl std::error::Error for HubUnavailable {}

#[derive(Deserialize)]
struct Answer {
    allowed: bool,
    #[serde(default)]
    reason: String,
}

pub struct Hub {
    http: reqwest::Client,
    /// Адрес сервера учётных записей без завершающей косой черты.
    url: String,
    /// В журнал не выводится.
    key: String,
}

impl Hub {
    /// `None`, если режим общих учётных записей не включён.
    pub fn from_config(cfg: &Config) -> Option<Hub> {
        let (url, key) = (cfg.hub_url.as_deref()?, cfg.hub_key.as_deref()?);
        Some(Hub {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(8))
                // переменные окружения с прокси не должны увести ключ на сторону
                .no_proxy()
                .build()
                .expect("клиент HTTP создаётся"),
            url: url.trim_end_matches('/').to_string(),
            key: key.to_string(),
        })
    }

    /// Может ли отправитель с этим Telegram ID сейчас отмечаться через бота.
    ///
    /// Ошибка означает «спросить ещё раз», а не «нельзя»: без ответа сервера
    /// отметка не принимается и не теряется.
    pub async fn check(&self, tg_user_id: i64) -> Result<Verdict> {
        let response = self
            .http
            .get(format!("{}/internal/time/telegram/{tg_user_id}", self.url))
            .header("X-Time-Hub-Key", &self.key)
            .send()
            .await
            .map_err(|e| anyhow!("{}", e.without_url()))?;
        let status = response.status();
        if !status.is_success() {
            bail!("ответ {}", status.as_u16());
        }
        let answer: Answer = response
            .json()
            .await
            .map_err(|e| anyhow!("{}", e.without_url()))?;
        Ok(if answer.allowed {
            Verdict::Allowed
        } else {
            Verdict::Denied(Denial::from_reason(&answer.reason))
        })
    }
}
