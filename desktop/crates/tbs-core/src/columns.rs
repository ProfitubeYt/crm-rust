//! The transaction table's columns: order, titles, default widths.

use crate::format;
use crate::model::Transaction;
use chrono::NaiveDateTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Column {
    RowNumber,
    OperationNumber,
    DateTime,
    TransactionDate,
    Time,
    Day,
    OperatorRaw,
    ApplicationMapped,
    Description,
    ReceiverName,
    ReceiverCard,
    Amount,
    BalanceAfter,
    CardLast4,
    IsP2p,
    TransactionType,
    Currency,
    SourceType,
    AnnotationDetails,
    AnnotationPersonCode,
    AnnotationPeriod,
    AnnotationArticleCode,
}

pub const ALL: [Column; 22] = [
    Column::RowNumber,
    Column::OperationNumber,
    Column::DateTime,
    Column::TransactionDate,
    Column::Time,
    Column::Day,
    Column::OperatorRaw,
    Column::ApplicationMapped,
    Column::Description,
    Column::ReceiverName,
    Column::ReceiverCard,
    Column::Amount,
    Column::BalanceAfter,
    Column::CardLast4,
    Column::IsP2p,
    Column::TransactionType,
    Column::Currency,
    Column::SourceType,
    Column::AnnotationDetails,
    Column::AnnotationPersonCode,
    Column::AnnotationPeriod,
    Column::AnnotationArticleCode,
];

pub const COUNT: usize = ALL.len();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Center,
}

impl Column {
    pub fn index(self) -> usize {
        ALL.iter().position(|c| *c == self).unwrap()
    }

    pub fn title(self) -> &'static str {
        match self {
            Column::RowNumber => "№",
            Column::OperationNumber => "№ опер.",
            Column::DateTime => "Дата и время",
            Column::TransactionDate => "Дата",
            Column::Time => "Время",
            Column::Day => "День",
            Column::OperatorRaw => "Оператор/Продавец",
            Column::ApplicationMapped => "Приложение",
            Column::Description => "Описание",
            Column::ReceiverName => "Получатель",
            Column::ReceiverCard => "Карта получателя",
            Column::Amount => "Сумма",
            Column::BalanceAfter => "Остаток",
            Column::CardLast4 => "ПК",
            Column::IsP2p => "П2П",
            Column::TransactionType => "Тип",
            Column::Currency => "Валюта",
            Column::SourceType => "Источник",
            Column::AnnotationDetails => "Подробности",
            Column::AnnotationPersonCode => "Лицо",
            Column::AnnotationPeriod => "Период",
            Column::AnnotationArticleCode => "Статья",
        }
    }

    pub fn default_width(self) -> f32 {
        match self {
            Column::RowNumber => 60.0,
            Column::OperationNumber => 78.0,
            Column::DateTime => 125.0,
            Column::TransactionDate => 92.0,
            Column::Time => 62.0,
            Column::Day => 50.0,
            Column::OperatorRaw => 240.0,
            Column::ApplicationMapped => 130.0,
            Column::Description => 200.0,
            Column::ReceiverName => 160.0,
            Column::ReceiverCard => 140.0,
            Column::Amount => 110.0,
            Column::BalanceAfter => 120.0,
            Column::CardLast4 => 60.0,
            Column::IsP2p => 50.0,
            Column::TransactionType => 96.0,
            Column::Currency => 64.0,
            Column::SourceType => 130.0,
            Column::AnnotationDetails => 200.0,
            Column::AnnotationPersonCode => 80.0,
            Column::AnnotationPeriod => 100.0,
            Column::AnnotationArticleCode => 150.0,
        }
    }

    pub fn align(self) -> Align {
        match self {
            Column::RowNumber | Column::OperationNumber | Column::Amount | Column::BalanceAfter => Align::Right,
            Column::IsP2p | Column::Day | Column::Currency | Column::CardLast4 => Align::Center,
            _ => Align::Left,
        }
    }

    /// Row and operation numbers depend on the current view, so the grid
    /// computes them; everything else is a property of the row.
    pub fn is_positional(self) -> bool {
        matches!(self, Column::RowNumber | Column::OperationNumber)
    }

    pub fn sortable(self) -> bool {
        self != Column::RowNumber
    }

    /// The server field behind an editable column.
    /// The column's name on the wire: export lists and saved layouts of the old client.
    pub fn id(self) -> &'static str {
        match self {
            Column::RowNumber => "row_number",
            Column::OperationNumber => "operation_number",
            Column::DateTime => "date_time",
            Column::TransactionDate => "transaction_date",
            Column::Time => "time",
            Column::Day => "day",
            Column::OperatorRaw => "operator_raw",
            Column::ApplicationMapped => "application_mapped",
            Column::Description => "description",
            Column::ReceiverName => "receiver_name",
            Column::ReceiverCard => "receiver_card",
            Column::Amount => "amount",
            Column::BalanceAfter => "balance_after",
            Column::CardLast4 => "card_last_4",
            Column::IsP2p => "is_p2p",
            Column::TransactionType => "transaction_type",
            Column::Currency => "currency",
            Column::SourceType => "source_type",
            Column::AnnotationDetails => "annotation_details",
            Column::AnnotationPersonCode => "annotation_person_code",
            Column::AnnotationPeriod => "annotation_period",
            Column::AnnotationArticleCode => "annotation_article_code",
        }
    }

    pub fn edit_field(self) -> Option<&'static str> {
        Some(match self {
            Column::OperatorRaw => "operator_raw",
            Column::ApplicationMapped => "application_mapped",
            Column::Description => "description",
            Column::ReceiverName => "receiver_name",
            Column::ReceiverCard => "receiver_card",
            Column::Amount => "amount",
            Column::BalanceAfter => "balance_after",
            Column::CardLast4 => "card_last_4",
            Column::Currency => "currency",
            Column::AnnotationDetails => "annotation_details",
            Column::AnnotationPersonCode => "annotation_person_code",
            Column::AnnotationPeriod => "annotation_period",
            Column::AnnotationArticleCode => "annotation_article_code",
            _ => return None,
        })
    }

    pub fn raw_value(self, tx: &Transaction) -> String {
        let v = match self {
            Column::OperatorRaw => &tx.operator_raw,
            Column::ApplicationMapped => &tx.application_mapped,
            Column::Description => &tx.description,
            Column::ReceiverName => &tx.receiver_name,
            Column::ReceiverCard => &tx.receiver_card,
            Column::Amount => &tx.amount,
            Column::BalanceAfter => &tx.balance_after,
            Column::CardLast4 => &tx.card_last_4,
            Column::Currency => &tx.currency,
            Column::AnnotationDetails => &tx.annotation_details,
            Column::AnnotationPersonCode => &tx.annotation_person_code,
            Column::AnnotationPeriod => &tx.annotation_period,
            Column::AnnotationArticleCode => &tx.annotation_article_code,
            _ => return String::new(),
        };
        v.clone().unwrap_or_default()
    }

    pub fn display(self, tx: &Transaction, when: Option<NaiveDateTime>) -> String {
        let text = |v: &Option<String>| v.as_deref().filter(|s| !s.is_empty()).unwrap_or(format::EMPTY).to_string();
        let blank = |v: &Option<String>| v.clone().unwrap_or_default();
        match self {
            Column::RowNumber | Column::OperationNumber => String::new(),
            Column::DateTime => format::datetime(when),
            Column::TransactionDate => format::date(when),
            Column::Time => format::time(when),
            Column::Day => format::weekday(when).to_string(),
            Column::OperatorRaw => text(&tx.operator_raw),
            Column::ApplicationMapped => text(&tx.application_mapped),
            Column::Description => blank(&tx.description),
            Column::ReceiverName => text(&tx.receiver_name),
            Column::ReceiverCard => text(&tx.receiver_card),
            Column::Amount => format::amount(tx.amount.as_deref()),
            Column::BalanceAfter => format::amount(tx.balance_after.as_deref()),
            Column::CardLast4 => text(&tx.card_last_4),
            Column::IsP2p => {
                if tx.is_p2p {
                    "1".into()
                } else {
                    String::new()
                }
            }
            Column::TransactionType => format::type_label(tx.transaction_type.as_deref())
                .map(str::to_string)
                .or_else(|| tx.transaction_type_display.clone())
                .or_else(|| tx.transaction_type.clone())
                .unwrap_or_else(|| format::EMPTY.into()),
            Column::Currency => text(&tx.currency),
            Column::SourceType => source_label(tx),
            Column::AnnotationDetails => blank(&tx.annotation_details),
            Column::AnnotationPersonCode => blank(&tx.annotation_person_code),
            Column::AnnotationPeriod => blank(&tx.annotation_period),
            Column::AnnotationArticleCode => blank(&tx.annotation_article_code),
        }
    }
}

pub fn source_label(tx: &Transaction) -> String {
    if let Some(t) = tx.source_chat_title.as_deref().filter(|s| !s.is_empty()) {
        return t.to_string();
    }
    if let Some(t) = tx.source_display.as_deref().filter(|s| !s.is_empty()) {
        return t.to_string();
    }
    match tx.source_channel.as_deref() {
        Some("TELEGRAM") => "Телеграм",
        Some("SMS") => "СМС",
        Some("MANUAL") => "Ручной",
        _ => format::EMPTY,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_order_is_stable() {
        assert_eq!(ALL.len(), COUNT);
        for (i, c) in ALL.iter().enumerate() {
            assert_eq!(c.index(), i);
        }
    }

    #[test]
    fn source_falls_back_in_order() {
        let mut tx = Transaction { source_channel: Some("SMS".into()), ..Default::default() };
        assert_eq!(source_label(&tx), "СМС");
        tx.source_display = Some("Канал".into());
        assert_eq!(source_label(&tx), "Канал");
        tx.source_chat_title = Some("HUMO".into());
        assert_eq!(source_label(&tx), "HUMO");
    }
}
