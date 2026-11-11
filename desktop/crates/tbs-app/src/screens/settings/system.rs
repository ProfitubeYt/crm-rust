//! Global switches, the bookkeeping workbook and the rules learned from it.

use std::path::PathBuf;
use std::time::Duration;

use iced::widget::{Column, Row, column, row};
use iced::{Subscription, Task};
use serde_json::json;
use tbs_core::api::{AnnotationReport, Api, RecomputeReply, RuleApplyReply, RuleCoverage, SystemSettings};
use tbs_core::format::{self, datetime_str};

use crate::ctx::{Ctx, Request, text_error};
use crate::files;
use crate::theme::{Tone, space};
use crate::ui::{self, El, Icon, Kind};

use super::workbook::{self, Settling, WatchState};
use super::{panel, setting_row};

const WATCH_EVERY: Duration = Duration::from_secs(3);
const SOURCE_MANUAL: &str = "manual_upload";
const SOURCE_WATCHER: &str = "native_watcher";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Otp,
    BotControl,
    TwoFactor,
    LaunchGate,
}

impl Switch {
    const ALL: [Self; 4] = [Self::Otp, Self::BotControl, Self::TwoFactor, Self::LaunchGate];

    fn key(self) -> &'static str {
        match self {
            Self::Otp => "otp_enabled",
            Self::BotControl => "bot_control_enabled",
            Self::TwoFactor => "totp_2fa_enabled",
            Self::LaunchGate => "launch_gate_enabled",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Otp => "OTP по scope",
            Self::BotControl => "Управление ботами",
            Self::TwoFactor => "Глобальная 2FA",
            Self::LaunchGate => "Gate запуска",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Otp => "Запрос одноразового кода при доступе к чувствительным разделам.",
            Self::BotControl => "Если выключено — операторы не могут запускать и останавливать ботов.",
            Self::TwoFactor => "Принудительно включить 2FA для всех пользователей при следующем входе.",
            Self::LaunchGate => "Дополнительная проверка перед запуском задач. Снижает риск случайного запуска.",
        }
    }

    fn state_word(self, on: bool) -> &'static str {
        match (self, on) {
            (_, true) => "Включено",
            (Self::BotControl, false) => "Только чтение",
            (_, false) => "Выключено",
        }
    }

    fn read(self, settings: &SystemSettings) -> bool {
        match self {
            Self::Otp => settings.otp_enabled,
            Self::BotControl => settings.bot_control_enabled,
            Self::TwoFactor => settings.totp_2fa_enabled,
            Self::LaunchGate => settings.launch_gate_enabled,
        }
    }
}

/// How one pass over the workbook ended.
#[derive(Clone, Debug)]
pub enum Outcome {
    Unchanged,
    Conflicts(AnnotationReport),
    NoMatch(AnnotationReport),
    Applied { report: AnnotationReport, sha256: String },
}

#[derive(Clone, Debug)]
pub enum Msg {
    Loaded(Result<SystemSettings, String>),
    Toggle(Switch, bool),
    Saved(Switch, Result<SystemSettings, String>),
    Pick,
    Picked(Option<PathBuf>),
    ToggleWatch,
    ImportNow,
    Overwrite,
    Watch,
    Imported(Result<Outcome, String>),
    RuleCount(Result<usize, String>),
    Coverage(Result<RuleCoverage, String>),
    Recompute,
    Recomputed(Result<RecomputeReply, String>),
    Preview,
    Previewed(Result<RuleApplyReply, String>),
    Apply,
    Applied(Result<RuleApplyReply, String>),
}

pub struct System {
    settings: Option<SystemSettings>,
    saving: Option<Switch>,
    watch: WatchState,
    settling: Settling,
    importing: bool,
    /// The file settled again while a pass was running.
    rerun: bool,
    report: Option<AnnotationReport>,
    conflicts: Option<i64>,
    rule_count: Option<usize>,
    coverage: Option<RuleCoverage>,
    preview: Option<RuleApplyReply>,
    rules_busy: bool,
}

async fn read_book(path: PathBuf) -> Result<workbook::Book, String> {
    tokio::task::spawn_blocking(move || workbook::read_waiting(&path)).await.map_err(text_error)?
}

/// Reads the book, tries it on the server and applies it unless something
/// needs a person to decide.
async fn import(api: Api, path: PathBuf, overwrite: bool, source: &'static str, known: Option<String>) -> Result<Outcome, String> {
    let book = read_book(path).await?;
    if known.as_deref() == Some(book.sha256.as_str()) {
        return Ok(Outcome::Unchanged);
    }
    let trial = api.import_annotations(&book.name, book.bytes.clone(), true, overwrite, source).await.map_err(text_error)?;
    if !overwrite && trial.report.counts.conflicts > 0 {
        return Ok(Outcome::Conflicts(trial.report));
    }
    if trial.report.matched == 0 {
        return Ok(Outcome::NoMatch(trial.report));
    }
    let done = api.import_annotations(&book.name, book.bytes, false, overwrite, source).await.map_err(text_error)?;
    Ok(Outcome::Applied { report: done.report, sha256: book.sha256 })
}

fn stat_cell<'a>(caption: &'a str, value: String) -> El<'a, Msg> {
    column![ui::faint(caption), ui::strong(value)].spacing(space::XXS).width(130.0).into()
}

fn share(value: Option<f64>) -> String {
    value.map_or_else(|| format::EMPTY.to_owned(), |part| format!("{}%", (part * 100.0).round()))
}

impl System {
    pub fn new(ctx: &Ctx) -> Self {
        let watch = WatchState::load(&ctx.data_dir);
        let settling = if watch.enabled { Settling::startup() } else { Settling::default() };
        Self {
            settings: None,
            saving: None,
            watch,
            settling,
            importing: false,
            rerun: false,
            report: None,
            conflicts: None,
            rule_count: None,
            coverage: None,
            preview: None,
            rules_busy: false,
        }
    }

    /// Runs on every screen: the book is saved while the table is open.
    pub fn watch(&self) -> Subscription<Msg> {
        if self.watch.enabled && self.watch.file_path.is_some() {
            iced::time::every(WATCH_EVERY).map(|_| Msg::Watch)
        } else {
            Subscription::none()
        }
    }

    pub fn load(&self, ctx: &Ctx) -> Task<Msg> {
        let (settings, rules, coverage) = (ctx.api.clone(), ctx.api.clone(), ctx.api.clone());
        Task::batch([
            Task::perform(async move { settings.system_settings().await.map_err(text_error) }, Msg::Loaded),
            Task::perform(async move { rules.annotation_rules().await.map(|list| list.len()).map_err(text_error) }, Msg::RuleCount),
            Task::perform(async move { coverage.rule_coverage().await.map_err(text_error) }, Msg::Coverage),
        ])
    }

    fn start_import(&mut self, ctx: &Ctx, overwrite: bool, source: &'static str, known: Option<String>) -> Task<Msg> {
        let Some(path) = self.watch.file_path.clone() else { return Task::none() };
        if self.importing {
            return Task::none();
        }
        self.importing = true;
        self.conflicts = None;
        Task::perform(import(ctx.api.clone(), path, overwrite, source, known), Msg::Imported)
    }

    fn finish_import(&mut self, result: Result<Outcome, String>, ctx: &mut Ctx) -> Task<Msg> {
        self.importing = false;
        match result {
            Ok(Outcome::Unchanged) => {}
            Ok(Outcome::Conflicts(report)) => {
                self.conflicts = Some(report.counts.conflicts);
                self.report = Some(report);
            }
            Ok(Outcome::NoMatch(report)) => {
                self.report = Some(report);
                ctx.toast(Tone::Warning, "Ни одна строка книги не сопоставилась с транзакциями.");
            }
            Ok(Outcome::Applied { report, sha256 }) => {
                ctx.toast(Tone::Success, format!("Разметка обновлена: {} транзакций.", report.applied));
                self.report = Some(report);
                self.watch.last_sha256 = Some(sha256);
                self.watch.last_run_at = Some(chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string());
                self.watch.save(&ctx.data_dir);
                ctx.request(Request::TransactionsChanged);
            }
            Err(reason) if reason.is_empty() => ctx.toast(Tone::Danger, "Импорт не удался."),
            Err(reason) => ctx.toast(Tone::Danger, reason),
        }
        if std::mem::take(&mut self.rerun) {
            let known = self.watch.last_sha256.clone();
            return self.start_import(ctx, false, SOURCE_WATCHER, known);
        }
        Task::none()
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Loaded(Ok(settings)) => self.settings = Some(settings),
            Msg::Loaded(Err(reason)) => ctx.toast(Tone::Danger, reason),
            Msg::Toggle(switch, on) => {
                if self.saving.is_some() {
                    return Task::none();
                }
                self.saving = Some(switch);
                let api = ctx.api.clone();
                let changes = json!({ switch.key(): on });
                return Task::perform(async move { api.update_system_settings(&changes).await.map_err(text_error) }, move |reply| Msg::Saved(switch, reply));
            }
            Msg::Saved(switch, reply) => {
                self.saving = None;
                match reply {
                    Ok(settings) => {
                        self.settings = Some(settings);
                        ctx.toast(Tone::Success, "Системные настройки обновлены");
                        if switch == Switch::Otp {
                            ctx.api.set_scope_token(None);
                            ctx.request(Request::TransactionsChanged);
                        }
                    }
                    Err(_) => ctx.toast(Tone::Danger, "Ошибка обновления"),
                }
            }
            Msg::Pick => return Task::perform(async { files::pick_path(&["xlsx", "xlsm"]).await }, Msg::Picked),
            Msg::Picked(None) => {}
            Msg::Picked(Some(path)) => {
                self.watch.file_path = Some(path);
                self.watch.last_sha256 = None;
                self.watch.last_run_at = None;
                self.watch.save(&ctx.data_dir);
                self.report = None;
                self.conflicts = None;
                self.settling = Settling::startup();
            }
            Msg::ToggleWatch => {
                self.watch.enabled = !self.watch.enabled;
                self.watch.save(&ctx.data_dir);
                self.settling = Settling::startup();
            }
            Msg::ImportNow => return self.start_import(ctx, false, SOURCE_MANUAL, None),
            Msg::Overwrite => return self.start_import(ctx, true, SOURCE_MANUAL, None),
            Msg::Watch => {
                let Some(path) = self.watch.file_path.as_deref() else { return Task::none() };
                if !self.watch.enabled || !self.settling.observe(workbook::signature(path)) {
                    return Task::none();
                }
                if self.importing {
                    self.rerun = true;
                    return Task::none();
                }
                let known = self.watch.last_sha256.clone();
                return self.start_import(ctx, false, SOURCE_WATCHER, known);
            }
            Msg::Imported(result) => return self.finish_import(result, ctx),
            Msg::RuleCount(Ok(count)) => self.rule_count = Some(count),
            Msg::Coverage(Ok(coverage)) => self.coverage = Some(coverage),
            Msg::RuleCount(Err(reason)) | Msg::Coverage(Err(reason)) => tracing::warn!("rules not loaded: {reason}"),
            Msg::Recompute => {
                self.rules_busy = true;
                let api = ctx.api.clone();
                return Task::perform(async move { api.recompute_rules().await.map_err(text_error) }, Msg::Recomputed);
            }
            Msg::Recomputed(reply) => {
                self.rules_busy = false;
                match reply {
                    Ok(answer) => {
                        self.preview = None;
                        ctx.toast(Tone::Success, format!("Правил активно: {} (новых {})", answer.total_enabled, answer.created));
                        return self.load(ctx);
                    }
                    Err(_) => ctx.toast(Tone::Danger, "Не удалось пересчитать правила"),
                }
            }
            Msg::Preview => {
                self.rules_busy = true;
                let api = ctx.api.clone();
                return Task::perform(async move { api.apply_rules(true).await.map_err(text_error) }, Msg::Previewed);
            }
            Msg::Previewed(reply) => {
                self.rules_busy = false;
                match reply {
                    Ok(answer) => {
                        self.preview = Some(answer);
                        ctx.toast(Tone::Neutral, format!("Будет дозаполнено строк: {}", answer.updated));
                    }
                    Err(_) => ctx.toast(Tone::Danger, "Не удалось применить правила"),
                }
            }
            Msg::Apply => {
                self.rules_busy = true;
                let api = ctx.api.clone();
                return Task::perform(async move { api.apply_rules(false).await.map_err(text_error) }, Msg::Applied);
            }
            Msg::Applied(reply) => {
                self.rules_busy = false;
                match reply {
                    Ok(answer) => {
                        self.preview = None;
                        ctx.toast(Tone::Success, format!("Дозаполнено строк: {}", answer.updated));
                        ctx.request(Request::TransactionsChanged);
                        return self.load(ctx);
                    }
                    Err(_) => ctx.toast(Tone::Danger, "Не удалось применить правила"),
                }
            }
        }
        Task::none()
    }

    fn switches(&self) -> El<'_, Msg> {
        let hint = "Глобальные переключатели — влияют на всех пользователей.";
        let Some(settings) = &self.settings else {
            return panel("Системные тогглы", hint, iced::widget::Space::new(), ui::loading("Загружаем настройки…"));
        };
        let mut lines = Column::new().spacing(space::MD);
        for switch in Switch::ALL {
            let on = switch.read(settings);
            let mut control = ui::switch(on, move |value| Msg::Toggle(switch, value));
            if self.saving.is_some() {
                control = control.on_toggle_maybe(None::<fn(bool) -> Msg>);
            }
            let side = row![ui::muted(switch.state_word(on)), control].spacing(space::SM).align_y(iced::Alignment::Center);
            lines = lines.push(setting_row(switch.name(), switch.hint(), side));
        }
        let stamp: El<'_, Msg> = match (&settings.updated_at, settings.updated_by) {
            (Some(at), Some(by)) => ui::badge(format!("обновлено · {} · user #{by}", datetime_str(Some(at.as_str()))), Tone::Neutral),
            (Some(at), None) => ui::badge(format!("обновлено · {}", datetime_str(Some(at.as_str()))), Tone::Neutral),
            _ => iced::widget::Space::new().into(),
        };
        panel("Системные тогглы", hint, stamp, lines)
    }

    fn book(&self) -> El<'_, Msg> {
        let hint = "Подробности, лицо, период и статья подтягиваются из вашего Excel-файла. Программа следит за ним и обновляет разметку после каждого сохранения.";
        let has_file = self.watch.file_path.is_some();
        let name = self.watch.file_name().unwrap_or_else(|| "Файл не выбран".to_owned());
        let mut file = column![ui::strong(name)].spacing(space::XXS);
        if let Some(at) = &self.watch.last_run_at {
            file = file.push(ui::faint(format!("Последняя загрузка: {}", datetime_str(Some(at.as_str())))));
        }
        let watch_caption = if self.watch.enabled { "Остановить слежение" } else { "Следить за файлом" };
        let import_caption = if self.importing { "Импорт…" } else { "Загрузить сейчас" };
        let actions = row![
            ui::btn_icon(Icon::FileUp, "Выбрать файл", Kind::Secondary, Some(Msg::Pick)),
            ui::btn(watch_caption, Kind::Secondary, has_file.then_some(Msg::ToggleWatch)),
            ui::btn(import_caption, Kind::Primary, (has_file && !self.importing).then_some(Msg::ImportNow)),
        ]
        .spacing(space::SM);
        let mut body = column![file, actions].spacing(space::MD);
        if let Some(conflicts) = self.conflicts {
            let warning = ui::notice(Tone::Warning, format!("Найдено расхождений с ручной правкой: {conflicts}. Импорт не применён."));
            body = body.push(warning).push(ui::btn("Применить, перезаписав ручные правки", Kind::Danger, (!self.importing).then_some(Msg::Overwrite)));
        }
        if let Some(report) = &self.report {
            let cells: [(&str, i64); 7] = [
                ("Строк в книге", report.row_count),
                ("Сопоставлено", report.matched),
                ("Обновлено", report.applied),
                ("Без изменений", report.skipped_same),
                ("Не найдено", report.counts.unmatched),
                ("Неоднозначных", report.counts.ambiguous),
                ("Конфликтов", report.counts.conflicts),
            ];
            let mut stats = Row::new().spacing(space::MD);
            for (caption, value) in cells {
                stats = stats.push(stat_cell(caption, value.to_string()));
            }
            body = body.push(ui::divider()).push(stats.wrap());
        }
        let aside: El<'_, Msg> = if self.watch.enabled { ui::badge("слежение включено", Tone::Success) } else { iced::widget::Space::new().into() };
        panel("Книга учёта", hint, aside, body)
    }

    fn rules(&self) -> El<'_, Msg> {
        let hint = "Правило запоминает, какое значение вы обычно ставите этому оператору, и подставляет его новым чекам. Ручные правки не затрагиваются.";
        let predictable = |pick: fn(&RuleCoverage) -> Option<f64>| self.coverage.as_ref().and_then(pick);
        let stats = row![
            stat_cell("Активных правил", self.rule_count.map_or_else(|| format::EMPTY.to_owned(), |count| count.to_string())),
            stat_cell("Статья · предсказуемо", share(predictable(|c| c.article.as_ref()?.predictable()))),
            stat_cell("Лицо · предсказуемо", share(predictable(|c| c.person.as_ref()?.predictable()))),
            stat_cell("Подробности · предсказуемо", share(predictable(|c| c.details.as_ref()?.predictable()))),
        ]
        .spacing(space::MD);
        let idle = !self.rules_busy;
        let apply = ui::btn("Дозаполнить", Kind::Primary, (idle && self.preview.is_some()).then_some(Msg::Apply));
        let apply: El<'_, Msg> = if self.preview.is_none() { ui::hint_on(apply, "Сначала посмотрите предпросмотр") } else { apply.into() };
        let actions = row![
            ui::btn_icon(Icon::Refresh, "Пересчитать правила", Kind::Secondary, idle.then_some(Msg::Recompute)),
            ui::btn_icon(Icon::Eye, "Предпросмотр", Kind::Secondary, idle.then_some(Msg::Preview)),
            apply,
        ]
        .spacing(space::SM);
        let mut body = column![stats].spacing(space::MD);
        if let Some(preview) = &self.preview {
            body = body.push(ui::muted(format!("Предпросмотр: просмотрено {}, будет изменено строк {} (полей {}).", preview.scanned, preview.updated, preview.changed_fields)));
        }
        panel("Правила разметки", hint, iced::widget::Space::new(), body.push(actions))
    }

    pub fn view<'a>(&'a self, _ctx: &Ctx) -> El<'a, Msg> {
        column![self.switches(), self.book(), self.rules()].spacing(space::MD).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_read_their_own_field() {
        let settings = SystemSettings { otp_enabled: true, launch_gate_enabled: true, ..Default::default() };
        assert!(Switch::Otp.read(&settings));
        assert!(!Switch::BotControl.read(&settings));
        assert!(!Switch::TwoFactor.read(&settings));
        assert!(Switch::LaunchGate.read(&settings));
    }

    #[test]
    fn bot_control_off_means_read_only() {
        assert_eq!(Switch::BotControl.state_word(false), "Только чтение");
        assert_eq!(Switch::Otp.state_word(false), "Выключено");
        assert_eq!(Switch::Otp.state_word(true), "Включено");
    }

    #[test]
    fn a_missing_share_is_a_dash_not_zero() {
        assert_eq!(share(None), format::EMPTY);
        assert_eq!(share(Some(0.876)), "88%");
    }
}
