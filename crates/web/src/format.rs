//! Small display-formatting helpers shared by the pages.

use crate::api::cpb;

/// The `YYYY-MM-DD` date portion of an RFC 3339 timestamp. Falls back to the
/// whole string when it is too short to slice, so a malformed timestamp
/// never panics the app.
pub fn date(timestamp: &str) -> &str {
    timestamp.get(..10).unwrap_or(timestamp)
}

/// Legal forms in the order the form offers them.
pub const LEGAL_FORMS: [cpb::LegalForm; 8] = [
    cpb::LegalForm::Aktiebolag,
    cpb::LegalForm::EnskildFirma,
    cpb::LegalForm::Handelsbolag,
    cpb::LegalForm::Kommanditbolag,
    cpb::LegalForm::EkonomiskForening,
    cpb::LegalForm::IdeellForening,
    cpb::LegalForm::Stiftelse,
    cpb::LegalForm::Other,
];

pub fn legal_form_label(form: cpb::LegalForm) -> &'static str {
    match form {
        cpb::LegalForm::Unspecified => "Välj…",
        cpb::LegalForm::Aktiebolag => "Aktiebolag",
        cpb::LegalForm::Handelsbolag => "Handelsbolag",
        cpb::LegalForm::Kommanditbolag => "Kommanditbolag",
        cpb::LegalForm::EnskildFirma => "Enskild firma",
        cpb::LegalForm::EkonomiskForening => "Ekonomisk förening",
        cpb::LegalForm::IdeellForening => "Ideell förening",
        cpb::LegalForm::Stiftelse => "Stiftelse",
        cpb::LegalForm::Other => "Annan",
    }
}

pub fn accounting_method_label(method: cpb::AccountingMethod) -> &'static str {
    match method {
        cpb::AccountingMethod::Cash => "Kontantmetoden",
        cpb::AccountingMethod::Invoice => "Faktureringsmetoden",
        cpb::AccountingMethod::Unspecified => "",
    }
}

/// This year by the browser's clock, for the default räkenskapsår.
pub fn current_year() -> i32 {
    js_sys::Date::new_0().get_full_year() as i32
}

/// Kronor as typed in Sweden ("1 234,50", "1234.5", "12") to öre. Spaces,
/// no-break spaces and narrow no-break spaces group thousands; comma or
/// point marks the decimals, of which there are at most two. Anything else,
/// including a sign or an empty field, is `None`.
pub fn parse_amount(raw: &str) -> Option<i64> {
    let digits: String = raw
        .chars()
        .filter(|c| !matches!(c, ' ' | '\u{a0}' | '\u{202f}'))
        .collect();
    let (kronor, ore) = digits.split_once([',', '.']).unwrap_or((&digits, ""));
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if kronor.is_empty() || ore.len() > 2 || !all_digits(kronor) || !all_digits(ore) {
        return None;
    }
    let ore: i64 = format!("{ore:0<2}").parse().ok()?;
    kronor
        .parse::<i64>()
        .ok()?
        .checked_mul(100)?
        .checked_add(ore)
}

/// Öre as kronor, thousands grouped with no-break spaces: 123450 → "1 234,50".
pub fn amount(ore: i64) -> String {
    let sign = if ore < 0 { "-" } else { "" };
    let ore = ore.unsigned_abs();
    let kronor = (ore / 100).to_string();
    let mut grouped = String::new();
    for (i, digit) in kronor.chars().enumerate() {
        if i > 0 && (kronor.len() - i).is_multiple_of(3) {
            grouped.push('\u{a0}');
        }
        grouped.push(digit);
    }
    format!("{sign}{grouped},{:02}", ore % 100)
}

/// Today by the browser's clock and time zone, as `YYYY-MM-DD`.
pub fn today() -> String {
    let now = js_sys::Date::new_0();
    format!(
        "{:04}-{:02}-{:02}",
        now.get_full_year(),
        now.get_month() + 1,
        now.get_date()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_amount_reads_swedish_and_plain_spellings() {
        assert_eq!(parse_amount("1 234,50"), Some(123_450));
        assert_eq!(parse_amount("1\u{a0}234,50"), Some(123_450));
        assert_eq!(parse_amount("1\u{202f}234,5"), Some(123_450));
        assert_eq!(parse_amount("1234.5"), Some(123_450));
        assert_eq!(parse_amount("12"), Some(1_200));
        assert_eq!(parse_amount(" 0,05 "), Some(5));
        assert_eq!(parse_amount("12,"), Some(1_200));
    }

    #[test]
    fn parse_amount_refuses_anything_else() {
        for bad in [
            "",
            " ",
            "-5",
            "+5",
            "1,234",
            "1,2,3",
            "1.2.3",
            "abc",
            "12 kr",
            ",50",
            "99999999999999999999",
        ] {
            assert_eq!(parse_amount(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn amount_formats_ore_as_kronor_with_grouped_thousands() {
        assert_eq!(amount(0), "0,00");
        assert_eq!(amount(5), "0,05");
        assert_eq!(amount(123_450), "1\u{a0}234,50");
        assert_eq!(amount(100_000_000), "1\u{a0}000\u{a0}000,00");
        assert_eq!(amount(-123_450), "-1\u{a0}234,50");
    }

    #[test]
    fn full_rfc3339_timestamp_yields_date_part() {
        assert_eq!(date("2026-09-28T12:34:56Z"), "2026-09-28");
    }

    #[test]
    fn short_string_is_returned_unchanged() {
        assert_eq!(date("2026"), "2026");
    }

    #[test]
    fn empty_string_is_returned_unchanged() {
        assert_eq!(date(""), "");
    }

    #[test]
    fn every_legal_form_has_a_swedish_label() {
        assert_eq!(legal_form_label(cpb::LegalForm::Aktiebolag), "Aktiebolag");
        assert_eq!(
            legal_form_label(cpb::LegalForm::EnskildFirma),
            "Enskild firma"
        );
        for form in LEGAL_FORMS {
            assert!(!legal_form_label(form).is_empty());
        }
        assert!(!LEGAL_FORMS.contains(&cpb::LegalForm::Unspecified));
    }
}
