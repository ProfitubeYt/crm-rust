//! The ledger's «Операция» cell: the receipt's own first line when the bank
//! put a headline there, and a label of ours when it did not.
//!
//! This is the customer's workbook rule (`OperationDisplay` in
//! `backend/tools/excel_sync/ExcelSync.bas`), kept the same on purpose: people
//! read the table against the workbook cell by cell.

use std::cmp::Ordering;

use crate::model::Transaction;

/// A longer first line is a sentence. Counted in code points.
const MAX_HEADLINE: usize = 60;

/// Asks for the character before it to be drawn as an emoji. The `ℹ` of
/// `ℹ️` is a letter to Unicode and a symbol only by this.
const EMOJI_PRESENTATION: char = '\u{fe0f}';

/// These open the amount, balance, merchant, card and date lines of a receipt.
const FIELD_MARKERS: [char; 11] = ['➖', '➕', '💵', '💰', '📍', '🏪', '💳', '🕓', '🕘', '🕒', '📅'];

/// What makes a HUMO debit a cash withdrawal, in the operator or the text.
const WITHDRAWAL_HINTS: [&str; 5] = ["снятие", "обнал", "atm", "банкомат", "bxo"];

/// `ℹ️`, spelled out so that the selector cannot be lost in an editor.
const NOTICE_MARK: &str = "\u{2139}\u{fe0f}";
/// `↩️`, likewise.
const REVERSAL_MARK: &str = "\u{21a9}\u{fe0f}";

/// Unicode 14.0 general category `S*` as inclusive ranges, ascending. The
/// workbook carries this table and the server reads Python 3.11's copy of it;
/// a newer one would take emoji added since for headlines in this client only.
const SYMBOLS: [(u32, u32); 234] = [
    (0x24, 0x24), (0x2B, 0x2B), (0x3C, 0x3E), (0x5E, 0x5E), (0x60, 0x60), (0x7C, 0x7C), (0x7E, 0x7E), (0xA2, 0xA6),
    (0xA8, 0xA9), (0xAC, 0xAC), (0xAE, 0xB1), (0xB4, 0xB4), (0xB8, 0xB8), (0xD7, 0xD7), (0xF7, 0xF7), (0x2C2, 0x2C5),
    (0x2D2, 0x2DF), (0x2E5, 0x2EB), (0x2ED, 0x2ED), (0x2EF, 0x2FF), (0x375, 0x375), (0x384, 0x385), (0x3F6, 0x3F6), (0x482, 0x482),
    (0x58D, 0x58F), (0x606, 0x608), (0x60B, 0x60B), (0x60E, 0x60F), (0x6DE, 0x6DE), (0x6E9, 0x6E9), (0x6FD, 0x6FE), (0x7F6, 0x7F6),
    (0x7FE, 0x7FF), (0x888, 0x888), (0x9F2, 0x9F3), (0x9FA, 0x9FB), (0xAF1, 0xAF1), (0xB70, 0xB70), (0xBF3, 0xBFA), (0xC7F, 0xC7F),
    (0xD4F, 0xD4F), (0xD79, 0xD79), (0xE3F, 0xE3F), (0xF01, 0xF03), (0xF13, 0xF13), (0xF15, 0xF17), (0xF1A, 0xF1F), (0xF34, 0xF34),
    (0xF36, 0xF36), (0xF38, 0xF38), (0xFBE, 0xFC5), (0xFC7, 0xFCC), (0xFCE, 0xFCF), (0xFD5, 0xFD8), (0x109E, 0x109F), (0x1390, 0x1399),
    (0x166D, 0x166D), (0x17DB, 0x17DB), (0x1940, 0x1940), (0x19DE, 0x19FF), (0x1B61, 0x1B6A), (0x1B74, 0x1B7C), (0x1FBD, 0x1FBD), (0x1FBF, 0x1FC1),
    (0x1FCD, 0x1FCF), (0x1FDD, 0x1FDF), (0x1FED, 0x1FEF), (0x1FFD, 0x1FFE), (0x2044, 0x2044), (0x2052, 0x2052), (0x207A, 0x207C), (0x208A, 0x208C),
    (0x20A0, 0x20C0), (0x2100, 0x2101), (0x2103, 0x2106), (0x2108, 0x2109), (0x2114, 0x2114), (0x2116, 0x2118), (0x211E, 0x2123), (0x2125, 0x2125),
    (0x2127, 0x2127), (0x2129, 0x2129), (0x212E, 0x212E), (0x213A, 0x213B), (0x2140, 0x2144), (0x214A, 0x214D), (0x214F, 0x214F), (0x218A, 0x218B),
    (0x2190, 0x2307), (0x230C, 0x2328), (0x232B, 0x2426), (0x2440, 0x244A), (0x249C, 0x24E9), (0x2500, 0x2767), (0x2794, 0x27C4), (0x27C7, 0x27E5),
    (0x27F0, 0x2982), (0x2999, 0x29D7), (0x29DC, 0x29FB), (0x29FE, 0x2B73), (0x2B76, 0x2B95), (0x2B97, 0x2BFF), (0x2CE5, 0x2CEA), (0x2E50, 0x2E51),
    (0x2E80, 0x2E99), (0x2E9B, 0x2EF3), (0x2F00, 0x2FD5), (0x2FF0, 0x2FFB), (0x3004, 0x3004), (0x3012, 0x3013), (0x3020, 0x3020), (0x3036, 0x3037),
    (0x303E, 0x303F), (0x309B, 0x309C), (0x3190, 0x3191), (0x3196, 0x319F), (0x31C0, 0x31E3), (0x3200, 0x321E), (0x322A, 0x3247), (0x3250, 0x3250),
    (0x3260, 0x327F), (0x328A, 0x32B0), (0x32C0, 0x33FF), (0x4DC0, 0x4DFF), (0xA490, 0xA4C6), (0xA700, 0xA716), (0xA720, 0xA721), (0xA789, 0xA78A),
    (0xA828, 0xA82B), (0xA836, 0xA839), (0xAA77, 0xAA79), (0xAB5B, 0xAB5B), (0xAB6A, 0xAB6B), (0xFB29, 0xFB29), (0xFBB2, 0xFBC2), (0xFD40, 0xFD4F),
    (0xFDCF, 0xFDCF), (0xFDFC, 0xFDFF), (0xFE62, 0xFE62), (0xFE64, 0xFE66), (0xFE69, 0xFE69), (0xFF04, 0xFF04), (0xFF0B, 0xFF0B), (0xFF1C, 0xFF1E),
    (0xFF3E, 0xFF3E), (0xFF40, 0xFF40), (0xFF5C, 0xFF5C), (0xFF5E, 0xFF5E), (0xFFE0, 0xFFE6), (0xFFE8, 0xFFEE), (0xFFFC, 0xFFFD), (0x10137, 0x1013F),
    (0x10179, 0x10189), (0x1018C, 0x1018E), (0x10190, 0x1019C), (0x101A0, 0x101A0), (0x101D0, 0x101FC), (0x10877, 0x10878), (0x10AC8, 0x10AC8), (0x1173F, 0x1173F),
    (0x11FD5, 0x11FF1), (0x16B3C, 0x16B3F), (0x16B45, 0x16B45), (0x1BC9C, 0x1BC9C), (0x1CF50, 0x1CFC3), (0x1D000, 0x1D0F5), (0x1D100, 0x1D126), (0x1D129, 0x1D164),
    (0x1D16A, 0x1D16C), (0x1D183, 0x1D184), (0x1D18C, 0x1D1A9), (0x1D1AE, 0x1D1EA), (0x1D200, 0x1D241), (0x1D245, 0x1D245), (0x1D300, 0x1D356), (0x1D6C1, 0x1D6C1),
    (0x1D6DB, 0x1D6DB), (0x1D6FB, 0x1D6FB), (0x1D715, 0x1D715), (0x1D735, 0x1D735), (0x1D74F, 0x1D74F), (0x1D76F, 0x1D76F), (0x1D789, 0x1D789), (0x1D7A9, 0x1D7A9),
    (0x1D7C3, 0x1D7C3), (0x1D800, 0x1D9FF), (0x1DA37, 0x1DA3A), (0x1DA6D, 0x1DA74), (0x1DA76, 0x1DA83), (0x1DA85, 0x1DA86), (0x1E14F, 0x1E14F), (0x1E2FF, 0x1E2FF),
    (0x1ECAC, 0x1ECAC), (0x1ECB0, 0x1ECB0), (0x1ED2E, 0x1ED2E), (0x1EEF0, 0x1EEF1), (0x1F000, 0x1F02B), (0x1F030, 0x1F093), (0x1F0A0, 0x1F0AE), (0x1F0B1, 0x1F0BF),
    (0x1F0C1, 0x1F0CF), (0x1F0D1, 0x1F0F5), (0x1F10D, 0x1F1AD), (0x1F1E6, 0x1F202), (0x1F210, 0x1F23B), (0x1F240, 0x1F248), (0x1F250, 0x1F251), (0x1F260, 0x1F265),
    (0x1F300, 0x1F6D7), (0x1F6DD, 0x1F6EC), (0x1F6F0, 0x1F6FC), (0x1F700, 0x1F773), (0x1F780, 0x1F7D8), (0x1F7E0, 0x1F7EB), (0x1F7F0, 0x1F7F0), (0x1F800, 0x1F80B),
    (0x1F810, 0x1F847), (0x1F850, 0x1F859), (0x1F860, 0x1F887), (0x1F890, 0x1F8AD), (0x1F8B0, 0x1F8B1), (0x1F900, 0x1FA53), (0x1FA60, 0x1FA6D), (0x1FA70, 0x1FA74),
    (0x1FA78, 0x1FA7C), (0x1FA80, 0x1FA86), (0x1FA90, 0x1FAAC), (0x1FAB0, 0x1FABA), (0x1FAC0, 0x1FAC5), (0x1FAD0, 0x1FAD9), (0x1FAE0, 0x1FAE7), (0x1FAF0, 0x1FAF6),
    (0x1FB00, 0x1FB92), (0x1FB94, 0x1FBCA),
];

fn is_symbol(c: char) -> bool {
    let code = c as u32;
    let place = |&(low, high): &(u32, u32)| if high < code { Ordering::Less } else if low > code { Ordering::Greater } else { Ordering::Equal };
    SYMBOLS.binary_search_by(place).is_ok()
}

/// Whitespace as Python's `str.strip` knows it and the workbook copies it:
/// Rust's own, and the four ASCII separators.
fn is_space(c: char) -> bool {
    matches!(c, '\u{9}'..='\u{d}' | '\u{1c}'..='\u{20}' | '\u{85}' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

/// Where Python's `str.splitlines` ends a line.
fn is_line_end(c: char) -> bool {
    matches!(c, '\u{a}'..='\u{d}' | '\u{1c}'..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// The receipt's first line, when it is the bank's headline: short, and opened
/// by a symbol (`🔴`, `💸`) or by a character drawn as one (`ℹ️`).
pub fn headline(raw: &str) -> Option<&str> {
    let first = raw.split(is_line_end).map(|line| line.trim_matches(is_space)).find(|line| !line.is_empty())?;
    if first.chars().count() > MAX_HEADLINE {
        return None;
    }
    let mut chars = first.chars();
    let lead = chars.next()?;
    if !is_symbol(lead) && chars.next() != Some(EMOJI_PRESENTATION) {
        return None;
    }
    // `📍 SHOP` opens with a symbol too: a receipt that lost its headline must
    // not have its merchant or its amount read as the operation.
    (!FIELD_MARKERS.contains(&lead)).then_some(first)
}

/// The type «Операция» is worded by. The workbook goes by the type a receipt
/// is stored with, so that one is taken whenever the server sends it.
/// `transaction_type` is the server's reading of the receipt for this answer,
/// which is what «Тип» shows; a server from before `stored_transaction_type`
/// sends only that, and it stands in as it did.
fn stored_kind(tx: &Transaction) -> &str {
    tx.stored_transaction_type.as_deref().or(tx.transaction_type.as_deref()).unwrap_or("")
}

/// The text of the «Операция» cell: the headline as the bank wrote it, or a
/// mark and a label for the type, written the way the row's bank writes them.
pub fn display(tx: &Transaction) -> String {
    let raw = tx.raw_message.as_deref().unwrap_or("");
    if let Some(line) = headline(raw) {
        return line.to_string();
    }
    let kind = stored_kind(tx);
    let upper = raw.to_uppercase();
    let notice = upper.contains("СЧЕТ ПО КАРТЕ ИЗМЕН") || upper.contains("СЧЁТ ПО КАРТЕ ИЗМЕН");
    let label = if notice {
        "Изменение счета"
    } else {
        match kind {
            "CREDIT" => "Пополнение",
            "CONVERSION" => "Конверсия",
            "REVERSAL" => "Отмена",
            _ => "Списание",
        }
    };
    // The bank a row was given in the workbook outranks where the receipt came from.
    let bank = match tx.excel_bank.as_deref() {
        Some(given @ ("CardXabar" | "HUMO Card")) => given,
        _ => tx.source_display.as_deref().unwrap_or(""),
    };
    let mark = if !bank.to_uppercase().contains("HUMO") {
        // Whatever is not HUMO is written as CardXabar writes: one red dot for every operation.
        "🔴"
    } else if notice {
        NOTICE_MARK
    } else if kind == "DEBIT" && is_withdrawal(tx, raw) {
        "🏧"
    } else {
        match kind {
            "DEBIT" => "💸",
            "CREDIT" => "🎉",
            "CONVERSION" => "💱",
            "REVERSAL" => REVERSAL_MARK,
            _ => NOTICE_MARK,
        }
    };
    format!("{mark} {label}")
}

fn is_withdrawal(tx: &Transaction, raw: &str) -> bool {
    let haystack = format!("{} {raw}", tx.operator_raw.as_deref().unwrap_or("")).to_lowercase();
    WITHDRAWAL_HINTS.iter().any(|hint| haystack.contains(hint))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected cells are written by code point, as the workbook's self-test
    // writes them, so that no editor decides what an emoji is made of.
    const DOT: &str = "\u{1f534}";
    const CASH: &str = "\u{1f3e7}";
    const PAID: &str = "\u{1f4b8}";
    const TOPPED_UP: &str = "\u{1f389}";
    const EXCHANGED: &str = "\u{1f4b1}";
    const UNDONE: &str = "\u{21a9}\u{fe0f}";
    const INFO: &str = "\u{2139}\u{fe0f}";

    /// The workbook's `EmptyFields`: a debit, with `source_display` as its self-test sets it.
    fn row(raw: &str, source: &str) -> Transaction {
        Transaction { raw_message: Some(raw.into()), transaction_type: Some("DEBIT".into()), source_display: Some(source.into()), ..Default::default() }
    }

    fn typed(kind: &str, source: &str) -> Transaction {
        Transaction { transaction_type: Some(kind.into()), ..row("[Photo]", source) }
    }

    #[test]
    fn the_symbol_table_is_the_workbooks() {
        // 234 ranges and 7 741 code points: UnicodeData.txt 14.0.0, category S*.
        assert_eq!(SYMBOLS.len(), 234);
        assert_eq!(SYMBOLS.iter().map(|(low, high)| high - low + 1).sum::<u32>(), 7741);
        assert!(SYMBOLS.iter().all(|(low, high)| low <= high));
        // The search needs them ascending and apart.
        assert!(SYMBOLS.windows(2).all(|pair| pair[0].1 < pair[1].0));
        assert_eq!((SYMBOLS[0], SYMBOLS[233]), ((0x24, 0x24), (0x1FB94, 0x1FBCA)));

        for symbol in ['$', '+', '<', '=', '>', '^', '`', '|', '~', '©', '№', '₽', '←', '\u{2795}', '\u{1f534}', '\u{1f4b8}', '\u{1fbca}'] {
            assert!(is_symbol(symbol), "{symbol:?}");
        }
        for other in ['#', '%', '[', '*', '1', 'A', 'я', ' ', '\u{a0}', '\u{2139}', '\u{fe0f}', '\u{1fbcb}', '\u{10ffff}'] {
            assert!(!is_symbol(other), "{other:?}");
        }
        // Both ends of a range, and the gap inside a block.
        assert!(is_symbol('\u{1f300}') && is_symbol('\u{1f6d7}') && !is_symbol('\u{1f6d8}'));
        // Emoji of Unicode 15 and later are not symbols to the workbook.
        assert!(is_symbol('\u{1fae7}') && !is_symbol('\u{1fae8}'));
        assert!(is_symbol('\u{1f6dd}') && !is_symbol('\u{1f6dc}'));
    }

    #[test]
    fn the_field_markers_are_the_workbooks() {
        let codes = [0x2796, 0x2795, 0x1F4B5, 0x1F4B0, 0x1F4CD, 0x1F3EA, 0x1F4B3, 0x1F553, 0x1F558, 0x1F552, 0x1F4C5];
        assert_eq!(FIELD_MARKERS.map(|marker| marker as u32), codes);
        for code in codes {
            let marker = char::from_u32(code).unwrap();
            assert!(is_symbol(marker), "{marker:?} would be a headline but for the list");
            assert_eq!(headline(&format!("{marker} field")), None, "{marker:?}");
            assert_eq!(headline(&format!("{marker}\u{fe0f} field")), None, "{marker:?}");
        }
    }

    #[test]
    fn lines_are_cut_where_the_workbook_cuts_them() {
        // `ReceiptWhitespace` and the line ends of `FirstReceiptLine`, in the numbers the workbook writes.
        let spaces = [9, 10, 11, 12, 13, 28, 29, 30, 31, 32, 133, 160, 5760, 8192, 8193, 8194, 8195, 8196, 8197, 8198, 8199, 8200, 8201, 8202, 8232, 8233, 8239, 8287, 12288];
        let ends = [10, 11, 12, 13, 28, 29, 30, 133, 8232, 8233];
        let all = |test: fn(char) -> bool| (0..=0x10FFFF).filter_map(char::from_u32).filter(|c| test(*c)).map(|c| c as u32).collect::<Vec<_>>();
        assert_eq!(all(is_space), spaces);
        assert_eq!(all(is_line_end), ends);
    }

    /// `SelfTestOperations` of the workbook, its headline half.
    #[test]
    fn the_workbooks_own_headlines() {
        let headline_text = format!("{DOT} Platezh");
        let wrapped = row(&format!("\r\n\u{a0}{headline_text}  \r\n\u{1f4cd} Merchant"), "CardXabar");
        assert_eq!(display(&wrapped), headline_text, "the bank's headline is kept as it is");
        assert_eq!(display(&row("\u{2139}\u{fe0f} Notice\ndetails", "CardXabar")), "\u{2139}\u{fe0f} Notice");
        assert_eq!(headline("\u{2139} Notice"), None, "a letterlike symbol without the selector");
        assert_eq!(headline(&format!("[Photo]\n{headline_text}")), None, "an OCR preamble");
        assert_eq!(headline("Transfer through terminal"), None, "plain text");
        assert_eq!(headline("=literal header"), Some("=literal header"), "a mathematical symbol");
        // Sixty code points in sixty-one UTF-16 units and sixty-three bytes.
        let sixty = format!("{DOT}{}", "x".repeat(59));
        assert_eq!(headline(&sixty), Some(sixty.as_str()));
        assert_eq!(headline(&format!("{sixty}x")), None);
    }

    /// `SelfTestOperations` of the workbook, its fallback half, step by step.
    #[test]
    fn the_workbooks_own_fallbacks() {
        let mut tx = row("[Photo]", "CardXabar");
        tx.operator_raw = Some("ATM".into());
        assert_eq!(display(&tx), format!("{DOT} Списание"), "CardXabar");
        tx.source_display = Some("HUMO Card".into());
        assert_eq!(display(&tx), format!("{CASH} Списание"), "a HUMO withdrawal");
        tx.operator_raw = Some("shop".into());
        assert_eq!(display(&tx), format!("{PAID} Списание"), "a HUMO debit");
        tx.source_display = Some("Manual".into());
        tx.excel_bank = Some("HUMO Card".into());
        assert_eq!(display(&tx), format!("{PAID} Списание"), "a manual receipt given to HUMO");
        tx.excel_bank = Some("CardXabar".into());
        tx.source_display = Some("HUMO Card".into());
        assert_eq!(display(&tx), format!("{DOT} Списание"), "the bank given outranks the source");
        tx.excel_bank = None;
        tx.transaction_type = Some("CREDIT".into());
        assert_eq!(display(&tx), format!("{TOPPED_UP} Пополнение"));
        tx.transaction_type = Some("CONVERSION".into());
        assert_eq!(display(&tx), format!("{EXCHANGED} Конверсия"));
        tx.transaction_type = Some("REVERSAL".into());
        assert_eq!(display(&tx), format!("{UNDONE} Отмена"));
        tx.raw_message = Some("Счёт по карте изменен".into());
        assert_eq!(display(&tx), format!("{INFO} Изменение счета"));
        tx.transaction_type = Some("DEBIT".into());
        tx.raw_message = Some(format!("{DOT} Platezh"));
        assert_eq!(display(&tx), format!("{DOT} Platezh"));
    }

    /// What the server's tests ask of the same rule (`operation_from_raw`).
    #[test]
    fn the_servers_headlines() {
        let card = "🔴 E-Com oplata\n📍 DEMO SHOP, UZ\n➖ 5 000.00 UZS\n💳 ***0000\n🕓 01.01.26 07:16\n💵 1 000 000.00 UZS";
        assert_eq!(headline(card), Some("\u{1f534} E-Com oplata"));
        let humo = "💸 Оплата\n➖ 40.000,00 UZS\n📍 DEMO GOLD>Urt\n💳 HUMOCARD *0000\n🕓 04.01.2026 11:24\n💰 10.000,00 UZS";
        assert_eq!(headline(humo), Some("\u{1f4b8} Оплата"));
        assert_eq!(headline("\u{2139}\u{fe0f} Счет по карте изменен\n➕ 1.000.000,00 UZS"), Some("\u{2139}\u{fe0f} Счет по карте изменен"));
        assert_eq!(headline("[Photo]\nПеревод через Payme"), None);
        assert_eq!(display(&row("Перевод через Payme", "CardXabar")), format!("{DOT} Списание"));
        assert_eq!(headline("📍 DEMO SHOP, UZ\n➖ 5 000.00 UZS"), None);
        assert_eq!(headline("➖ 5 000.00 UZS\n💳 ***0000"), None);
        assert_eq!(headline("🕓 01.01.26 07:16"), None);
        assert_eq!(headline(&format!("{DOT} {}", "x".repeat(80))), None);
    }

    #[test]
    fn a_headline_is_measured_in_code_points() {
        // 120 UTF-16 units and 240 bytes, and still sixty characters.
        let sixty = DOT.repeat(60);
        assert_eq!(headline(&sixty), Some(sixty.as_str()));
        assert_eq!(headline(&DOT.repeat(61)), None);
        // The selector counts as one of them.
        let with_selector = format!("{INFO}{}", "я".repeat(58));
        assert_eq!(headline(&with_selector), Some(with_selector.as_str()));
        assert_eq!(headline(&format!("{with_selector}я")), None);
        assert_eq!(headline(DOT), Some(DOT));
    }

    #[test]
    fn the_first_line_is_cut_as_python_cuts_it() {
        let wanted = Some("\u{1f534} Platezh");
        // Blank lines, a non-breaking space, a tab and an ideographic space go.
        assert_eq!(headline("\n \r\n\u{a0}\t\u{1f534} Platezh\u{3000}\u{a0}\r\n\u{1f4cd} Merchant"), wanted);
        // Every separator `str.splitlines` knows ends the line.
        for end in ['\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}'] {
            assert_eq!(headline(&format!("{end}\u{1f534} Platezh{end}\u{1f4cd} Merchant")), wanted, "{end:?}");
        }
        // The unit separator is space to trim and not the end of a line; Rust's `trim` would leave it.
        assert_eq!(headline("\u{1f}\u{1f534} Platezh\u{1f}"), wanted);
        assert_eq!(headline("\u{1f534} Pla\u{1f}tezh"), Some("\u{1f534} Pla\u{1f}tezh"));
        // What is inside the line stays as it is.
        assert_eq!(headline("  \u{1f534}  E-Com \u{a0} oplata  "), Some("\u{1f534}  E-Com \u{a0} oplata"));
        // A zero-width space is not space to Python, so it opens the line and the line is no headline.
        assert_eq!(headline("\u{200b}\u{1f534} Platezh"), None);
    }

    #[test]
    fn a_first_line_without_a_symbol_is_not_a_headline() {
        for raw in ["[Photo]", "Перевод через терминал", "12:30 оплата", "* звёздочка", "#тег", "«Кавычки»", "— тире"] {
            assert_eq!(headline(raw), None, "{raw}");
        }
        // The second character may ask for an emoji, whatever the first is.
        assert_eq!(headline("1\u{fe0f}\u{20e3} шаг"), Some("1\u{fe0f}\u{20e3} шаг"));
        assert_eq!(headline("\u{1fae8}\u{fe0f} Shake"), Some("\u{1fae8}\u{fe0f} Shake"));
        // An emoji newer than the workbook's table is text to it, and so to us.
        assert_eq!(headline("\u{1fae8} Shake"), None);
        assert_eq!(headline("\u{1fae7} Bubbles"), Some("\u{1fae7} Bubbles"));
    }

    #[test]
    fn no_text_at_all_falls_back_to_the_label() {
        for raw in ["", " ", "\r\n", "\u{a0}\n\t\n"] {
            assert_eq!(headline(raw), None, "{raw:?}");
            assert_eq!(display(&row(raw, "CardXabar")), format!("{DOT} Списание"), "{raw:?}");
        }
        assert_eq!(display(&Transaction::default()), format!("{DOT} Списание"));
        let credit = Transaction { transaction_type: Some("CREDIT".into()), source_display: Some("HUMO Card".into()), ..Default::default() };
        assert_eq!(display(&credit), format!("{TOPPED_UP} Пополнение"));
    }

    #[test]
    fn every_other_bank_gets_the_red_dot() {
        for source in ["CardXabar", "Телеграм", "СМС", "Ручной", "Telegram 100", ""] {
            assert_eq!(display(&typed("DEBIT", source)), format!("{DOT} Списание"), "{source}");
            assert_eq!(display(&typed("CREDIT", source)), format!("{DOT} Пополнение"), "{source}");
            assert_eq!(display(&typed("CONVERSION", source)), format!("{DOT} Конверсия"), "{source}");
            assert_eq!(display(&typed("REVERSAL", source)), format!("{DOT} Отмена"), "{source}");
            assert_eq!(display(&typed("OTHER", source)), format!("{DOT} Списание"), "{source}");
            assert_eq!(display(&row("Счет по карте изменен", source)), format!("{DOT} Изменение счета"), "{source}");
        }
        let unnamed = Transaction { source_display: None, transaction_type: None, ..typed("DEBIT", "") };
        assert_eq!(display(&unnamed), format!("{DOT} Списание"));
        // Even a withdrawal: the dot is all CardXabar writes.
        let cash = Transaction { operator_raw: Some("ATM 0001".into()), ..typed("DEBIT", "CardXabar") };
        assert_eq!(display(&cash), format!("{DOT} Списание"));
    }

    #[test]
    fn humo_gets_a_mark_for_each_type() {
        for source in ["HUMO Card", "humo", "Чат Humo 2"] {
            assert_eq!(display(&typed("DEBIT", source)), format!("{PAID} Списание"), "{source}");
            assert_eq!(display(&typed("CREDIT", source)), format!("{TOPPED_UP} Пополнение"), "{source}");
            assert_eq!(display(&typed("CONVERSION", source)), format!("{EXCHANGED} Конверсия"), "{source}");
            assert_eq!(display(&typed("REVERSAL", source)), format!("{UNDONE} Отмена"), "{source}");
            // A type the workbook does not know reads as a debit, marked as a notice.
            assert_eq!(display(&typed("OTHER", source)), format!("{INFO} Списание"), "{source}");
            assert_eq!(display(&typed("debit", source)), format!("{INFO} Списание"), "{source}");
            let untyped = Transaction { transaction_type: None, ..typed("DEBIT", source) };
            assert_eq!(display(&untyped), format!("{INFO} Списание"), "{source}");
        }
    }

    #[test]
    fn a_balance_notice_outranks_the_type() {
        for raw in ["Счет по карте изменен", "счёт по карте изменён на 5 000", "[Photo]\nВаш СЧЕТ ПО КАРТЕ ИЗМЕНИЛСЯ"] {
            for kind in ["DEBIT", "CREDIT", "CONVERSION", "REVERSAL", ""] {
                let tx = Transaction { transaction_type: Some(kind.into()), ..row(raw, "HUMO Card") };
                assert_eq!(display(&tx), format!("{INFO} Изменение счета"), "{raw} as {kind}");
            }
        }
        // Also where a debit would otherwise be a withdrawal.
        let cash = Transaction { operator_raw: Some("ATM 0001".into()), ..row("Счет по карте изменен", "HUMO Card") };
        assert_eq!(display(&cash), format!("{INFO} Изменение счета"));
        // The words have to be these.
        assert_eq!(display(&row("Счет карты изменен", "HUMO Card")), format!("{PAID} Списание"));
    }

    #[test]
    fn a_humo_debit_at_a_cash_machine_is_a_withdrawal() {
        let at = |operator: &str, raw: &str, kind: &str| {
            display(&Transaction { operator_raw: Some(operator.into()), transaction_type: Some(kind.into()), ..row(raw, "HUMO Card") })
        };
        for operator in ["Снятие наличных", "ОБНАЛ", "ATM 0001", "Банкомат", "BXO TOSHKENT", "atmosfera cafe"] {
            assert_eq!(at(operator, "[Photo]", "DEBIT"), format!("{CASH} Списание"), "{operator}");
        }
        // The receipt's text is read as well as the operator.
        for raw in ["Перевод\nснятие в банкомате", "[Photo]\nBXO 0001", "обналичивание", "Выдача наличных ATM"] {
            assert_eq!(at("shop", raw, "DEBIT"), format!("{CASH} Списание"), "{raw}");
        }
        let unnamed = Transaction { operator_raw: None, ..row("Выдача в банкомате", "HUMO Card") };
        assert_eq!(display(&unnamed), format!("{CASH} Списание"));
        // Only a debit can be one.
        assert_eq!(at("ATM 0001", "[Photo]", "CREDIT"), format!("{TOPPED_UP} Пополнение"));
        assert_eq!(at("ATM 0001", "[Photo]", "REVERSAL"), format!("{UNDONE} Отмена"));
        assert_eq!(at("ATM 0001", "[Photo]", ""), format!("{INFO} Списание"));
        assert_eq!(at("shop", "[Photo]", "DEBIT"), format!("{PAID} Списание"));
    }

    #[test]
    fn the_bank_given_in_the_workbook_outranks_the_source() {
        let given = |bank: &str, source: &str| display(&Transaction { excel_bank: Some(bank.into()), ..typed("DEBIT", source) });
        assert_eq!(given("HUMO Card", "CardXabar"), format!("{PAID} Списание"));
        assert_eq!(given("HUMO Card", "Ручной"), format!("{PAID} Списание"));
        assert_eq!(given("CardXabar", "HUMO Card"), format!("{DOT} Списание"));
        // Only the two names the workbook writes count; anything else leaves the source to decide.
        for other in ["", "HUMO", "humo card", "Humo Card", "cardxabar", "Kapital"] {
            assert_eq!(given(other, "CardXabar"), format!("{DOT} Списание"), "{other}");
            assert_eq!(given(other, "HUMO Card"), format!("{PAID} Списание"), "{other}");
        }
    }

    /// A receipt without a headline: stored with one type, read by the server as another.
    fn kept(stored: Option<&str>, read: Option<&str>, source: &str) -> Transaction {
        Transaction { stored_transaction_type: stored.map(Into::into), transaction_type: read.map(Into::into), ..row("[Photo]", source) }
    }

    /// What a HUMO receipt of each type reads as without a headline.
    fn humo_cells() -> [(&'static str, String); 4] {
        [("DEBIT", format!("{PAID} Списание")), ("CREDIT", format!("{TOPPED_UP} Пополнение")), ("CONVERSION", format!("{EXCHANGED} Конверсия")), ("REVERSAL", format!("{UNDONE} Отмена"))]
    }

    #[test]
    fn the_stored_type_words_the_operation_when_the_server_sends_it() {
        // Stored as a top-up and read again as a debit: the workbook has the stored one, and so has the cell.
        assert_eq!(display(&kept(Some("CREDIT"), Some("DEBIT"), "HUMO Card")), format!("{TOPPED_UP} Пополнение"));
        assert_eq!(display(&kept(Some("CREDIT"), Some("DEBIT"), "CardXabar")), format!("{DOT} Пополнение"));
        assert_eq!(display(&kept(Some("DEBIT"), Some("CREDIT"), "CardXabar")), format!("{DOT} Списание"));
        // Whatever the server's own reading says, and also when it says nothing.
        for (stored, cell) in humo_cells() {
            for read in [Some("DEBIT"), Some("CREDIT"), Some("CONVERSION"), Some("REVERSAL"), Some(""), None] {
                assert_eq!(display(&kept(Some(stored), read, "HUMO Card")), cell, "{stored} stored, {read:?} read");
            }
        }
        // A stored type the workbook has no word for is worded as the workbook words it, not swapped for the reading.
        assert_eq!(display(&kept(Some("HOLD"), Some("CREDIT"), "HUMO Card")), format!("{INFO} Списание"));
        assert_eq!(display(&kept(Some(""), Some("CREDIT"), "HUMO Card")), format!("{INFO} Списание"));
    }

    #[test]
    fn an_older_server_leaves_the_operation_to_the_type_it_sends() {
        // No `stored_transaction_type` in the answer: the cell is what it was before the field.
        for (read, cell) in humo_cells() {
            assert_eq!(display(&kept(None, Some(read), "HUMO Card")), cell, "{read}");
            assert_eq!(display(&kept(None, Some(read), "HUMO Card")), display(&typed(read, "HUMO Card")), "{read}");
            assert_eq!(display(&kept(None, Some(read), "CardXabar")), display(&typed(read, "CardXabar")), "{read}");
        }
        // Neither type sent: the workbook's own last resort, as before.
        assert_eq!(display(&kept(None, None, "HUMO Card")), format!("{INFO} Списание"));
        assert_eq!(display(&kept(None, None, "CardXabar")), format!("{DOT} Списание"));
    }

    #[test]
    fn a_withdrawal_is_told_by_the_stored_type() {
        let atm = |stored: Option<&str>, read: &str| display(&Transaction { operator_raw: Some("ATM 0001".into()), ..kept(stored, Some(read), "HUMO Card") });
        // A receipt stored as a debit at a cash machine is a withdrawal, whatever the server reads it as.
        assert_eq!(atm(Some("DEBIT"), "CREDIT"), format!("{CASH} Списание"));
        assert_eq!(atm(Some("DEBIT"), "DEBIT"), format!("{CASH} Списание"));
        // Only a debit can be one, and it is the stored type that says which it is.
        assert_eq!(atm(Some("CREDIT"), "DEBIT"), format!("{TOPPED_UP} Пополнение"));
        assert_eq!(atm(Some("REVERSAL"), "DEBIT"), format!("{UNDONE} Отмена"));
        // An older server: the type it sends decides, as before.
        assert_eq!(atm(None, "DEBIT"), format!("{CASH} Списание"));
        assert_eq!(atm(None, "CREDIT"), format!("{TOPPED_UP} Пополнение"));
    }

    #[test]
    fn the_stored_type_leaves_the_headline_the_notice_and_the_bank_alone() {
        // A headline is the receipt's own line, whatever either type says.
        let headed = Transaction { stored_transaction_type: Some("CREDIT".into()), ..row("\u{1f534} Platezh\n➖ 5 000.00 UZS", "HUMO Card") };
        assert_eq!(display(&headed), format!("{DOT} Platezh"));
        // A notice of a changed account is one by its text.
        let notice = |source: &str| display(&Transaction { stored_transaction_type: Some("CREDIT".into()), ..row("Счет по карте изменен", source) });
        assert_eq!(notice("HUMO Card"), format!("{INFO} Изменение счета"));
        assert_eq!(notice("CardXabar"), format!("{DOT} Изменение счета"));
        // The bank given in the workbook still outranks the source, with the stored type as with the read one.
        let given = |bank: &str, source: &str| display(&Transaction { excel_bank: Some(bank.into()), ..kept(Some("REVERSAL"), Some("DEBIT"), source) });
        assert_eq!(given("HUMO Card", "CardXabar"), format!("{UNDONE} Отмена"));
        assert_eq!(given("HUMO Card", "Ручной"), format!("{UNDONE} Отмена"));
        assert_eq!(given("CardXabar", "HUMO Card"), format!("{DOT} Отмена"));
        assert_eq!(given("Kapital", "HUMO Card"), format!("{UNDONE} Отмена"));
    }

    #[test]
    fn a_headline_is_shown_whatever_the_row_says() {
        let tx = Transaction {
            transaction_type: Some("CREDIT".into()),
            excel_bank: Some("HUMO Card".into()),
            operator_raw: Some("ATM 0001".into()),
            ..row("\u{1f534} Platezh\n➖ 5 000.00 UZS", "HUMO Card")
        };
        assert_eq!(display(&tx), format!("{DOT} Platezh"));
    }
}
