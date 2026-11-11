//! Two-factor protection of the signed-in account.

use iced::widget::{Column, Space, button, center, column, container, qr_code, row, text};
use iced::{Alignment, Color, Fill, Shrink, Task, Theme};
use tbs_core::api::{TwoFactorConfirm, TwoFactorSetup, TwoFactorStatus};
use tbs_core::model::UserInfo;

use crate::ctx::{Ctx, text_error};
use crate::theme::{self, Tone, size, space};
use crate::ui::{self, El, Kind};

use super::panel;

const CODE_LENGTH: usize = 6;
/// The server takes a TOTP or a backup code here, so the limits are its own.
const STEP_CODE_MIN: usize = 4;
const STEP_CODE_MAX: usize = 32;

#[derive(Clone, Debug)]
pub enum Msg {
    Status(Result<TwoFactorStatus, String>),
    Me(Result<UserInfo, String>),
    StartSetup,
    Prepared(Result<TwoFactorSetup, String>),
    Code(String),
    Confirm,
    Confirmed(Result<TwoFactorConfirm, String>),
    ToggleBackup,
    CloseSetup,
    Ask(Purpose),
    StepCode(String),
    StepSubmit,
    StepCancel,
    Disabled(Result<serde_json::Value, String>),
    GotCodes(Result<Vec<String>, String>),
    CloseCodes,
}

/// What a code is being asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Disable,
    NewCodes,
}

struct StepUp {
    purpose: Purpose,
    code: String,
    checking: bool,
    error: Option<String>,
}

#[derive(Default)]
struct Setup {
    data: Option<TwoFactorSetup>,
    qr: Option<qr_code::Data>,
    failed: bool,
    code: String,
    checking: bool,
    backup_open: bool,
}

#[derive(Default)]
pub struct Security {
    status: Option<TwoFactorStatus>,
    setup: Option<Setup>,
    step_up: Option<StepUp>,
    fresh_codes: Option<Vec<String>>,
}

fn digits_only(raw: &str) -> String {
    raw.chars().filter(char::is_ascii_digit).take(CODE_LENGTH).collect()
}

fn step_code_only(raw: &str) -> String {
    raw.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(STEP_CODE_MAX).collect()
}

fn code_grid<'a>(codes: &'a [String]) -> El<'a, Msg> {
    let mut lines = Column::new().spacing(space::XS);
    for pair in codes.chunks(2) {
        let cell = |value: Option<&'a String>| container(text(value.map(String::as_str).unwrap_or("")).font(theme::FONT_MONO).size(size::SM)).width(Fill);
        lines = lines.push(row![cell(pair.first()), cell(pair.get(1))].spacing(space::MD));
    }
    container(lines).padding(space::SM).width(Fill).style(theme::inset).into()
}

impl Security {
    pub fn escape(&mut self) {
        self.setup = None;
        self.step_up = None;
        self.fresh_codes = None;
    }

    /// A refused code leaves the dialog open so the person can type it again.
    fn step_failed(&mut self, reason: String) {
        if let Some(step) = &mut self.step_up {
            step.checking = false;
            step.code.clear();
            step.error = Some(reason);
        }
    }

    pub fn load(&self, ctx: &Ctx) -> Task<Msg> {
        let api = ctx.api.clone();
        Task::perform(async move { api.two_factor_status().await.map_err(text_error) }, Msg::Status)
    }

    fn refresh_user(ctx: &Ctx) -> Task<Msg> {
        let api = ctx.api.clone();
        Task::perform(async move { api.me().await.map_err(text_error) }, Msg::Me)
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Status(Ok(status)) => self.status = Some(status),
            Msg::Status(Err(reason)) => ctx.toast(Tone::Danger, reason),
            Msg::Me(Ok(user)) => ctx.user = Some(user),
            Msg::Me(Err(reason)) => tracing::warn!("profile not refreshed: {reason}"),
            Msg::StartSetup => {
                self.setup = Some(Setup::default());
                let api = ctx.api.clone();
                return Task::perform(async move { api.two_factor_setup(None).await.map_err(text_error) }, Msg::Prepared);
            }
            Msg::Prepared(Ok(data)) => {
                if let Some(setup) = &mut self.setup {
                    setup.qr = qr_code::Data::new(data.otpauth_uri.as_bytes()).ok();
                    setup.data = Some(data);
                }
            }
            Msg::Prepared(Err(reason)) => {
                tracing::warn!("2fa setup failed: {reason}");
                if let Some(setup) = &mut self.setup {
                    setup.failed = true;
                }
                ctx.toast(Tone::Danger, "Не удалось начать настройку");
            }
            Msg::Code(raw) => {
                if let Some(setup) = &mut self.setup {
                    setup.code = digits_only(&raw);
                }
            }
            Msg::Confirm => {
                if let Some(setup) = &mut self.setup
                    && setup.code.len() == CODE_LENGTH
                    && !setup.checking
                {
                    setup.checking = true;
                    let (api, code) = (ctx.api.clone(), setup.code.clone());
                    return Task::perform(async move { api.two_factor_confirm(&code, None).await.map_err(text_error) }, Msg::Confirmed);
                }
            }
            Msg::Confirmed(reply) => {
                if let Some(setup) = &mut self.setup {
                    setup.checking = false;
                }
                match reply {
                    Ok(answer) if answer.ok => {
                        self.setup = None;
                        ctx.toast(Tone::Success, "Двухфакторная аутентификация включена");
                        return Task::batch([self.load(ctx), Self::refresh_user(ctx)]);
                    }
                    Ok(_) => ctx.toast(Tone::Danger, "Неверный код подтверждения"),
                    Err(_) => ctx.toast(Tone::Danger, "Не удалось подтвердить код"),
                }
            }
            Msg::ToggleBackup => {
                if let Some(setup) = &mut self.setup {
                    setup.backup_open = !setup.backup_open;
                }
            }
            Msg::CloseSetup => self.setup = None,
            Msg::Ask(purpose) => self.step_up = Some(StepUp { purpose, code: String::new(), checking: false, error: None }),
            Msg::StepCode(raw) => {
                if let Some(step) = &mut self.step_up {
                    step.code = step_code_only(&raw);
                    step.error = None;
                }
            }
            Msg::StepSubmit => {
                if let Some(step) = &mut self.step_up
                    && step.code.len() >= STEP_CODE_MIN
                    && !step.checking
                {
                    step.checking = true;
                    let (api, code) = (ctx.api.clone(), step.code.clone());
                    return match step.purpose {
                        Purpose::Disable => Task::perform(async move { api.two_factor_disable(&code).await.map_err(text_error) }, Msg::Disabled),
                        Purpose::NewCodes => Task::perform(
                            async move { api.two_factor_backup_codes(&code).await.map(|reply| reply.backup_codes).map_err(text_error) },
                            Msg::GotCodes,
                        ),
                    };
                }
            }
            Msg::StepCancel => self.step_up = None,
            Msg::Disabled(Ok(_)) => {
                self.step_up = None;
                ctx.toast(Tone::Success, "Двухфакторная аутентификация отключена");
                return Task::batch([self.load(ctx), Self::refresh_user(ctx)]);
            }
            Msg::GotCodes(Ok(codes)) => {
                self.step_up = None;
                self.fresh_codes = Some(codes);
                ctx.toast(Tone::Success, "Резервные коды обновлены");
            }
            Msg::Disabled(Err(reason)) | Msg::GotCodes(Err(reason)) => self.step_failed(reason),
            Msg::CloseCodes => self.fresh_codes = None,
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, ctx: &Ctx) -> El<'a, Msg> {
        let forced = ctx.user.as_ref().is_some_and(|u| u.force_2fa) || self.status.is_some_and(|s| s.force_2fa);
        let enabled = self.status.map(|s| s.enabled);
        let hint = match enabled {
            None => "Загружаем статус…".to_owned(),
            Some(true) => "Защищает аккаунт одноразовыми кодами из приложения-аутентификатора.".to_owned(),
            Some(false) if forced => "Не настроена. Рекомендуется включить для защиты аккаунта. · Принудительно администратором".to_owned(),
            Some(false) => "Не настроена. Рекомендуется включить для защиты аккаунта.".to_owned(),
        };
        let pill = match enabled {
            None => ui::badge("проверяем", Tone::Neutral),
            Some(true) => ui::badge("активна", Tone::Success),
            Some(false) => ui::badge("отключена", Tone::Warning),
        };
        let actions: El<'a, Msg> = match enabled {
            None => Space::new().into(),
            Some(true) => {
                let codes = ui::btn("Перегенерировать резервные коды", Kind::Secondary, Some(Msg::Ask(Purpose::NewCodes)));
                let off = ui::btn("Отключить 2FA", Kind::Danger, (!forced).then_some(Msg::Ask(Purpose::Disable)));
                let off: El<'a, Msg> = if forced { ui::hint_on(off, "2FA принудительна — отключение запрещено") } else { off.into() };
                row![codes, off].spacing(space::SM).into()
            }
            Some(false) => {
                let busy = self.setup.is_some();
                ui::btn(if busy { "Подготавливаем" } else { "Настроить 2FA" }, Kind::Primary, (!busy).then_some(Msg::StartSetup)).into()
            }
        };
        panel("Двухфакторная аутентификация", hint, pill, actions)
    }

    fn setup_dialog<'a>(&'a self, setup: &'a Setup) -> El<'a, Msg> {
        let mut body = column![
            ui::dialog_title("Настройка 2FA", Some(Msg::CloseSetup)),
            ui::muted("Введите 6-значный код из приложения для подтверждения."),
        ]
        .spacing(space::MD);
        match (&setup.data, setup.failed) {
            (_, true) => body = body.push(ui::error_line("Не удалось загрузить настройку. Попробуйте ещё раз.")),
            (None, _) => body = body.push(ui::muted("Готовим QR-код…")),
            (Some(data), _) => {
                if let Some(qr) = &setup.qr {
                    let code = container(qr_code(qr).total_size(196)).padding(space::SM).style(|_: &Theme| container::Style { background: Some(Color::WHITE.into()), ..Default::default() });
                    body = body.push(center(code).height(Shrink));
                }
                body = body
                    .push(ui::muted("Или введите ключ вручную:"))
                    .push(container(text(data.secret.as_str()).font(theme::FONT_MONO).size(size::SM)).padding(space::SM).width(Fill).style(theme::inset))
                    .push(ui::field("Код", ui::input("000000", &setup.code).on_input(Msg::Code).on_submit(Msg::Confirm)));
                if !data.backup_codes.is_empty() {
                    let glyph = if setup.backup_open { ui::Icon::ChevronUp } else { ui::Icon::ChevronDown };
                    let caption = format!("Резервные коды ({})", data.backup_codes.len());
                    body = body.push(button(row![ui::icon(glyph, 14.0), ui::label(caption)].spacing(space::XS).align_y(Alignment::Center)).padding(0).style(theme::btn_link).on_press(Msg::ToggleBackup));
                    if setup.backup_open {
                        body = body.push(code_grid(&data.backup_codes)).push(ui::muted("Сохраните эти коды — они понадобятся при потере доступа к приложению."));
                    }
                }
            }
        }
        let ready = setup.code.len() == CODE_LENGTH && !setup.checking;
        let actions = row![
            ui::fill(),
            ui::btn("Отмена", Kind::Secondary, Some(Msg::CloseSetup)),
            ui::btn(if setup.checking { "Проверяем" } else { "Подтвердить" }, Kind::Primary, ready.then_some(Msg::Confirm)),
        ]
        .spacing(space::SM);
        ui::modal(Space::new(), body.push(actions), 480.0, Some(Msg::CloseSetup))
    }

    fn step_up_dialog<'a>(&'a self, step: &'a StepUp) -> El<'a, Msg> {
        let (head, hint, action, kind) = match step.purpose {
            Purpose::Disable => ("Отключить 2FA?", "Аккаунт будет защищён только паролем. Для подтверждения введите код из приложения или резервный код.", "Отключить", Kind::Danger),
            Purpose::NewCodes => ("Новые резервные коды", "Старые коды перестанут работать. Для подтверждения введите код из приложения или резервный код.", "Сгенерировать", Kind::Primary),
        };
        let mut body = column![
            ui::dialog_title(head, Some(Msg::StepCancel)),
            ui::muted(hint),
            ui::field("Код", ui::input("000000", &step.code).on_input(Msg::StepCode).on_submit(Msg::StepSubmit)),
        ]
        .spacing(space::MD);
        if let Some(reason) = &step.error {
            body = body.push(ui::error_line(reason.as_str()));
        }
        let ready = step.code.len() >= STEP_CODE_MIN && !step.checking;
        let actions = row![
            ui::fill(),
            ui::btn("Отмена", Kind::Secondary, Some(Msg::StepCancel)),
            ui::btn(if step.checking { "Проверяем" } else { action }, kind, ready.then_some(Msg::StepSubmit)),
        ]
        .spacing(space::SM);
        ui::modal(Space::new(), body.push(actions), 440.0, Some(Msg::StepCancel))
    }

    pub fn overlay(&self) -> Option<El<'_, Msg>> {
        if let Some(setup) = &self.setup {
            return Some(self.setup_dialog(setup));
        }
        if let Some(step) = &self.step_up {
            return Some(self.step_up_dialog(step));
        }
        self.fresh_codes.as_ref().map(|codes| {
            let body = column![
                ui::dialog_title("Новые резервные коды", Some(Msg::CloseCodes)),
                ui::muted("Сохраните коды в надёжном месте. Старые коды больше не работают."),
                code_grid(codes),
                row![ui::fill(), ui::btn("Готово", Kind::Primary, Some(Msg::CloseCodes))],
            ]
            .spacing(space::MD);
            ui::modal(Space::new(), body, 400.0, Some(Msg::CloseCodes))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_keeps_six_digits_and_nothing_else() {
        assert_eq!(digits_only("12a 34-5678"), "123456");
        assert_eq!(digits_only("код"), "");
    }

    #[test]
    fn a_step_code_takes_a_backup_code_too() {
        assert_eq!(step_code_only(" ab12-CD34 "), "ab12-CD34");
        assert_eq!(step_code_only(&"7".repeat(40)).len(), STEP_CODE_MAX);
    }

    fn security_with_step(purpose: Purpose, ctx: &mut Ctx) -> Security {
        let mut screen = Security::default();
        let _ = screen.update(Msg::Ask(purpose), ctx);
        let _ = screen.update(Msg::StepCode("123456".into()), ctx);
        screen
    }

    #[test]
    fn a_refused_code_keeps_the_dialog_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = Ctx::for_tests(dir.path());
        let mut screen = security_with_step(Purpose::Disable, &mut ctx);
        let _ = screen.update(Msg::StepSubmit, &mut ctx);
        assert!(screen.step_up.as_ref().unwrap().checking);

        let _ = screen.update(Msg::Disabled(Err("Неверный код (осталось: 4)".into())), &mut ctx);
        let step = screen.step_up.as_ref().expect("the dialog closed");
        assert!(!step.checking && step.code.is_empty());
        assert_eq!(step.error.as_deref(), Some("Неверный код (осталось: 4)"));

        let _ = screen.update(Msg::StepCode("1".into()), &mut ctx);
        assert_eq!(screen.step_up.as_ref().unwrap().error, None);
    }

    #[test]
    fn a_short_code_is_not_sent() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = Ctx::for_tests(dir.path());
        let mut screen = security_with_step(Purpose::NewCodes, &mut ctx);
        let _ = screen.update(Msg::StepCode("123".into()), &mut ctx);
        let _ = screen.update(Msg::StepSubmit, &mut ctx);
        assert!(!screen.step_up.as_ref().unwrap().checking);
    }

    #[test]
    fn new_codes_replace_the_dialog() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = Ctx::for_tests(dir.path());
        let mut screen = security_with_step(Purpose::NewCodes, &mut ctx);
        let _ = screen.update(Msg::GotCodes(Ok(vec!["aaaa-bbbb".into()])), &mut ctx);
        assert!(screen.step_up.is_none());
        assert_eq!(screen.fresh_codes.as_deref(), Some(&["aaaa-bbbb".to_string()][..]));
    }
}
