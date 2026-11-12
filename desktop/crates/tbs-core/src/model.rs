//! Wire types shared by the API, the cache and the query engine.

use std::collections::BTreeMap;

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
    /// Folder and source ids. The server sends numbers; nothing here reads
    /// them, so they are kept as they come and no shape of them stops a sign-in.
    #[serde(deserialize_with = "null_default")]
    pub allowed_folders: Vec<serde_json::Value>,
    #[serde(deserialize_with = "null_default")]
    pub forbidden_periods: Vec<serde_json::Value>,
    #[serde(deserialize_with = "null_default")]
    pub allowed_sources: Vec<serde_json::Value>,
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
    /// The workspaces the start window lists for this account, by the server's
    /// word. A server from before workspaces does not send it: Checks is all
    /// that server has, and every account on it is a Checks account.
    #[serde(deserialize_with = "as_sent")]
    pub workspaces: Option<Vec<String>>,
    /// Every role held, the workspaces the start window does not list included.
    #[serde(deserialize_with = "as_sent")]
    pub workspace_roles: Option<BTreeMap<String, String>>,
    /// May administer the accounts of the whole CRM. A server from before that
    /// right was taken apart from the Checks role does not send it.
    #[serde(deserialize_with = "as_sent")]
    pub global_admin: Option<bool>,
}

/// A grant described in a shape this build does not read is taken as not
/// described at all: the account signs in as on a server from before
/// workspaces, and the server goes on judging every request itself.
fn as_sent<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

/// The workspaces as the server names them.
pub const WORKSPACE_CHECKS: &str = "checks";
pub const WORKSPACE_TIME: &str = "time";

impl UserInfo {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    /// The server said which workspaces the account has, so one that is not
    /// named was not granted. An older server's silence says nothing of the kind.
    pub fn workspaces_known(&self) -> bool {
        self.workspaces.is_some()
    }

    /// Whether the start window may offer a workspace. Nothing is guessed for
    /// a server from before workspaces: it has Checks and nothing else.
    pub fn has_workspace(&self, id: &str) -> bool {
        match &self.workspaces {
            Some(granted) => granted.iter().any(|name| name == id),
            None => id == WORKSPACE_CHECKS,
        }
    }

    /// CRM administration, which only the server's own word grants. An older
    /// server has no administration panel to open, so there nobody has it.
    pub fn is_global_admin(&self) -> bool {
        self.global_admin == Some(true)
    }

    /// May manage accounts from the Checks settings. Where the server keeps
    /// CRM administration apart, that right decides; before the split the
    /// Checks administrator was the one who managed accounts.
    pub fn manages_accounts(&self) -> bool {
        self.global_admin.unwrap_or_else(|| self.is_admin())
    }

    /// Takes the server's fresh word on what the account may open and leaves
    /// everything else as the workspace on screen was started with.
    pub fn adopt_access(&mut self, fresh: &UserInfo) {
        self.workspaces.clone_from(&fresh.workspaces);
        self.workspace_roles.clone_from(&fresh.workspace_roles);
        self.global_admin = fresh.global_admin;
    }

    /// The account no longer has this workspace, the server just said so.
    pub fn drop_workspace(&mut self, id: &str) {
        let mut granted = self.workspaces.take().unwrap_or_else(|| vec![WORKSPACE_CHECKS.to_owned()]);
        granted.retain(|name| name != id);
        self.workspaces = Some(granted);
        if let Some(roles) = &mut self.workspace_roles {
            roles.remove(id);
        }
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

    /// `/api/auth/me` as release 2.0.3 answers it: the server on production
    /// while this client is being written. It knows neither workspaces nor CRM
    /// administration.
    const ME_BEFORE_WORKSPACES: &str = r#"{"id":2,"username":"anna","display_name":"Анна К.","role":"admin","allowed_tabs":["dashboard","reference","automation","userbot","logs","admin"],
        "allowed_folders":[],"forbidden_periods":[],"allowed_sources":[],"can_toggle_sources":true,"permissions_version":4,"totp_enabled":true,
        "totp_confirmed_at":"2026-10-01T08:00:00","force_2fa":false,"session_ttl_days":7,"exp":1794000000}"#;

    fn me(extra: &str) -> UserInfo {
        let raw = format!(r#"{{"id":5,"username":"olga","display_name":"Ольга","role":"operator","allowed_tabs":["dashboard"],"allowed_folders":[],"forbidden_periods":[],
            "allowed_sources":[],"can_toggle_sources":false,"permissions_version":1{extra}}}"#);
        serde_json::from_str(&raw).unwrap()
    }

    #[test]
    fn an_older_server_is_a_checks_server_and_nothing_is_guessed_beyond_it() {
        let admin: UserInfo = serde_json::from_str(ME_BEFORE_WORKSPACES).unwrap();
        assert_eq!((admin.workspaces.clone(), admin.workspace_roles.clone(), admin.global_admin), (None, None, None));
        assert!(!admin.workspaces_known());
        // What it always was: every account opens Checks, and its administrator manages the accounts.
        assert!(admin.has_workspace(WORKSPACE_CHECKS) && admin.manages_accounts());
        // What that server does not have is not offered, to its administrator either.
        assert!(!admin.has_workspace(WORKSPACE_TIME) && !admin.is_global_admin());

        let operator = me("");
        assert!(operator.has_workspace(WORKSPACE_CHECKS));
        assert!(!operator.has_workspace(WORKSPACE_TIME) && !operator.is_global_admin() && !operator.manages_accounts());
    }

    #[test]
    fn the_servers_word_decides_each_workspace_and_administration_on_its_own() {
        let access = |user: &UserInfo| (user.has_workspace(WORKSPACE_CHECKS), user.has_workspace(WORKSPACE_TIME), user.is_global_admin());

        let checks = me(r#","workspaces":["checks"],"workspace_roles":{"checks":"operator"},"global_admin":false"#);
        assert_eq!(access(&checks), (true, false, false));
        let time = me(r#","workspaces":["time"],"workspace_roles":{"time":"manager"},"global_admin":false"#);
        assert_eq!(access(&time), (false, true, false));
        let both = me(r#","workspaces":["checks","time"],"workspace_roles":{"checks":"operator","time":"viewer"},"global_admin":false"#);
        assert_eq!(access(&both), (true, true, false));
        // An employee marks attendance through the bot: a role, and no workspace to open.
        let neither = me(r#","workspaces":[],"workspace_roles":{"time":"employee"},"global_admin":false"#);
        assert_eq!(access(&neither), (false, false, false));
        assert!(neither.workspaces_known());
        assert_eq!(neither.workspace_roles.as_ref().and_then(|roles| roles.get("time")).map(String::as_str), Some("employee"));
        // Administration opens no workspace, and no workspace role gives it.
        let administrator = me(r#","workspaces":[],"workspace_roles":{},"global_admin":true"#);
        assert_eq!(access(&administrator), (false, false, true));
        let mut checks_admin = me(r#","workspaces":["checks","time"],"workspace_roles":{"checks":"admin","time":"manager"},"global_admin":false"#);
        checks_admin.role = "admin".into();
        assert!(checks_admin.is_admin() && !checks_admin.is_global_admin() && !checks_admin.manages_accounts());
        // A workspace this client has never heard of is not an error.
        let later = me(r#","workspaces":["checks","payroll"],"workspace_roles":{"payroll":"chief"},"global_admin":null"#);
        assert!(later.has_workspace(WORKSPACE_CHECKS) && later.has_workspace("payroll") && !later.is_global_admin());
    }

    #[test]
    fn an_operator_with_folders_and_sources_still_reads() {
        // The server sends the ids as numbers; an account that fails to read cannot sign in.
        let raw = r#"{"id":6,"username":"petr","role":"operator","allowed_folders":[3,8],"allowed_sources":[12],"forbidden_periods":[{"from":"2026-01-01","to":"2026-01-31"}]}"#;
        let user: UserInfo = serde_json::from_str(raw).unwrap();
        assert_eq!((user.allowed_folders.len(), user.allowed_sources.len(), user.forbidden_periods.len()), (2, 1, 1));
    }

    #[test]
    fn fresh_access_replaces_the_old_and_nothing_else() {
        let mut at_work = me(r#","workspaces":["checks","time"],"workspace_roles":{"checks":"operator","time":"viewer"},"global_admin":true"#);
        let mut fresh = me(r#","workspaces":["time"],"workspace_roles":{"time":"viewer"},"global_admin":false"#);
        fresh.allowed_tabs = vec!["logs".into()];
        at_work.adopt_access(&fresh);
        assert!(!at_work.has_workspace(WORKSPACE_CHECKS) && at_work.has_workspace(WORKSPACE_TIME) && !at_work.is_global_admin());
        assert_eq!(at_work.allowed_tabs, ["dashboard"], "the workspace on screen keeps what it was started with");

        at_work.drop_workspace(WORKSPACE_TIME);
        assert!(!at_work.has_workspace(WORKSPACE_TIME) && at_work.workspaces_known());
        assert_eq!(at_work.workspace_roles, Some(BTreeMap::new()));

        // A refusal that names a workspace comes from a server that knows them:
        // an account read before the upgrade had Checks, and now has what is left.
        let mut before: UserInfo = serde_json::from_str(ME_BEFORE_WORKSPACES).unwrap();
        before.drop_workspace(WORKSPACE_CHECKS);
        assert!(before.workspaces_known() && !before.has_workspace(WORKSPACE_CHECKS));
    }

    #[test]
    fn a_grant_in_a_shape_this_build_does_not_read_does_not_stop_the_sign_in() {
        for odd in [
            r#","workspaces":"checks","workspace_roles":["time"],"global_admin":"yes""#,
            r#","workspaces":{"checks":true},"workspace_roles":7,"global_admin":1"#,
            r#","workspaces":["checks",5],"workspace_roles":{"time":{"role":"manager"}}"#,
        ] {
            let user = me(odd);
            assert_eq!((user.workspaces.clone(), user.workspace_roles.clone(), user.global_admin), (None, None, None), "{odd}");
            // As on a server from before workspaces: Checks, and nothing granted by guessing.
            assert!(user.has_workspace(WORKSPACE_CHECKS) && !user.has_workspace(WORKSPACE_TIME) && !user.manages_accounts(), "{odd}");
        }
        // One odd field does not take the readable ones with it.
        let user = me(r#","workspaces":["time"],"workspace_roles":"?","global_admin":true"#);
        assert!(user.has_workspace(WORKSPACE_TIME) && !user.has_workspace(WORKSPACE_CHECKS) && user.manages_accounts());
        assert_eq!(user.workspace_roles, None);
    }

    #[test]
    fn the_account_round_trips_with_and_without_the_new_fields() {
        for user in [serde_json::from_str::<UserInfo>(ME_BEFORE_WORKSPACES).unwrap(), me(r#","workspaces":["time"],"workspace_roles":{"time":"manager"},"global_admin":true"#)] {
            assert_eq!(serde_json::from_value::<UserInfo>(serde_json::to_value(&user).unwrap()).unwrap(), user);
        }
    }
}
