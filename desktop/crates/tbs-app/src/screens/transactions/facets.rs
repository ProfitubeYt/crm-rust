//! The filters of the table one at a time: what each is called, what it says
//! on a chip, and how it is taken off.
//!
//! `Filters` keeps a range as two fields and a list as one. A person counts a
//! range as one filter, and this is that count, in the order the sheet has them.

use chrono::NaiveDate;
use tbs_core::format;
use tbs_core::query::{AnnotationState, Filters};

use super::edit::TYPES;

pub const DAYS: [(u8, &str); 7] = [(1, "Пн"), (2, "Вт"), (3, "Ср"), (4, "Чт"), (5, "Пт"), (6, "Сб"), (0, "Вс")];
pub const METHODS: [(&str, &str); 4] = [("regex", "Regex"), ("gpt", "GPT"), ("manual", "Ручной"), ("operator_mapper", "Mapper")];
pub const CHANNELS: [(&str, &str); 3] = [("TELEGRAM", "Telegram"), ("SMS", "СМС"), ("MANUAL", "Ручной")];

/// A chip names this many of a list and counts the rest.
const NAMED: usize = 2;
/// A name or a typed text is cut to this many characters, so a chip stays a chip.
const NAME_LENGTH: usize = 28;

/// The dictionaries a chip reads a code by. Where one is missing, the code itself is shown.
#[derive(Clone, Copy, Debug, Default)]
pub struct Names<'a> {
    pub persons: &'a [(String, String)],
    pub articles: &'a [(String, String)],
    pub chats: &'a [(i64, String)],
}

/// One filter of the table, as a person counts them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facet {
    Date,
    Days,
    Amount,
    Currency,
    Types,
    Channel,
    Chats,
    Operators,
    Apps,
    Method,
    Confidence,
    Details,
    Period,
    State,
    Persons,
    Articles,
    Card,
}

/// Cuts a line to `length` characters, the last of them «…» when something was left out.
pub fn shorten(text: &str, length: usize) -> String {
    if text.chars().count() <= length {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(length.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// Whether two sets of filters narrow the table the same way. The search is
/// not among them: it has a box of its own.
pub fn agree(a: &Filters, b: &Filters) -> bool {
    Facet::ALL.into_iter().all(|facet| facet.same(a, b))
}

/// The same table with every filter off. The search stays.
pub fn cleared(filters: &Filters) -> Filters {
    Filters { search: filters.search.clone(), ..Default::default() }
}

fn named<'a>(known: &[(&'a str, &'a str)], code: &'a str) -> &'a str {
    known.iter().find(|(id, _)| *id == code).map_or(code, |(_, name)| *name)
}

fn looked_up<'a>(known: &'a [(String, String)], code: &'a str) -> &'a str {
    known.iter().find(|(id, _)| id == code).map_or(code, |(_, name)| name.as_str())
}

/// The first names of a list and how many more there are: «Korzinka, Ромашка +2».
fn listed<'a>(names: impl Iterator<Item = &'a str>) -> Option<String> {
    let names: Vec<&str> = names.collect();
    if names.is_empty() {
        return None;
    }
    let mut line = names.iter().take(NAMED).map(|name| shorten(name, NAME_LENGTH)).collect::<Vec<_>>().join(", ");
    if names.len() > NAMED {
        line.push_str(&format!(" +{}", names.len() - NAMED));
    }
    Some(line)
}

/// A stretch of time: «01.11.2026 – 11.11.2026», or the one end that is set.
fn stretch(from: Option<String>, to: Option<String>) -> Option<String> {
    match (from, to) {
        (Some(from), Some(to)) if from == to => Some(from),
        (Some(from), Some(to)) => Some(format!("{from} – {to}")),
        (Some(from), None) => Some(format!("с {from}")),
        (None, Some(to)) => Some(format!("по {to}")),
        (None, None) => None,
    }
}

/// The bounds of a number: «от 1 000 до 5 000», or the one bound that is set.
fn bounds(min: Option<String>, max: Option<String>) -> Option<String> {
    match (min, max) {
        (Some(min), Some(max)) if min == max => Some(min),
        (Some(min), Some(max)) => Some(format!("от {min} до {max}")),
        (Some(min), None) => Some(format!("от {min}")),
        (None, Some(max)) => Some(format!("до {max}")),
        (None, None) => None,
    }
}

fn day(date: NaiveDate) -> String {
    date.format("%d.%m.%Y").to_string()
}

fn month(key: i32) -> String {
    format!("{:02}.{}", key % 100, key / 100)
}

/// A number with its thousands apart and as many decimals as it was typed with.
fn number(value: f64) -> String {
    let decimals = value.to_string().split_once('.').map_or(0, |(_, part)| part.len());
    format::number(value, decimals)
}

/// A stored fraction as the whole percent the form shows for it.
fn percent(value: f64) -> String {
    format!("{}%", (value * 100.0).round())
}

/// The days in the order of the week, whatever order they were ticked in.
fn weekdays(chosen: &[u8]) -> String {
    let mut names: Vec<String> = DAYS.iter().filter(|(id, _)| chosen.contains(id)).map(|(id, _)| format::WEEKDAYS[*id as usize].to_string()).collect();
    // A number no week has was saved by something else. It is shown, so that it can be seen and dropped.
    names.extend(chosen.iter().filter(|id| **id > 6).map(u8::to_string));
    names.join(", ")
}

/// The same choices, in whatever order they were made.
fn alike<T: Ord + Clone>(a: &[T], b: &[T]) -> bool {
    let sorted = |items: &[T]| {
        let mut items = items.to_vec();
        items.sort();
        items.dedup();
        items
    };
    sorted(a) == sorted(b)
}

impl Facet {
    pub const ALL: [Facet; 17] = [
        Facet::Date,
        Facet::Days,
        Facet::Amount,
        Facet::Currency,
        Facet::Types,
        Facet::Channel,
        Facet::Chats,
        Facet::Operators,
        Facet::Apps,
        Facet::Method,
        Facet::Confidence,
        Facet::Details,
        Facet::Period,
        Facet::State,
        Facet::Persons,
        Facet::Articles,
        Facet::Card,
    ];

    /// The name a chip begins with.
    pub fn label(self) -> &'static str {
        match self {
            Facet::Date => "Дата",
            Facet::Days => "Дни недели",
            Facet::Amount => "Сумма",
            Facet::Currency => "Валюта",
            Facet::Types => "Тип",
            Facet::Channel => "Источник",
            Facet::Chats => "Чат",
            Facet::Operators => "Оператор",
            Facet::Apps => "Приложение",
            Facet::Method => "Парсер",
            Facet::Confidence => "Уверенность",
            Facet::Details => "Детали",
            Facet::Period => "Период учёта",
            Facet::State => "Разметка",
            Facet::Persons => "Лицо",
            Facet::Articles => "Статья",
            Facet::Card => "Карта",
        }
    }

    /// Whether the filter narrows the table. Counts the same as `Filters::active_count`.
    pub fn is_active(self, filters: &Filters) -> bool {
        let f = filters;
        match self {
            Facet::Date => f.date_from.is_some() || f.date_to.is_some(),
            Facet::Days => !f.days_of_week.is_empty(),
            Facet::Amount => f.amount_min.is_some() || f.amount_max.is_some(),
            Facet::Currency => f.currency.is_some(),
            Facet::Types => !f.transaction_types.is_empty(),
            Facet::Channel => f.source_channel.is_some(),
            Facet::Chats => !f.source_chat_ids.is_empty(),
            Facet::Operators => !f.operators.is_empty(),
            Facet::Apps => !f.apps.is_empty(),
            Facet::Method => f.parsing_method.is_some(),
            Facet::Confidence => f.confidence_min.is_some() || f.confidence_max.is_some(),
            Facet::Details => !f.annotation_details.trim().is_empty(),
            Facet::Period => f.annotation_period_from.is_some() || f.annotation_period_to.is_some(),
            Facet::State => f.annotation_state != AnnotationState::All,
            Facet::Persons => !f.annotation_persons.is_empty(),
            Facet::Articles => !f.annotation_articles.is_empty(),
            Facet::Card => f.card.is_some(),
        }
    }

    /// What the filter is set to, in words; nothing while it is off.
    pub fn value(self, filters: &Filters, names: &Names<'_>) -> Option<String> {
        let f = filters;
        match self {
            Facet::Date => stretch(f.date_from.map(day), f.date_to.map(day)),
            Facet::Days => (!f.days_of_week.is_empty()).then(|| weekdays(&f.days_of_week)),
            Facet::Amount => bounds(f.amount_min.map(number), f.amount_max.map(number)),
            Facet::Currency => f.currency.clone(),
            Facet::Types => listed(f.transaction_types.iter().map(|code| named(&TYPES, code))),
            Facet::Channel => f.source_channel.as_deref().map(|code| named(&CHANNELS, code).to_string()),
            Facet::Chats => {
                let titles: Vec<String> = f.source_chat_ids.iter().map(|id| names.chats.iter().find(|(known, _)| known == id).map_or_else(|| id.to_string(), |(_, title)| title.clone())).collect();
                listed(titles.iter().map(String::as_str))
            }
            Facet::Operators => listed(f.operators.iter().map(String::as_str)),
            Facet::Apps => listed(f.apps.iter().map(String::as_str)),
            Facet::Method => f.parsing_method.as_deref().map(|code| named(&METHODS, code).to_string()),
            Facet::Confidence => bounds(f.confidence_min.map(percent), f.confidence_max.map(percent)),
            Facet::Details => Some(f.annotation_details.trim()).filter(|text| !text.is_empty()).map(|text| shorten(text, NAME_LENGTH)),
            Facet::Period => stretch(f.annotation_period_from.map(month), f.annotation_period_to.map(month)),
            Facet::State => match f.annotation_state {
                AnnotationState::All => None,
                AnnotationState::Annotated => Some("есть".to_string()),
                AnnotationState::Missing => Some("нет".to_string()),
            },
            Facet::Persons => listed(f.annotation_persons.iter().map(|code| looked_up(names.persons, code))),
            Facet::Articles => listed(f.annotation_articles.iter().map(|code| looked_up(names.articles, code))),
            Facet::Card => f.card.clone(),
        }
    }

    /// The words of the chip that stands for the filter above the table.
    pub fn chip(self, filters: &Filters, names: &Names<'_>) -> Option<String> {
        self.value(filters, names).map(|value| format!("{}: {value}", self.label()))
    }

    /// Whether the filter is set the same way in both.
    pub fn same(self, a: &Filters, b: &Filters) -> bool {
        match self {
            Facet::Date => (a.date_from, a.date_to) == (b.date_from, b.date_to),
            Facet::Days => alike(&a.days_of_week, &b.days_of_week),
            Facet::Amount => (a.amount_min, a.amount_max) == (b.amount_min, b.amount_max),
            Facet::Currency => a.currency == b.currency,
            Facet::Types => alike(&a.transaction_types, &b.transaction_types),
            Facet::Channel => a.source_channel == b.source_channel,
            Facet::Chats => alike(&a.source_chat_ids, &b.source_chat_ids),
            Facet::Operators => alike(&a.operators, &b.operators),
            Facet::Apps => alike(&a.apps, &b.apps),
            Facet::Method => a.parsing_method == b.parsing_method,
            Facet::Confidence => (a.confidence_min, a.confidence_max) == (b.confidence_min, b.confidence_max),
            Facet::Details => a.annotation_details.trim() == b.annotation_details.trim(),
            Facet::Period => (a.annotation_period_from, a.annotation_period_to) == (b.annotation_period_from, b.annotation_period_to),
            Facet::State => a.annotation_state == b.annotation_state,
            Facet::Persons => alike(&a.annotation_persons, &b.annotation_persons),
            Facet::Articles => alike(&a.annotation_articles, &b.annotation_articles),
            Facet::Card => a.card == b.card,
        }
    }

    /// Takes the filter off and leaves every other one as it is.
    pub fn clear(self, filters: &mut Filters) {
        let f = filters;
        match self {
            Facet::Date => (f.date_from, f.date_to) = (None, None),
            Facet::Days => f.days_of_week.clear(),
            Facet::Amount => (f.amount_min, f.amount_max) = (None, None),
            Facet::Currency => f.currency = None,
            Facet::Types => f.transaction_types.clear(),
            Facet::Channel => f.source_channel = None,
            Facet::Chats => f.source_chat_ids.clear(),
            Facet::Operators => f.operators.clear(),
            Facet::Apps => f.apps.clear(),
            Facet::Method => f.parsing_method = None,
            Facet::Confidence => (f.confidence_min, f.confidence_max) = (None, None),
            Facet::Details => f.annotation_details.clear(),
            Facet::Period => (f.annotation_period_from, f.annotation_period_to) = (None, None),
            Facet::State => f.annotation_state = AnnotationState::All,
            Facet::Persons => f.annotation_persons.clear(),
            Facet::Articles => f.annotation_articles.clear(),
            Facet::Card => f.card = None,
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(year, month, day)
    }

    /// Every filter on at once, with values the form writes back as it read them.
    pub fn rich() -> Filters {
        Filters {
            search: "korzinka".into(),
            date_from: date(2026, 11, 1),
            date_to: date(2026, 11, 11),
            days_of_week: vec![1, 3, 0],
            amount_min: Some(1000.0),
            amount_max: Some(5000.0),
            currency: Some("USD".into()),
            transaction_types: vec!["DEBIT".into(), "REVERSAL".into()],
            operators: vec!["Korzinka".into(), "Магазин Ромашка".into(), "Кофейня Арбуз".into(), "Такси Комета".into()],
            apps: vec!["Payme".into()],
            source_channel: Some("TELEGRAM".into()),
            source_chat_ids: vec![-100500, 42],
            card: Some("1234".into()),
            parsing_method: Some("gpt".into()),
            confidence_min: Some(0.25),
            confidence_max: Some(0.9),
            annotation_details: "ст парковка".into(),
            annotation_persons: vec!["Б".into(), "ИВ".into()],
            annotation_articles: vec!["ст".into()],
            annotation_period_from: Some(202603),
            annotation_period_to: Some(202611),
            annotation_state: AnnotationState::Missing,
        }
    }

    /// `rich` with every filter off but one.
    fn only(facet: Facet) -> Filters {
        let mut filters = rich();
        for other in Facet::ALL.into_iter().filter(|other| *other != facet) {
            other.clear(&mut filters);
        }
        filters
    }

    fn chips(filters: &Filters, names: &Names<'_>) -> Vec<String> {
        Facet::ALL.into_iter().filter_map(|facet| facet.chip(filters, names)).collect()
    }

    #[test]
    fn every_filter_has_a_chip_that_says_what_it_holds() {
        let persons = [("Б".to_string(), "Борисов Б.".to_string())];
        let articles = [("ст".to_string(), "Стоянка".to_string())];
        let chats = [(42_i64, "Касса · чеки".to_string())];
        let names = Names { persons: &persons, articles: &articles, chats: &chats };
        assert_eq!(
            chips(&rich(), &names),
            [
                "Дата: 01.11.2026 – 11.11.2026",
                "Дни недели: пн, ср, вс",
                "Сумма: от 1\u{a0}000 до 5\u{a0}000",
                "Валюта: USD",
                "Тип: Списание, Отмена",
                "Источник: Telegram",
                "Чат: -100500, Касса · чеки",
                "Оператор: Korzinka, Магазин Ромашка +2",
                "Приложение: Payme",
                "Парсер: GPT",
                "Уверенность: от 25% до 90%",
                "Детали: ст парковка",
                "Период учёта: 03.2026 – 11.2026",
                "Разметка: нет",
                "Лицо: Борисов Б., ИВ",
                "Статья: Стоянка",
                "Карта: 1234",
            ]
        );
        // Without the dictionaries the codes stand for themselves.
        let bare = chips(&rich(), &Names::default());
        assert_eq!((bare[6].as_str(), bare[14].as_str(), bare[15].as_str()), ("Чат: -100500, 42", "Лицо: Б, ИВ", "Статья: ст"));
        assert_eq!(chips(&Filters::default(), &names), Vec::<String>::new());
        // The search is not a filter of the sheet and gets no chip.
        assert_eq!(chips(&Filters { search: "korzinka".into(), ..Default::default() }, &names), Vec::<String>::new());
    }

    #[test]
    fn a_range_says_which_of_its_ends_are_set() {
        let chip = |facet: Facet, filters: Filters| facet.chip(&filters, &Names::default()).unwrap();
        let base = Filters::default;
        assert_eq!(chip(Facet::Date, Filters { date_from: date(2026, 11, 5), ..base() }), "Дата: с 05.11.2026");
        assert_eq!(chip(Facet::Date, Filters { date_to: date(2026, 11, 5), ..base() }), "Дата: по 05.11.2026");
        // One day at both ends is that day.
        assert_eq!(chip(Facet::Date, Filters { date_from: date(2026, 11, 5), date_to: date(2026, 11, 5), ..base() }), "Дата: 05.11.2026");
        assert_eq!(chip(Facet::Amount, Filters { amount_min: Some(1500.5), ..base() }), "Сумма: от 1\u{a0}500,5");
        assert_eq!(chip(Facet::Amount, Filters { amount_max: Some(2_500_000.0), ..base() }), "Сумма: до 2\u{a0}500\u{a0}000");
        assert_eq!(chip(Facet::Amount, Filters { amount_min: Some(700.0), amount_max: Some(700.0), ..base() }), "Сумма: 700");
        assert_eq!(chip(Facet::Confidence, Filters { confidence_min: Some(0.6), ..base() }), "Уверенность: от 60%");
        assert_eq!(chip(Facet::Confidence, Filters { confidence_max: Some(0.07), ..base() }), "Уверенность: до 7%");
        assert_eq!(chip(Facet::Period, Filters { annotation_period_from: Some(202601), ..base() }), "Период учёта: с 01.2026");
        assert_eq!(chip(Facet::Period, Filters { annotation_period_to: Some(202512), ..base() }), "Период учёта: по 12.2025");
        assert_eq!(chip(Facet::Period, Filters { annotation_period_from: Some(202601), annotation_period_to: Some(202601), ..base() }), "Период учёта: 01.2026");
        assert_eq!(chip(Facet::State, Filters { annotation_state: AnnotationState::Annotated, ..base() }), "Разметка: есть");
    }

    #[test]
    fn a_list_names_two_and_counts_the_rest() {
        let chip = |facet: Facet, filters: Filters| facet.chip(&filters, &Names::default()).unwrap();
        let apps = |names: &[&str]| Filters { apps: names.iter().map(|name| name.to_string()).collect(), ..Default::default() };
        assert_eq!(chip(Facet::Apps, apps(&["Payme"])), "Приложение: Payme");
        assert_eq!(chip(Facet::Apps, apps(&["Payme", "Click"])), "Приложение: Payme, Click");
        assert_eq!(chip(Facet::Apps, apps(&["Payme", "Click", "Uzum"])), "Приложение: Payme, Click +1");
        assert_eq!(chip(Facet::Apps, apps(&["Payme", "Click", "Uzum", "Paynet", "Humans"])), "Приложение: Payme, Click +3");
        // A long name is cut, so the chip stays one short line and still says how many there are.
        let long = "Общество с ограниченной ответственностью «Ромашка Маркет»";
        assert_eq!(chip(Facet::Operators, Filters { operators: vec![long.into(), long.into(), "x".into()], ..Default::default() }), "Оператор: Общество с ограниченной отв…, Общество с ограниченной отв… +1");
        let details = chip(Facet::Details, Filters { annotation_details: format!("  {long}  "), ..Default::default() });
        assert_eq!(details, "Детали: Общество с ограниченной отв…");
        // Every day of the week reads in the order of the week, however they were ticked.
        assert_eq!(chip(Facet::Days, Filters { days_of_week: vec![0, 6, 5, 4, 3, 2, 1], ..Default::default() }), "Дни недели: пн, вт, ср, чт, пт, сб, вс");
    }

    #[test]
    fn a_value_of_another_build_is_shown_as_it_is_stored() {
        let filters = Filters {
            currency: Some("EUR".into()),
            transaction_types: vec!["REFUND".into(), "DEBIT".into()],
            source_channel: Some("EMAIL".into()),
            parsing_method: Some("vision".into()),
            days_of_week: vec![2, 9],
            ..Default::default()
        };
        assert_eq!(chips(&filters, &Names::default()), ["Дни недели: вт, 9", "Валюта: EUR", "Тип: REFUND, Списание", "Источник: EMAIL", "Парсер: vision"]);
    }

    #[test]
    fn the_chips_count_what_the_button_counts() {
        let mut cases = vec![Filters::default(), rich(), Filters { search: "x".into(), annotation_details: "   ".into(), ..Default::default() }];
        cases.extend(Facet::ALL.into_iter().map(only));
        // Half a range is a filter too, and an empty text of another build counts as the button counts it.
        cases.push(Filters { date_to: date(2026, 1, 1), amount_min: Some(0.0), confidence_max: Some(1.0), annotation_period_from: Some(202601), ..Default::default() });
        cases.push(Filters { currency: Some(String::new()), card: Some(String::new()), ..Default::default() });
        for filters in cases {
            let active = Facet::ALL.into_iter().filter(|facet| facet.is_active(&filters)).count();
            assert_eq!(active, filters.active_count(), "{filters:?}");
            assert_eq!(chips(&filters, &Names::default()).len(), active, "{filters:?}");
            for facet in Facet::ALL {
                assert_eq!(facet.value(&filters, &Names::default()).is_some(), facet.is_active(&filters), "{facet:?} of {filters:?}");
            }
        }
        for facet in Facet::ALL {
            assert_eq!(only(facet).active_count(), 1, "{facet:?}");
            assert!(facet.is_active(&only(facet)));
        }
    }

    #[test]
    fn taking_one_filter_off_leaves_the_others() {
        let full = rich();
        for facet in Facet::ALL {
            let mut left = full.clone();
            facet.clear(&mut left);
            assert!(!facet.is_active(&left), "{facet:?} is still on");
            assert_eq!(facet.chip(&left, &Names::default()), None);
            assert_eq!(left.active_count(), Facet::ALL.len() - 1, "{facet:?}");
            for other in Facet::ALL.into_iter().filter(|other| *other != facet) {
                assert!(other.same(&left, &full), "{other:?} moved with {facet:?}");
            }
            assert!(!facet.same(&left, &full));
            assert_eq!(left.search, full.search);
            // Taken off twice, it is as off as once.
            let once = left.clone();
            facet.clear(&mut left);
            assert_eq!(left, once);
        }
    }

    #[test]
    fn clearing_everything_keeps_the_search() {
        let blank = cleared(&rich());
        assert_eq!(blank, Filters { search: "korzinka".into(), ..Default::default() });
        assert_eq!(blank.active_count(), 0);
        assert_eq!(cleared(&Filters::default()), Filters::default());
    }

    #[test]
    fn filters_agree_whatever_the_search_and_the_order_of_ticks() {
        let full = rich();
        assert!(agree(&full, &full));
        assert!(agree(&full, &Filters { search: String::new(), ..full.clone() }));
        let shuffled = Filters {
            days_of_week: vec![0, 1, 3],
            transaction_types: vec!["REVERSAL".into(), "DEBIT".into()],
            operators: full.operators.iter().rev().cloned().collect(),
            source_chat_ids: vec![42, -100500],
            annotation_persons: vec!["ИВ".into(), "Б".into()],
            annotation_details: " ст парковка ".into(),
            ..full.clone()
        };
        assert!(agree(&full, &shuffled));
        // Any one filter set otherwise is a disagreement, and so is one taken off.
        for facet in Facet::ALL {
            let mut other = full.clone();
            facet.clear(&mut other);
            assert!(!agree(&full, &other), "{facet:?}");
            assert!(!agree(&other, &full), "{facet:?}");
        }
        assert!(!agree(&full, &Filters { amount_max: Some(5000.5), ..full.clone() }));
        assert!(!agree(&full, &Filters { operators: vec!["Korzinka".into()], ..full.clone() }));
        assert!(agree(&Filters::default(), &Filters { search: "x".into(), annotation_details: "  ".into(), ..Default::default() }));
    }

    #[test]
    fn a_long_line_is_cut_to_its_length() {
        assert_eq!(shorten("Ромашка", 7), "Ромашка");
        assert_eq!(shorten("Ромашка", 6), "Ромаш…");
        assert_eq!(shorten("Ромашка", 1), "…");
        assert_eq!(shorten("", 5), "");
        assert_eq!(shorten("Ромашка Маркет", 8).chars().count(), 8);
    }
}
