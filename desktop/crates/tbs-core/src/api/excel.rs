//! Excel workbook connections: grants, passwords and the workbook itself.

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Api, ApiError};
use crate::model::null_default;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExcelGrant {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    pub created_at: Option<String>,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    pub workbook_id: Option<String>,
    pub last_used_at: Option<String>,
    pub device_name: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub device_bound: bool,
}

/// The owner's password that lets a new computer open a shared workbook.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExcelActivation {
    #[serde(deserialize_with = "null_default")]
    pub workbook_id: String,
    pub updated_at: Option<String>,
    pub locked_until: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ExcelGrantCreated {
    pub grant: ExcelGrant,
    #[serde(deserialize_with = "null_default")]
    pub token: String,
    #[serde(deserialize_with = "null_default")]
    pub server_url: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Grants {
    #[serde(deserialize_with = "null_default")]
    grants: Vec<ExcelGrant>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Activations {
    #[serde(deserialize_with = "null_default")]
    activations: Vec<ExcelActivation>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ActivationReply {
    activation: ExcelActivation,
}

impl Api {
    pub async fn excel_grants(&self) -> Result<Vec<ExcelGrant>, ApiError> {
        let reply: Grants = self.get("/api/excel/grants", &[]).await?;
        Ok(reply.grants)
    }

    pub async fn excel_issue_grant(&self, name: &str, expires_in_days: u32) -> Result<ExcelGrantCreated, ApiError> {
        self.post("/api/excel/grants", &json!({ "name": name, "expires_in_days": expires_in_days })).await
    }

    pub async fn excel_revoke_grant(&self, id: &str) -> Result<(), ApiError> {
        let _: Value = self.delete(&format!("/api/excel/grants/{id}")).await?;
        Ok(())
    }

    pub async fn excel_activations(&self) -> Result<Vec<ExcelActivation>, ApiError> {
        let reply: Activations = self.get("/api/excel/activations", &[]).await?;
        Ok(reply.activations)
    }

    pub async fn excel_set_password(&self, workbook_id: &str, password: &str) -> Result<ExcelActivation, ApiError> {
        let reply: ActivationReply = self.put("/api/excel/activations", &json!({ "workbook_id": workbook_id, "password": password })).await?;
        Ok(reply.activation)
    }

    pub async fn excel_remove_password(&self, workbook_id: &str) -> Result<(), ApiError> {
        let _: Value = self.delete(&format!("/api/excel/activations/{workbook_id}")).await?;
        Ok(())
    }

    pub async fn excel_workbook(&self) -> Result<Vec<u8>, ApiError> {
        let (_, bytes) = self.download("/api/excel/workbook", &[]).await?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_reads_with_nulls() {
        let grant: ExcelGrant = serde_json::from_str(r#"{"id":"12","name":null,"device_bound":null,"workbook_id":"w"}"#).unwrap();
        assert_eq!(grant.id, "12");
        assert_eq!(grant.name, "");
        assert!(!grant.device_bound);
    }
}
