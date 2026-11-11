//! The transactions endpoints.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{Api, ApiError};
use crate::model::{Transaction, TransactionPage, null_default};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct UpdateReply {
    #[serde(deserialize_with = "null_default")]
    pub ok: bool,
    #[serde(deserialize_with = "null_default")]
    pub success: bool,
    pub message: Option<String>,
    pub transaction: Option<Transaction>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct BulkReply {
    #[serde(deserialize_with = "null_default")]
    pub ok: bool,
    #[serde(deserialize_with = "null_default")]
    pub success: bool,
    #[serde(deserialize_with = "null_default")]
    pub deleted_count: i64,
    #[serde(deserialize_with = "null_default")]
    pub updated_count: i64,
    #[serde(deserialize_with = "null_default")]
    pub failed_ids: Vec<i64>,
    #[serde(deserialize_with = "null_default")]
    pub errors: Vec<String>,
}

type RowFields = (i64, Map<String, Value>);

/// Splits updates into what the bulk endpoint keeps and what it would lose.
fn split_for_bulk(updates: &[RowFields]) -> (Vec<RowFields>, Vec<RowFields>) {
    let (mut rest, mut single) = (Vec::new(), Vec::new());
    for (id, fields) in updates {
        let mut fields = fields.clone();
        if let Some(text) = fields.remove("description") {
            single.push((*id, Map::from_iter([("description".to_string(), text)])));
        }
        if !fields.is_empty() {
            rest.push((*id, fields));
        }
    }
    (rest, single)
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct NewTransaction {
    pub datetime: String,
    pub operator: String,
    pub amount: String,
    pub card_last4: String,
    pub transaction_type: String,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<String>,
    pub is_p2p: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver_card: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_text: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct InitReply {
    pub security_status: Value,
    pub years: Vec<YearCount>,
    pub operators: Vec<String>,
    pub applications: Vec<String>,
    pub telegram_sources: Vec<Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct YearCount {
    pub year: i32,
    pub count: i64,
}

/// Turns the server's refusal into the sentence the table shows.
pub fn delete_error_text(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("locked") || lower.contains("заблокирован") {
        "период заблокирован".into()
    } else if lower.contains("outside") || lower.contains("scope") {
        "вне доступного периода".into()
    } else if lower.contains("forbidden") || lower.contains("запрещ") {
        "дата запрещена для пользователя".into()
    } else if lower.contains("not found") || lower.contains("не найден") {
        "запись не найдена".into()
    } else {
        raw.to_string()
    }
}

impl Api {
    pub async fn list_transactions(&self, page: usize, page_size: usize) -> Result<TransactionPage, ApiError> {
        self.get("/api/transactions/", &[("page", page.to_string()), ("page_size", page_size.to_string())]).await
    }

    pub async fn transaction(&self, id: i64) -> Result<Transaction, ApiError> {
        self.get(&format!("/api/transactions/{id}"), &[]).await
    }

    pub async fn transactions_init(&self) -> Result<InitReply, ApiError> {
        self.get("/api/transactions/init", &[]).await
    }

    pub async fn update_transaction(&self, id: i64, fields: &Map<String, Value>) -> Result<UpdateReply, ApiError> {
        self.put(&format!("/api/transactions/{id}"), fields).await
    }

    pub async fn delete_transaction(&self, id: i64) -> Result<UpdateReply, ApiError> {
        self.delete(&format!("/api/transactions/{id}")).await
    }

    /// The server takes at most 100 ids per call; this splits and merges.
    pub async fn bulk_delete(&self, ids: &[i64]) -> Result<BulkReply, ApiError> {
        let mut total = BulkReply { ok: true, success: true, ..Default::default() };
        for chunk in ids.chunks(100) {
            let part: BulkReply = self.post("/api/transactions/bulk-delete", &json!({ "ids": chunk })).await?;
            total.deleted_count += part.deleted_count;
            total.failed_ids.extend(part.failed_ids);
            total.errors.extend(part.errors);
        }
        total.success = total.failed_ids.is_empty();
        Ok(total)
    }

    /// The server takes at most 500 updates per call.
    ///
    /// The bulk endpoint drops `description` without saying so and still counts
    /// the row as updated, so those go one by one through the single update.
    pub async fn bulk_update(&self, updates: &[(i64, Map<String, Value>)]) -> Result<BulkReply, ApiError> {
        let (rest, single) = split_for_bulk(updates);
        let mut total = BulkReply { ok: true, success: true, ..Default::default() };
        for chunk in rest.chunks(500) {
            let body: Vec<Value> = chunk.iter().map(|(id, fields)| json!({ "id": id, "fields": fields })).collect();
            let part: BulkReply = self.patch("/api/transactions/bulk-update", &json!({ "updates": body })).await?;
            total.updated_count += part.updated_count;
            total.failed_ids.extend(part.failed_ids);
            total.errors.extend(part.errors);
        }
        for (id, fields) in &single {
            match self.update_transaction(*id, fields).await {
                Ok(_) => total.updated_count += 1,
                Err(error) => {
                    if !total.failed_ids.contains(id) {
                        total.failed_ids.push(*id);
                    }
                    total.errors.push(error.to_string());
                }
            }
        }
        total.success = total.failed_ids.is_empty();
        Ok(total)
    }

    pub async fn create_transaction(&self, new: &NewTransaction) -> Result<Transaction, ApiError> {
        self.post("/api/transactions/", new).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_errors_read_like_the_old_client() {
        assert_eq!(delete_error_text("Period is locked"), "период заблокирован");
        assert_eq!(delete_error_text("outside current scope"), "вне доступного периода");
        assert_eq!(delete_error_text("Date forbidden"), "дата запрещена для пользователя");
        assert_eq!(delete_error_text("Transaction not found"), "запись не найдена");
        assert_eq!(delete_error_text("что-то ещё"), "что-то ещё");
    }

    #[test]
    fn a_description_never_goes_through_the_bulk_endpoint() {
        let fields = |value: Value| value.as_object().unwrap().clone();
        let updates = vec![
            (1, fields(json!({ "description": "текст", "amount": "5" }))),
            (2, fields(json!({ "description": "" }))),
            (3, fields(json!({ "currency": "USD" }))),
        ];
        let (rest, single) = split_for_bulk(&updates);
        assert_eq!(rest, vec![(1, fields(json!({ "amount": "5" }))), (3, fields(json!({ "currency": "USD" })))]);
        assert_eq!(single, vec![(1, fields(json!({ "description": "текст" }))), (2, fields(json!({ "description": "" })))]);
    }

    #[test]
    fn optional_fields_stay_out_of_the_payload() {
        let v = serde_json::to_value(NewTransaction { operator: "x".into(), ..Default::default() }).unwrap();
        assert!(v.get("app").is_none());
        assert_eq!(v["operator"], "x");
    }
}
