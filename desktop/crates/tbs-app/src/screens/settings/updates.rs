//! The running version and the way to a newer one.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use iced::futures::{SinkExt, Stream, channel::mpsc};
use iced::widget::{Space, column, progress_bar, row};
use iced::{Alignment, Task};
use tbs_core::updates::{self, Release};

use crate::ctx::Ctx;
use crate::theme::{self, Tone, space};
use crate::ui::{self, El, Icon, Kind};

use super::panel;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, Default, PartialEq)]
enum Stage {
    #[default]
    Idle,
    Checking,
    Current,
    Available(Release),
    Downloading(Release, f32),
    Ready(PathBuf),
    Failed(String),
}

#[derive(Clone, Debug)]
pub enum Msg {
    Check,
    Checked(Result<Option<Release>, String>),
    Download,
    Progress(Progress),
    Install,
}

#[derive(Clone, Debug)]
pub enum Progress {
    Share(f32),
    Done(Result<PathBuf, String>),
}

#[derive(Default)]
pub struct Updates {
    stage: Stage,
}

fn fetch(release: Release, dir: PathBuf) -> impl Stream<Item = Progress> {
    iced::stream::channel(32, async move |mut out: mpsc::Sender<Progress>| {
        let shares = Arc::new(Mutex::new(Vec::<f32>::new()));
        let sink = shares.clone();
        let mut download = Box::pin(updates::download(&release, &dir, move |got, total| {
            if total > 0 {
                sink.lock().unwrap().push(got as f32 / total as f32);
            }
        }));
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(200));
        let result = loop {
            tokio::select! {
                done = &mut download => break done,
                _ = tick.tick() => {
                    let latest = shares.lock().unwrap().drain(..).last();
                    if let Some(share) = latest {
                        let _ = out.send(Progress::Share(share)).await;
                    }
                }
            }
        };
        let _ = out.send(Progress::Done(result)).await;
    })
}

impl Updates {
    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Check => {
                self.stage = Stage::Checking;
                return Task::perform(updates::check(VERSION), Msg::Checked);
            }
            Msg::Checked(Ok(Some(release))) => self.stage = Stage::Available(release),
            Msg::Checked(Ok(None)) => self.stage = Stage::Current,
            Msg::Checked(Err(reason)) => self.stage = Stage::Failed(reason),
            Msg::Download => {
                if let Stage::Available(release) = &self.stage {
                    let release = release.clone();
                    self.stage = Stage::Downloading(release.clone(), 0.0);
                    return Task::stream(fetch(release, ctx.data_dir.join("updates"))).map(Msg::Progress);
                }
            }
            Msg::Progress(Progress::Share(share)) => {
                if let Stage::Downloading(_, progress) = &mut self.stage {
                    *progress = share;
                }
            }
            Msg::Progress(Progress::Done(Ok(path))) => self.stage = Stage::Ready(path),
            Msg::Progress(Progress::Done(Err(reason))) => self.stage = Stage::Failed(reason),
            Msg::Install => {
                if let Stage::Ready(path) = &self.stage {
                    match std::process::Command::new(path).spawn() {
                        Ok(_) => return iced::exit(),
                        Err(error) => {
                            tracing::error!("installer did not start: {error}");
                            ctx.toast(Tone::Danger, "Не удалось запустить установщик");
                        }
                    }
                }
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, _ctx: &Ctx) -> El<'a, Msg> {
        let (pill, line): (El<'a, Msg>, String) = match &self.stage {
            Stage::Idle => (ui::badge("готово", Tone::Neutral), "Нажмите «Проверить», чтобы найти новую версию.".to_owned()),
            Stage::Checking => (ui::badge("проверка", Tone::Neutral), "Проверка обновлений…".to_owned()),
            Stage::Current => (ui::badge("актуально", Tone::Success), "Установлена последняя версия.".to_owned()),
            Stage::Available(release) => (ui::badge("есть обновление", Tone::Accent), format!("Найдена новая версия {}.", release.version)),
            Stage::Downloading(_, share) => (ui::badge("загрузка", Tone::Accent), format!("Скачивание… {}%", (share * 100.0).round())),
            Stage::Ready(_) => (ui::badge("готово к установке", Tone::Success), "Обновление скачано. Можно установить.".to_owned()),
            Stage::Failed(reason) => (ui::badge("ошибка", Tone::Danger), reason.clone()),
        };
        let checking = matches!(self.stage, Stage::Checking | Stage::Downloading(..));
        let mut actions = row![ui::btn_icon(Icon::Refresh, "Проверить", Kind::Secondary, (!checking).then_some(Msg::Check))].spacing(space::SM);
        if matches!(self.stage, Stage::Available(_)) {
            actions = actions.push(ui::btn_icon(Icon::Download, "Скачать", Kind::Primary, Some(Msg::Download)));
        }
        if matches!(self.stage, Stage::Ready(_)) {
            actions = actions.push(ui::btn("Установить и перезапустить", Kind::Primary, Some(Msg::Install)));
        }
        let mut body = column![ui::muted(line), actions].spacing(space::MD);
        if let Stage::Downloading(_, share) = &self.stage {
            body = body.push(progress_bar(0.0..=1.0, *share).style(theme::progress));
        }
        let aside = row![ui::badge(format!("v{VERSION}"), Tone::Neutral), pill].spacing(space::XS).align_y(Alignment::Center);
        column![panel("Версия приложения", "Новая версия скачивается в фоне, установка перезапускает программу.", aside, body), Space::new()].into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_to_download_means_no_download() {
        let mut screen = Updates { stage: Stage::Current };
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = crate::ctx::Ctx::for_tests(dir.path());
        let _ = screen.update(Msg::Download, &mut ctx);
        assert_eq!(screen.stage, Stage::Current);
    }

    #[test]
    fn a_failed_download_shows_why() {
        let mut screen = Updates { stage: Stage::Downloading(Release::default(), 0.5) };
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = crate::ctx::Ctx::for_tests(dir.path());
        let _ = screen.update(Msg::Progress(Progress::Done(Err("сеть".into()))), &mut ctx);
        assert_eq!(screen.stage, Stage::Failed("сеть".into()));
    }
}
