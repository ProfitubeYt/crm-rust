//! Wire types shared by the API, the cache and the query engine.

use serde::{Deserialize, Deserializer, Serialize};

/// The server sends decimals as strings today and as numbers in older paths.
pub fn de_decimal<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Option::<serde_json::Value>::deserialize(d)? {
        Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s),
        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
        _ => None,
    })
}

/// `#[serde(default)]` covers a missing field but not an explicit `null`,
/// which the server's optional response models send for every unset value.
pub fn null_default<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transaction {
    pub id: i64,
    pub transaction_date: Option<String>,
    #[serde(deserialize_with = "de_decimal")]
    pub amount: Option<String>,
    pub currency: Option<String>,
    pub card_last_4: Option<String>,
    pub operator_raw: Option<String>,
    pub application_mapped: Option<String>,
    pub description: Option<String>,
    pub transaction_type: Option<String>,
    pub transaction_type_display: Option<String>,
    /// The type the receipt is stored with, where `transaction_type` is what
    /// the server reads off the receipt again for every answer. The workbook
    /// words «Операция» by the stored one. A server from before the field
    /// does not send it, and `transaction_type` stands in.
    pub stored_transaction_type: Option<String>,
    #[serde(deserialize_with = "de_decimal")]
    pub balance_after: Option<String>,
    pub receiver_name: Option<String>,
    pub receiver_card: Option<String>,
    pub source_type: Option<String>,
    pub source_channel: Option<String>,
    pub source_display: Option<String>,
    pub source_chat_id: Option<i64>,
    pub source_chat_title: Option<String>,
    pub source_origin_type: Option<String>,
    /// The bank a row was given by hand in the workbook: `CardXabar` or `HUMO Card`.
    pub excel_bank: Option<String>,
    pub parsing_method: Option<String>,
    pub parsing_confidence: Option<f64>,
    #[serde(deserialize_with = "null_default")]
    pub is_p2p: bool,
    pub annotation_details: Option<String>,
    pub annotation_person_code: Option<String>,
    pub annotation_period: Option<String>,
    pub annotation_article_code: Option<String>,
    pub annotation_details_src: Option<String>,
    pub annotation_person_src: Option<String>,
    pub annotation_period_src: Option<String>,
    pub annotation_article_src: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub raw_message: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct TransactionPage {
    pub ok: bool,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
    pub items: Vec<Transaction>,
    /// `null` when the server has no rows to page through, as for a locked period.
    #[serde(deserialize_with = "null_default")]
    pub has_more: bool,
    pub locked_notice: Option<serde_json::Value>,
    pub latest_transaction_at: Option<String>,
    pub oldest_transaction_at: Option<String>,
    pub server_generated_at: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserInfo {
    pub id: i64,
    pub user_id: Option<i64>,
    pub username: String,
    pub display_name: Option<String>,
    pub role: String,
    #[serde(deserialize_with = "null_default")]
    pub is_active: bool,
    #[serde(deserialize_with = "null_default")]
    pub allowed_tabs: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub allowed_folders: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub forbidden_periods: Vec<serde_json::Value>,
    #[serde(deserialize_with = "null_default")]
    pub allowed_sources: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub can_toggle_sources: bool,
    #[serde(deserialize_with = "null_default")]
    pub permissions_version: i64,
    #[serde(deserialize_with = "null_default")]
    pub totp_enabled: bool,
    pub totp_confirmed_at: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub force_2fa: bool,
    pub session_ttl_days: Option<i64>,
    pub access_until: Option<String>,
    pub access_note: Option<String>,
    pub last_login_at: Option<String>,
}

impl UserInfo {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    /// Admins see every section; `transactions` is the old name of `dashboard`.
    pub fn has_tab(&self, tab: &str) -> bool {
        if self.is_admin() {
            return true;
        }
        let wanted = if tab == "transactions" { "dashboard" } else { tab };
        self.allowed_tabs.iter().any(|t| {
            let t = if t == "transactions" { "dashboard" } else { t.as_str() };
            t == wanted
        })
    }

    pub fn title(&self) -> &str {
        self.display_name.as_deref().filter(|n| !n.is_empty()).unwrap_or(&self.username)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_arrive_in_either_shape() {
        let a: Transaction = serde_json::from_str(r#"{"id":1,"amount":"5000.00","balance_after":12.5}"#).unwrap();
        assert_eq!(a.amount.as_deref(), Some("5000.00"));
        assert_eq!(a.balance_after.as_deref(), Some("12.5"));
        let b: Transaction = serde_json::from_str(r#"{"id":2,"amount":null}"#).unwrap();
        assert_eq!(b.amount, None);
    }

    #[test]
    fn the_given_bank_is_optional() {
        // A server that does not send the field yet, one that sends null, one that sends it.
        let absent: Transaction = serde_json::from_str(r#"{"id":1,"source_display":"Телеграм"}"#).unwrap();
        assert_eq!(absent.excel_bank, None);
        let null: Transaction = serde_json::from_str(r#"{"id":2,"excel_bank":null}"#).unwrap();
        assert_eq!(null.excel_bank, None);
        let given: Transaction = serde_json::from_str(r#"{"id":3,"excel_bank":"HUMO Card"}"#).unwrap();
        assert_eq!(given.excel_bank.as_deref(), Some("HUMO Card"));
        // The cache writes the field out even when empty: that is how a cache from before it is told apart.
        assert!(serde_json::to_value(&absent).unwrap().as_object().unwrap().contains_key("excel_bank"));
        assert_eq!(serde_json::from_value::<Transaction>(serde_json::to_value(&given).unwrap()).unwrap(), given);
    }

    #[test]
    fn the_stored_type_is_optional() {
        // A server from before the field, one that sends null, one that sends it.
        let absent: Transaction = serde_json::from_str(r#"{"id":1,"transaction_type":"DEBIT","transaction_type_display":"Списание"}"#).unwrap();
        assert_eq!((absent.stored_transaction_type.as_deref(), absent.transaction_type.as_deref()), (None, Some("DEBIT")));
        let null: Transaction = serde_json::from_str(r#"{"id":2,"transaction_type":"DEBIT","stored_transaction_type":null,"excel_bank":null}"#).unwrap();
        assert_eq!((null.stored_transaction_type.as_deref(), null.excel_bank.as_deref(), null.transaction_type.as_deref()), (None, None, Some("DEBIT")));
        let sent: Transaction =
            serde_json::from_str(r#"{"id":3,"transaction_type":"DEBIT","transaction_type_display":"Изменение счета","stored_transaction_type":"CREDIT","excel_bank":"HUMO Card"}"#).unwrap();
        assert_eq!(sent.stored_transaction_type.as_deref(), Some("CREDIT"));
        // It comes beside the server's own reading and wording, not instead of them.
        assert_eq!((sent.transaction_type.as_deref(), sent.transaction_type_display.as_deref(), sent.excel_bank.as_deref()), (Some("DEBIT"), Some("Изменение счета"), Some("HUMO Card")));
        // The cache writes the field out even when empty: that is how a cache from before it is told apart.
        assert!(serde_json::to_value(&absent).unwrap().as_object().unwrap().contains_key("stored_transaction_type"));
        for tx in [absent, null, sent] {
            assert_eq!(serde_json::from_value::<Transaction>(serde_json::to_value(&tx).unwrap()).unwrap(), tx);
        }
    }

    #[test]
    fn a_page_reads_with_the_new_fields_and_without_them() {
        // An older server's page, then the same rows from one that knows both fields.
        let old = r#"{"ok":true,"total":2,"page":1,"page_size":1000,"has_more":false,"items":[
            {"id":1,"transaction_type":"DEBIT","amount":"5000.00"},
            {"id":2,"transaction_type":"CREDIT","amount":"7000.00"}]}"#;
        let new = r#"{"ok":true,"total":2,"page":1,"page_size":1000,"has_more":false,"items":[
            {"id":1,"transaction_type":"DEBIT","amount":"5000.00","stored_transaction_type":"DEBIT","excel_bank":null},
            {"id":2,"transaction_type":"CREDIT","amount":"7000.00","stored_transaction_type":null,"excel_bank":"CardXabar"}]}"#;
        let (old, new): (TransactionPage, TransactionPage) = (serde_json::from_str(old).unwrap(), serde_json::from_str(new).unwrap());
        let fields = |page: &TransactionPage| page.items.iter().map(|tx| (tx.id, tx.stored_transaction_type.clone(), tx.excel_bank.clone())).collect::<Vec<_>>();
        assert_eq!(fields(&old), [(1, None, None), (2, None, None)]);
        assert_eq!(fields(&new), [(1, Some("DEBIT".to_string()), None), (2, None, Some("CardXabar".to_string()))]);
        // Everything else in the rows is the same either way.
        for (before, after) in old.items.iter().zip(&new.items) {
            assert_eq!(Transaction { stored_transaction_type: None, excel_bank: None, ..after.clone() }, *before);
        }
    }

    #[test]
    fn a_locked_period_answers_without_pages() {
        let page: TransactionPage = serde_json::from_str(r#"{"ok":true,"total":0,"page":1,"page_size":1000,"items":[],"has_more":null,"locked_notice":"Период закрыт"}"#).unwrap();
        assert!(!page.has_more);
        assert!(page.items.is_empty());
    }

    #[test]
    fn tab_access() {
        let mut u = UserInfo { role: "user".into(), allowed_tabs: vec!["transactions".into(), "logs".into()], ..Default::default() };
        assert!(u.has_tab("dashboard"));
        assert!(u.has_tab("logs"));
        assert!(!u.has_tab("reference"));
        u.role = "admin".into();
        assert!(u.has_tab("reference"));
    }
}
