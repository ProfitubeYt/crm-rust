//! Login, session and launch-gate calls.

use reqwest::Method;
use serde::Deserialize;
use serde_json::json;

use super::{Api, ApiError, Call};
use crate::model::{UserInfo, null_default};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LoginResponse {
    #[serde(deserialize_with = "null_default")]
    pub ok: bool,
    pub token: Option<String>,
    pub scope_token: Option<String>,
    pub refresh_token: Option<String>,
    pub user: Option<UserInfo>,
    pub error: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub requires_2fa: bool,
    #[serde(deserialize_with = "null_default")]
    pub requires_2fa_setup: bool,
    pub temp_token: Option<String>,
    pub attempts_left: Option<i64>,
    pub locked_seconds: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Entitlement {
    #[serde(deserialize_with = "null_default")]
    pub ok: bool,
    #[serde(deserialize_with = "null_default")]
    pub expired: bool,
    pub allowed_until: Option<String>,
    pub days_left: Option<i64>,
    pub note: Option<String>,
    pub username: Option<String>,
    pub role: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub allowed_tabs: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub permissions_version: i64,
    pub checked_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LaunchStatus {
    #[serde(deserialize_with = "null_default")]
    pub password_set: bool,
    #[serde(deserialize_with = "null_default")]
    pub locked: bool,
    pub locked_until: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LaunchVerify {
    #[serde(deserialize_with = "null_default")]
    pub verified: bool,
    pub session_token: Option<String>,
    pub expires_at: Option<String>,
    pub attempts_left: Option<i64>,
    pub locked: Option<bool>,
    pub locked_until: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct QrSession {
    pub session_id: String,
    pub qr_url: String,
    pub expires_in: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct QrStatus {
    /// `pending`, `confirmed` or `expired`.
    pub status: String,
    pub token: Option<String>,
    pub scope_token: Option<String>,
    pub refresh_token: Option<String>,
    pub user: Option<UserInfo>,
}

/// Another sign-in, or a sign-out, finished while this one was on its way.
/// Its tokens are not kept, so its answer must not read as a sign-in either:
/// the window would name one account while the session belonged to another.
fn overtaken() -> ApiError {
    ApiError::Http { status: 409, code: Some("sign_in_overtaken".to_owned()), message: "Вход отменён: сессия изменилась. Войдите ещё раз".to_owned() }
}

impl Api {
    fn accept_login(&self, reply: LoginResponse, epoch: u64) -> Result<LoginResponse, ApiError> {
        if reply.ok && reply.token.is_some() && !self.begin_session(epoch, reply.token.clone(), reply.refresh_token.clone(), reply.scope_token.clone()) {
            return Err(overtaken());
        }
        Ok(reply)
    }

    /// A rejected password is a 200 or a 401 with `ok:false`; both are normal
    /// answers the login form needs to read, not failures.
    async fn login_call(&self, call: Call) -> Result<LoginResponse, ApiError> {
        let epoch = self.epoch();
        let reply = self.execute(&call).await?;
        if let Ok(parsed) = serde_json::from_slice::<LoginResponse>(&reply.bytes) {
            if (200..300).contains(&reply.status) || parsed.error.is_some() || parsed.locked_seconds.is_some() {
                return self.accept_login(parsed, epoch);
            }
        }
        Err(super::error_from_body(reply.status, &reply.bytes))
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<LoginResponse, ApiError> {
        let call = Call::new(Method::POST, "/api/auth/login").json(&json!({ "username": username, "password": password }));
        self.login_call(call).await
    }

    pub async fn login_2fa(&self, temp_token: &str, code: &str) -> Result<LoginResponse, ApiError> {
        let call = Call::new(Method::POST, "/api/auth/login/2fa").json(&json!({ "temp_token": temp_token, "code": code }));
        self.login_call(call).await
    }

    /// Restores a session from the stored refresh token at startup.
    pub async fn resume(&self) -> Result<Option<UserInfo>, ApiError> {
        let Some(token) = self.refresh_token() else { return Ok(None) };
        let epoch = self.epoch();
        let call = Call::new(Method::POST, "/api/auth/refresh")
            .json(&json!({ "refresh_token": token }))
            .header("X-Refresh-Token", token);
        let reply = self.execute(&call).await?;
        if reply.status >= 500 {
            return Err(super::error_from_body(reply.status, &reply.bytes));
        }
        let parsed: LoginResponse = serde_json::from_slice(&reply.bytes).unwrap_or_default();
        if reply.status == 200 && parsed.ok && parsed.token.is_some() {
            let user = parsed.user.clone();
            self.accept_login(parsed, epoch)?;
            return match user {
                Some(u) => Ok(Some(u)),
                None => self.me().await.map(Some),
            };
        }
        // The saved session was refused; one begun meanwhile is not it.
        if self.epoch() == epoch {
            self.clear_session();
        }
        Ok(None)
    }

    pub async fn me(&self) -> Result<UserInfo, ApiError> {
        self.get("/api/auth/me", &[]).await
    }

    pub async fn logout(&self) {
        if let Some(token) = self.refresh_token() {
            let call = Call::new(Method::POST, "/api/auth/logout")
                .json(&json!({ "refresh_token": token }))
                .header("X-Refresh-Token", token);
            let _ = self.execute(&call).await;
        }
        self.clear_session();
    }

    pub async fn entitlement(&self) -> Result<Entitlement, ApiError> {
        self.get("/api/auth/entitlement", &[]).await
    }

    pub async fn launch_status(&self) -> Result<LaunchStatus, ApiError> {
        self.get("/api/security/app/launch-status", &[]).await
    }

    pub async fn verify_launch(&self, password: &str) -> Result<LaunchVerify, ApiError> {
        let reply: LaunchVerify = self.post("/api/security/app/verify-launch", &json!({ "password": password })).await?;
        if reply.verified {
            self.set_launch_token(reply.session_token.clone());
        }
        Ok(reply)
    }

    pub async fn qr_generate(&self) -> Result<QrSession, ApiError> {
        self.post("/api/auth/qr/generate", &json!({})).await
    }

    /// The server hands the tokens out once, on the first poll after confirmation.
    pub async fn qr_status(&self, session_id: &str) -> Result<QrStatus, ApiError> {
        let epoch = self.epoch();
        let reply: QrStatus = self.get(&format!("/api/auth/qr/status/{session_id}"), &[]).await?;
        if reply.status == "confirmed" && reply.token.is_some() && !self.begin_session(epoch, reply.token.clone(), reply.refresh_token.clone(), reply.scope_token.clone()) {
            return Err(overtaken());
        }
        Ok(reply)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct TwoFactorSetup {
    pub secret: String,
    pub otpauth_uri: String,
    pub qr_code_base64: String,
    #[serde(deserialize_with = "null_default")]
    pub backup_codes: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct TwoFactorConfirm {
    #[serde(deserialize_with = "null_default")]
    pub ok: bool,
    #[serde(deserialize_with = "null_default")]
    pub enabled: bool,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TwoFactorStatus {
    #[serde(deserialize_with = "null_default")]
    pub enabled: bool,
    #[serde(deserialize_with = "null_default")]
    pub force_2fa: bool,
    #[serde(deserialize_with = "null_default")]
    pub global_policy_enabled: bool,
    #[serde(deserialize_with = "null_default")]
    pub required_now: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct BackupCodes {
    #[serde(deserialize_with = "null_default")]
    pub ok: bool,
    #[serde(deserialize_with = "null_default")]
    pub backup_codes: Vec<String>,
}

impl Api {
    pub async fn two_factor_status(&self) -> Result<TwoFactorStatus, ApiError> {
        self.get("/api/2fa/status", &[]).await
    }

    pub async fn two_factor_setup(&self, temp_token: Option<&str>) -> Result<TwoFactorSetup, ApiError> {
        self.post("/api/2fa/setup", &json!({ "temp_token": temp_token })).await
    }

    /// 400/401/429 are ordinary answers here: a wrong code is not a failure of the call.
    pub async fn two_factor_confirm(&self, code: &str, temp_token: Option<&str>) -> Result<TwoFactorConfirm, ApiError> {
        let call = Call::new(Method::POST, "/api/2fa/confirm").json(&json!({ "code": code, "temp_token": temp_token }));
        let reply = self.execute(&call).await?;
        match reply.status {
            200 | 400 | 401 | 429 => serde_json::from_slice(&reply.bytes).or(Ok(TwoFactorConfirm::default())),
            status => Err(super::error_from_body(status, &reply.bytes)),
        }
    }

    /// `code` is a fresh TOTP or a backup code: the server asks for one again here.
    pub async fn two_factor_backup_codes(&self, code: &str) -> Result<BackupCodes, ApiError> {
        self.post("/api/2fa/backup-codes", &json!({ "code": code })).await
    }

    /// `code` is a fresh TOTP or a backup code, as for new backup codes.
    pub async fn two_factor_disable(&self, code: &str) -> Result<serde_json::Value, ApiError> {
        self.send(Call::new(Method::DELETE, "/api/2fa").json(&json!({ "code": code }))).await
    }
}
