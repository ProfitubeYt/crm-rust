//! The Time workspace. Its page talks to the server's gateway through
//! [`Api::proxy`]; the one call the shell makes itself is the workbook, which
//! a page has neither a session nor a disk for.

use super::{Api, ApiError};

/// `YYYY-MM`, the way the gateway names a month.
pub fn is_month(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7 && bytes[4] == b'-' && bytes.iter().enumerate().all(|(at, byte)| at == 4 || byte.is_ascii_digit())
}

impl Api {
    /// The month's workbook with the name the server gives it.
    pub async fn time_export(&self, month: &str) -> Result<(Option<String>, Vec<u8>), ApiError> {
        if !is_month(month) {
            return Err(ApiError::Http { status: 400, code: None, message: "Некорректный месяц".to_owned() });
        }
        self.download(&format!("/api/time/export/{month}"), &[]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_month_names_a_workbook() {
        assert!(is_month("2026-11"));
        for wrong in ["", "2026-1", "2026-11-01", "2026/11", "../../x", "2026-1a", "２０２６-11"] {
            assert!(!is_month(wrong), "{wrong}");
        }
    }
}
