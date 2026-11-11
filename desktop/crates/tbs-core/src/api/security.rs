//! Access scopes: which years a user may open, and the codes that open them.

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Api, ApiError};
use crate::model::null_default;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct AccessScope {
    pub id: i64,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub years: Vec<i32>,
    pub period_start: Option<String>,
    pub period_end: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub allow_transactions: bool,
    #[serde(deserialize_with = "null_default")]
    pub allow_sources: bool,
    #[serde(deserialize_with = "null_default")]
    pub is_active: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AccessStatus {
    #[serde(deserialize_with = "null_default")]
    pub scopes_enabled: bool,
    #[serde(deserialize_with = "null_default")]
    pub available_years: Vec<i32>,
    pub current_scope: Option<AccessScope>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct UnlockedScope {
    #[serde(deserialize_with = "null_default")]
    pub token: String,
    pub expires_at: Option<String>,
    pub scope: Option<AccessScope>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ScopeCode {
    #[serde(deserialize_with = "null_default")]
    pub status: String,
    #[serde(deserialize_with = "null_default")]
    pub expires_in: i64,
    #[serde(deserialize_with = "null_default")]
    pub code_length: i64,
    #[serde(deserialize_with = "null_default")]
    pub otp_skipped: bool,
    pub token: Option<String>,
    pub expires_at: Option<String>,
    pub scope: Option<AccessScope>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LockedPeriod {
    pub id: i64,
    #[serde(deserialize_with = "null_default")]
    pub date_from: String,
    #[serde(deserialize_with = "null_default")]
    pub date_to: String,
    pub reason: Option<String>,
    pub locked_at: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LockedPeriods {
    #[serde(deserialize_with = "null_default")]
    periods: Vec<LockedPeriod>,
}

impl Api {
    pub async fn access_status(&self) -> Result<AccessStatus, ApiError> {
        self.get("/api/security/status", &[]).await
    }

    pub async fn access_scopes(&self) -> Result<Vec<AccessScope>, ApiError> {
        self.get("/api/security/scopes", &[]).await
    }

    pub async fn create_scope(&self, body: &Value) -> Result<AccessScope, ApiError> {
        self.post("/api/security/scopes", body).await
    }

    pub async fn update_scope(&self, id: i64, body: &Value) -> Result<AccessScope, ApiError> {
        self.put(&format!("/api/security/scopes/{id}"), body).await
    }

    pub async fn delete_scope(&self, id: i64) -> Result<Value, ApiError> {
        self.delete(&format!("/api/security/scopes/{id}")).await
    }

    /// Asks the server to send a code. A token in the answer means OTP is off
    /// for this scope and it is already open.
    pub async fn request_scope_code(&self, scope_id: i64) -> Result<ScopeCode, ApiError> {
        let reply: ScopeCode = self.post(&format!("/api/security/scope/{scope_id}/request-code"), &json!({})).await?;
        if let Some(token) = &reply.token {
            self.set_scope_token(Some(token.clone()));
        }
        Ok(reply)
    }

    pub async fn verify_scope_code(&self, scope_id: i64, code: &str) -> Result<UnlockedScope, ApiError> {
        let reply: UnlockedScope = self.post(&format!("/api/security/scope/{scope_id}/verify-code"), &json!({ "code": code })).await?;
        if !reply.token.is_empty() {
            self.set_scope_token(Some(reply.token.clone()));
        }
        Ok(reply)
    }

    pub async fn locked_periods(&self) -> Result<Vec<LockedPeriod>, ApiError> {
        let reply: LockedPeriods = self.get("/api/security/locked-periods", &[]).await?;
        Ok(reply.periods)
    }
}
