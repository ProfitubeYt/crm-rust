//! What the receipt worker did, and who touched what.

use serde::Deserialize;
use serde_json::Value;

use super::{Api, ApiError};
use crate::model::null_default;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct LogEntry {
    pub id: i64,
    #[serde(deserialize_with = "null_default")]
    pub task_id: String,
    pub chat_id: i64,
    pub message_id: i64,
    #[serde(deserialize_with = "null_default")]
    pub status: String,
    pub error: Option<String>,
    pub transaction_id: Option<i64>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub is_duplicate: bool,
    pub operator_raw: Option<String>,
    pub amount: Option<String>,
    pub raw_message: Option<String>,
    pub fingerprint: Option<String>,
    pub currency: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LogPage {
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
    #[serde(deserialize_with = "null_default")]
    pub items: Vec<LogEntry>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LogStats {
    pub total: i64,
    pub success: i64,
    pub failed: i64,
    pub duplicates: i64,
    pub processing: i64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct AuditRecord {
    pub id: i64,
    #[serde(deserialize_with = "null_default")]
    pub action: String,
    #[serde(deserialize_with = "null_default")]
    pub success: bool,
    pub scope_id: Option<i64>,
    pub user_id: Option<i64>,
    pub ip_address: Option<String>,
    pub details: Option<Value>,
    pub created_at: Option<String>,
}

impl Api {
    pub async fn log_stats(&self) -> Result<LogStats, ApiError> {
        self.get("/api/logs/stats", &[]).await
    }

    pub async fn logs(&self, query: &[(&str, String)]) -> Result<LogPage, ApiError> {
        self.get("/api/logs", query).await
    }

    pub async fn audit(&self, query: &[(&str, String)]) -> Result<Vec<AuditRecord>, ApiError> {
        self.get("/api/audit", query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_line_survives_empty_fields() {
        let raw = r#"{"id":7,"task_id":"t","chat_id":-100,"message_id":5,"status":"dead","error":null,"transaction_id":null,
            "created_at":"2026-10-19T08:00:00","updated_at":null,"is_duplicate":false,"operator_raw":null,"amount":null}"#;
        let entry: LogEntry = serde_json::from_str(raw).unwrap();
        assert_eq!(entry.chat_id, -100);
        assert_eq!(entry.status, "dead");
        assert!(entry.error.is_none());
    }

    #[test]
    fn an_audit_line_keeps_its_details() {
        let raw = r#"{"id":1,"action":"login","success":true,"scope_id":null,"user_id":3,"ip_address":null,"details":{"a":1},"created_at":null}"#;
        let record: AuditRecord = serde_json::from_str(raw).unwrap();
        assert_eq!(record.user_id, Some(3));
        assert_eq!(record.details.unwrap()["a"], 1);
    }
}
