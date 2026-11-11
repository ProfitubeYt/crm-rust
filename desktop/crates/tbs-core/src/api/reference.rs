//! Operator reference and the annotation dictionaries.

use std::collections::BTreeMap;

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Api, ApiError, Call};
use crate::model::null_default;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct OperatorRef {
    pub id: i64,
    #[serde(deserialize_with = "null_default")]
    pub operator_name: String,
    #[serde(deserialize_with = "null_default")]
    pub application_name: String,
    #[serde(deserialize_with = "null_default")]
    pub is_p2p: bool,
    #[serde(deserialize_with = "null_default")]
    pub is_active: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct OperatorPage {
    #[serde(deserialize_with = "null_default")]
    pub total: i64,
    #[serde(deserialize_with = "null_default")]
    pub page: i64,
    #[serde(deserialize_with = "null_default")]
    pub page_size: i64,
    #[serde(deserialize_with = "null_default")]
    pub items: Vec<OperatorRef>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct AnnotationArticle {
    pub id: i64,
    #[serde(deserialize_with = "null_default")]
    pub code: String,
    #[serde(deserialize_with = "null_default")]
    pub label: String,
    #[serde(deserialize_with = "null_default")]
    pub direction: String,
    #[serde(deserialize_with = "null_default")]
    pub sort_order: i64,
    #[serde(deserialize_with = "null_default")]
    pub is_active: bool,
    #[serde(deserialize_with = "null_default")]
    pub is_auto: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct AnnotationPerson {
    pub id: i64,
    #[serde(deserialize_with = "null_default")]
    pub code: String,
    pub full_name: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub sort_order: i64,
    #[serde(deserialize_with = "null_default")]
    pub is_active: bool,
    #[serde(deserialize_with = "null_default")]
    pub is_auto: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ImportReply {
    #[serde(deserialize_with = "null_default")]
    pub imported: i64,
    #[serde(deserialize_with = "null_default")]
    pub skipped: i64,
    #[serde(deserialize_with = "null_default")]
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Description {
    pub id: i64,
    #[serde(deserialize_with = "null_default")]
    pub text: String,
    #[serde(deserialize_with = "null_default")]
    pub operator_keys: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub operator_count: i64,
    #[serde(deserialize_with = "null_default")]
    pub sources: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct DescriptionPage {
    #[serde(deserialize_with = "null_default")]
    pub total: i64,
    #[serde(deserialize_with = "null_default")]
    pub page: i64,
    #[serde(deserialize_with = "null_default")]
    pub page_size: i64,
    #[serde(deserialize_with = "null_default")]
    pub items: Vec<Description>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AnnotationCounts {
    #[serde(deserialize_with = "null_default")]
    pub conflicts: i64,
    #[serde(deserialize_with = "null_default")]
    pub ambiguous: i64,
    #[serde(deserialize_with = "null_default")]
    pub unmatched: i64,
    #[serde(deserialize_with = "null_default")]
    pub forbidden: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AnnotationReport {
    #[serde(deserialize_with = "null_default")]
    pub row_count: i64,
    #[serde(deserialize_with = "null_default")]
    pub matched: i64,
    #[serde(deserialize_with = "null_default")]
    pub applied: i64,
    #[serde(deserialize_with = "null_default")]
    pub skipped_same: i64,
    #[serde(deserialize_with = "null_default")]
    pub counts: AnnotationCounts,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AnnotationImport {
    #[serde(deserialize_with = "null_default")]
    pub ok: bool,
    pub batch_id: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub status: String,
    #[serde(deserialize_with = "null_default")]
    pub report: AnnotationReport,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AnnotationRule {
    pub id: i64,
    #[serde(deserialize_with = "null_default")]
    pub operator_key: String,
    #[serde(deserialize_with = "null_default")]
    pub field: String,
    #[serde(deserialize_with = "null_default")]
    pub is_enabled: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default)]
pub struct CoverageShare {
    pub rows: i64,
    pub pct: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct FieldCoverage {
    pub annotated_rows: i64,
    pub distinct_operators: i64,
    pub coverage: BTreeMap<String, CoverageShare>,
}

impl FieldCoverage {
    /// The share of rows whose operator repeats one value at least 90% of the time.
    pub fn predictable(&self) -> Option<f64> {
        self.coverage.get("0.9").map(|share| share.pct)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RuleCoverage {
    pub article: Option<FieldCoverage>,
    pub person: Option<FieldCoverage>,
    pub details: Option<FieldCoverage>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RecomputeReply {
    pub created: i64,
    pub updated: i64,
    pub disabled: i64,
    pub total_enabled: i64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RuleApplyReply {
    pub scanned: i64,
    pub updated: i64,
    pub changed_fields: i64,
    pub dry_run: bool,
}

#[derive(Deserialize)]
struct CoverageEnvelope {
    #[serde(default)]
    coverage: RuleCoverage,
}

impl Api {
    /// Everything at once; the list is small and feeds autocomplete.
    pub async fn all_operators(&self) -> Result<Vec<OperatorRef>, ApiError> {
        let page: OperatorPage = self.get("/api/reference", &[("all", "true".to_owned())]).await?;
        Ok(page.items)
    }

    pub async fn operators(&self, query: &[(&str, String)]) -> Result<OperatorPage, ApiError> {
        self.get("/api/reference", query).await
    }

    pub async fn create_operator(&self, body: &Value) -> Result<OperatorRef, ApiError> {
        self.post("/api/reference", body).await
    }

    pub async fn update_operator(&self, id: i64, body: &Value) -> Result<OperatorRef, ApiError> {
        self.put(&format!("/api/reference/{id}"), body).await
    }

    pub async fn delete_operator(&self, id: i64) -> Result<Value, ApiError> {
        self.delete(&format!("/api/reference/{id}")).await
    }

    pub async fn applications(&self) -> Result<Vec<String>, ApiError> {
        self.get("/api/reference/applications", &[]).await
    }

    pub async fn annotation_articles(&self, include_inactive: bool) -> Result<Vec<AnnotationArticle>, ApiError> {
        self.get("/api/reference/annotations/articles", &[("include_inactive", include_inactive.to_string())]).await
    }

    pub async fn annotation_persons(&self, include_inactive: bool) -> Result<Vec<AnnotationPerson>, ApiError> {
        self.get("/api/reference/annotations/persons", &[("include_inactive", include_inactive.to_string())]).await
    }

    pub async fn create_article(&self, code: &str, label: &str) -> Result<AnnotationArticle, ApiError> {
        self.post("/api/reference/annotations/articles", &json!({ "code": code, "label": label })).await
    }

    pub async fn update_article(&self, id: i64, body: &Value) -> Result<AnnotationArticle, ApiError> {
        self.patch(&format!("/api/reference/annotations/articles/{id}"), body).await
    }

    pub async fn delete_article(&self, id: i64) -> Result<Value, ApiError> {
        self.delete(&format!("/api/reference/annotations/articles/{id}")).await
    }

    pub async fn create_person(&self, code: &str, full_name: &str) -> Result<AnnotationPerson, ApiError> {
        self.post("/api/reference/annotations/persons", &json!({ "code": code, "full_name": full_name })).await
    }

    pub async fn update_person(&self, id: i64, body: &Value) -> Result<AnnotationPerson, ApiError> {
        self.patch(&format!("/api/reference/annotations/persons/{id}"), body).await
    }

    pub async fn delete_person(&self, id: i64) -> Result<Value, ApiError> {
        self.delete(&format!("/api/reference/annotations/persons/{id}")).await
    }

    pub async fn export_operators(&self) -> Result<(Option<String>, Vec<u8>), ApiError> {
        self.download("/api/reference/export/excel", &[]).await
    }

    pub async fn import_operators(&self, filename: &str, bytes: Vec<u8>) -> Result<ImportReply, ApiError> {
        self.upload("/api/reference/import/excel", &[], filename, bytes).await
    }

    pub async fn descriptions(&self, query: &[(&str, String)]) -> Result<DescriptionPage, ApiError> {
        self.get("/api/descriptions", query).await
    }

    pub async fn create_description(&self, text: &str, operators: &[String]) -> Result<Description, ApiError> {
        self.post("/api/descriptions", &json!({ "text": text, "operator_raws": operators, "source": "manual" })).await
    }

    pub async fn update_description(&self, id: i64, text: &str) -> Result<Description, ApiError> {
        self.patch(&format!("/api/descriptions/{id}"), &json!({ "text": text })).await
    }

    pub async fn delete_description(&self, id: i64) -> Result<Value, ApiError> {
        self.delete(&format!("/api/descriptions/{id}")).await
    }

    pub async fn link_operators(&self, id: i64, operators: &[String]) -> Result<Description, ApiError> {
        self.post(&format!("/api/descriptions/{id}/operators"), &json!({ "operator_raws": operators, "source": "manual" })).await
    }

    pub async fn unlink_operators(&self, id: i64, operators: &[String]) -> Result<Description, ApiError> {
        let path = format!("/api/descriptions/{id}/operators");
        self.send(Call::new(Method::DELETE, &path).json(&json!({ "operator_raws": operators }))).await
    }

    /// `source` tells the server whether a person or the file watcher sent the book.
    pub async fn import_annotations(
        &self,
        filename: &str,
        bytes: Vec<u8>,
        dry_run: bool,
        overwrite_manual: bool,
        source: &str,
    ) -> Result<AnnotationImport, ApiError> {
        let query = [
            ("dry_run", dry_run.to_string()),
            ("overwrite_manual", overwrite_manual.to_string()),
            ("source", source.to_owned()),
        ];
        self.upload("/api/transactions/annotations/import", &query, filename, bytes).await
    }

    pub async fn annotation_rules(&self) -> Result<Vec<AnnotationRule>, ApiError> {
        self.get("/api/transactions/annotations/rules", &[("limit", "5000".to_owned())]).await
    }

    pub async fn rule_coverage(&self) -> Result<RuleCoverage, ApiError> {
        let envelope: CoverageEnvelope = self.get("/api/transactions/annotations/rules/coverage", &[]).await?;
        Ok(envelope.coverage)
    }

    pub async fn recompute_rules(&self) -> Result<RecomputeReply, ApiError> {
        self.post("/api/transactions/annotations/rules/recompute", &json!({})).await
    }

    pub async fn apply_rules(&self, dry_run: bool) -> Result<RuleApplyReply, ApiError> {
        let call = Call::new(Method::POST, "/api/transactions/annotations/rules/apply").query(&[("dry_run", dry_run.to_string())]);
        self.send(call).await
    }
}
