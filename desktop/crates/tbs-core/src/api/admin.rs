//! Team administration: accounts, global switches, locked periods, live sessions.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Api, ApiError};
use crate::model::null_default;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct AdminUser {
    pub id: i64,
    #[serde(deserialize_with = "null_default")]
    pub username: String,
    #[serde(deserialize_with = "null_default")]
    pub display_name: String,
    #[serde(deserialize_with = "null_default")]
    pub role: String,
    #[serde(deserialize_with = "null_default")]
    pub is_active: bool,
    #[serde(deserialize_with = "null_default")]
    pub allowed_tabs: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub allowed_folders: Vec<i64>,
    #[serde(deserialize_with = "null_default")]
    pub forbidden_periods: Vec<Value>,
    #[serde(deserialize_with = "null_default")]
    pub allowed_sources: Vec<i64>,
    #[serde(deserialize_with = "null_default")]
    pub can_toggle_sources: bool,
    #[serde(deserialize_with = "null_default")]
    pub permissions_version: i64,
    #[serde(deserialize_with = "null_default")]
    pub force_2fa: bool,
    #[serde(deserialize_with = "null_default")]
    pub totp_enabled: bool,
    pub totp_confirmed_at: Option<String>,
    pub telegram_id: Option<i64>,
    pub telegram_username: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub session_ttl_days: i64,
    pub access_until: Option<String>,
    pub access_note: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub failed_attempts: i64,
    pub locked_until: Option<String>,
    pub last_login_at: Option<String>,
}

impl AdminUser {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    /// The name people recognise, falling back to the login.
    pub fn title(&self) -> &str {
        match self.display_name.trim() {
            "" => &self.username,
            name => name,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SystemSettings {
    pub id: i64,
    #[serde(deserialize_with = "null_default")]
    pub otp_enabled: bool,
    #[serde(deserialize_with = "null_default")]
    pub bot_control_enabled: bool,
    #[serde(deserialize_with = "null_default")]
    pub totp_2fa_enabled: bool,
    #[serde(deserialize_with = "null_default")]
    pub launch_gate_enabled: bool,
    pub updated_at: Option<String>,
    pub updated_by: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PeriodRecord {
    pub id: i64,
    #[serde(deserialize_with = "null_default")]
    pub date_from: String,
    #[serde(deserialize_with = "null_default")]
    pub date_to: String,
    pub reason: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub is_active: bool,
    pub locked_by_tg_id: Option<i64>,
    pub created_at: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub excluded_user_ids: Vec<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ActiveSession {
    #[serde(deserialize_with = "null_default")]
    pub session_id: String,
    #[serde(deserialize_with = "null_default")]
    pub token_kind: String,
    pub subject: Option<String>,
    pub user_id: Option<i64>,
    pub ip_address: Option<String>,
    pub created_at: Option<String>,
    pub last_activity_at: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct KilledSessions {
    #[serde(deserialize_with = "null_default")]
    pub ok: bool,
    #[serde(deserialize_with = "null_default")]
    pub killed: i64,
}

/// The fields both the create and the edit form send.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct NewUser {
    pub username: String,
    pub password: String,
    pub role: String,
    pub display_name: String,
    pub is_active: bool,
    pub allowed_tabs: Vec<String>,
    pub allowed_folders: Vec<i64>,
    pub forbidden_periods: Vec<Value>,
    pub allowed_sources: Vec<i64>,
    pub can_toggle_sources: bool,
    pub force_2fa: bool,
    pub session_ttl_days: i64,
    pub telegram_id: Option<i64>,
    pub telegram_username: Option<String>,
}

impl Api {
    pub async fn admin_users(&self) -> Result<Vec<AdminUser>, ApiError> {
        self.get("/api/admin/users", &[("include_inactive", "true".into())]).await
    }

    pub async fn create_user(&self, user: &NewUser) -> Result<AdminUser, ApiError> {
        self.post("/api/admin/users", user).await
    }

    pub async fn update_user(&self, id: i64, changes: &Value) -> Result<AdminUser, ApiError> {
        self.patch(&format!("/api/admin/users/{id}"), changes).await
    }

    pub async fn reset_password(&self, id: i64, password: &str) -> Result<AdminUser, ApiError> {
        self.post(&format!("/api/admin/users/{id}/reset-password"), &json!({ "password": password })).await
    }

    pub async fn reset_two_factor(&self, id: i64) -> Result<AdminUser, ApiError> {
        self.post(&format!("/api/admin/users/{id}/reset-2fa"), &json!({})).await
    }

    pub async fn force_two_factor(&self, id: i64, force: bool) -> Result<AdminUser, ApiError> {
        self.post(&format!("/api/admin/users/{id}/force-2fa"), &json!({ "force": force })).await
    }

    pub async fn unlock_user(&self, id: i64) -> Result<AdminUser, ApiError> {
        self.post(&format!("/api/admin/users/{id}/unlock"), &json!({})).await
    }

    pub async fn deactivate_user(&self, id: i64) -> Result<AdminUser, ApiError> {
        self.delete(&format!("/api/admin/users/{id}")).await
    }

    pub async fn link_telegram(&self, id: i64, telegram_id: i64, username: Option<&str>) -> Result<AdminUser, ApiError> {
        let body = json!({ "telegram_id": telegram_id, "telegram_username": username });
        self.post(&format!("/api/admin/users/{id}/link-telegram"), &body).await
    }

    pub async fn unlink_telegram(&self, id: i64) -> Result<AdminUser, ApiError> {
        self.post(&format!("/api/admin/users/{id}/unlink-telegram"), &json!({})).await
    }

    pub async fn system_settings(&self) -> Result<SystemSettings, ApiError> {
        self.get("/api/system-settings", &[]).await
    }

    pub async fn update_system_settings(&self, changes: &Value) -> Result<SystemSettings, ApiError> {
        self.patch("/api/system-settings", changes).await
    }

    pub async fn periods(&self) -> Result<Vec<PeriodRecord>, ApiError> {
        self.get("/api/periods", &[("active_only", "false".into())]).await
    }

    pub async fn create_period(&self, date_from: &str, date_to: &str, reason: Option<&str>) -> Result<PeriodRecord, ApiError> {
        let body = json!({ "date_from": date_from, "date_to": date_to, "reason": reason, "excluded_user_ids": [] });
        self.post("/api/periods", &body).await
    }

    pub async fn update_period(&self, id: i64, changes: &Value) -> Result<PeriodRecord, ApiError> {
        self.patch(&format!("/api/periods/{id}"), changes).await
    }

    pub async fn deactivate_period(&self, id: i64) -> Result<Value, ApiError> {
        self.delete(&format!("/api/periods/{id}")).await
    }

    pub async fn sessions(&self) -> Result<Vec<ActiveSession>, ApiError> {
        self.get("/api/security/sessions", &[("limit", "200".into())]).await
    }

    pub async fn kill_session(&self, session_id: &str) -> Result<Value, ApiError> {
        self.delete(&format!("/api/security/sessions/{}", escaped(session_id))).await
    }

    pub async fn kill_other_sessions(&self, current: &str) -> Result<KilledSessions, ApiError> {
        self.post("/api/security/sessions/kill-all-except-current", &json!({ "except_session_id": current })).await
    }
}

/// Session ids are opaque, so they cannot go into a path as they are.
fn escaped(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_survives_empty_fields() {
        let raw = r#"{"id":4,"username":"anna","display_name":null,"role":"operator","is_active":true,
            "allowed_tabs":["dashboard"],"allowed_folders":null,"forbidden_periods":[{"from":"2026-01-01","to":"2026-02-01"}],
            "allowed_sources":[],"can_toggle_sources":false,"permissions_version":3,"force_2fa":false,"totp_enabled":true,
            "totp_confirmed_at":null,"telegram_id":123,"telegram_username":null,"session_ttl_days":7,"access_until":null,
            "access_note":null,"failed_attempts":0,"locked_until":null,"last_login_at":null}"#;
        let user: AdminUser = serde_json::from_str(raw).unwrap();
        assert_eq!(user.title(), "anna");
        assert!(user.allowed_folders.is_empty());
        assert_eq!(user.forbidden_periods.len(), 1);
        assert!(!user.is_admin());
    }

    #[test]
    fn a_period_keeps_its_exclusions() {
        let raw = r#"{"id":1,"date_from":"2026-01-01","date_to":"2026-03-31","reason":null,"is_active":true,
            "locked_by_tg_id":null,"created_at":null,"excluded_user_ids":[2,5]}"#;
        let period: PeriodRecord = serde_json::from_str(raw).unwrap();
        assert_eq!(period.excluded_user_ids, vec![2, 5]);
    }

    #[test]
    fn a_session_id_is_safe_in_a_path() {
        assert_eq!(escaped("abc-1_2.3~"), "abc-1_2.3~");
        assert_eq!(escaped("a/b c"), "a%2Fb%20c");
    }
}
