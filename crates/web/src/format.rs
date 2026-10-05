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

/// Days from 1970-01-01 to `date` (`YYYY-MM-DD`), or `None` if it isn't a
/// date. Howard Hinnant's civil-day arithmetic, so no date library goes
/// into the wasm.
pub fn day_number(date: &str) -> Option<i64> {
    let mut parts = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// `date` (`YYYY-MM-DD`) plus `days`, or `None` if it isn't a date.
pub fn plus_days(date: &str, days: i64) -> Option<String> {
    let z = day_number(date)? + 719_468 + days;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

// Used by Verifikationer from the next commits on.
#[allow(dead_code)]
/// A UTC timestamp (`YYYY-MM-DDTHH:MM…Z`) as local `YYYY-MM-DD HH:MM`,
/// `offset_minutes` east of UTC. "" if it isn't one.
pub fn local_time(utc: &str, offset_minutes: i32) -> String {
    let parsed = (|| {
        let (date, time) = utc.split_once('T')?;
        if !time.ends_with('Z') || time.as_bytes().get(2) != Some(&b':') {
            return None;
        }
        let hour: i64 = time.get(..2)?.parse().ok()?;
        let minute: i64 = time.get(3..5)?.parse().ok()?;
        if hour > 23 || minute > 59 {
            return None;
        }
        // Minutes since midnight in the local zone, and the days that shifts.
        let minutes = hour * 60 + minute + i64::from(offset_minutes);
        let date = plus_days(date, minutes.div_euclid(1440))?;
        let minutes = minutes.rem_euclid(1440);
        Some(format!("{date} {:02}:{:02}", minutes / 60, minutes % 60))
    })();
    parsed.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn plus_days_crosses_months_years_and_leap_days() {
        assert_eq!(plus_days("2026-01-31", 30).as_deref(), Some("2026-03-02"));
        assert_eq!(plus_days("2026-12-15", 30).as_deref(), Some("2027-01-14"));
        assert_eq!(plus_days("2024-02-01", 29).as_deref(), Some("2024-03-01"));
        assert_eq!(plus_days("2026-03-01", 0).as_deref(), Some("2026-03-01"));
        assert_eq!(plus_days("idag", 30), None);
        assert_eq!(plus_days("2026-13-01", 30), None);
    }

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

    #[test]
    fn day_numbers_count_from_1970() {
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(day_number("1970-01-02"), Some(1));
        assert_eq!(day_number("2026-10-04"), Some(20_730));
        assert_eq!(day_number("2026-13-01"), None);
        assert_eq!(day_number("nonsense"), None);
        assert_eq!(day_number(""), None);
    }

    #[test]
    fn a_utc_time_is_shown_in_the_local_zone_without_seconds() {
        assert_eq!(
            local_time("2026-10-02T12:12:45.123Z", 120),
            "2026-10-02 14:12"
        );
        assert_eq!(local_time("2026-10-02T12:12:45Z", 0), "2026-10-02 12:12");
        assert_eq!(
            local_time("2026-01-15T08:05:00.000Z", 60),
            "2026-01-15 09:05"
        );
    }

    #[test]
    fn local_time_rolls_over_midnight_and_new_year() {
        assert_eq!(
            local_time("2026-10-02T23:30:00.000Z", 120),
            "2026-10-03 01:30"
        );
        assert_eq!(
            local_time("2026-12-31T23:30:00.000Z", 60),
            "2027-01-01 00:30"
        );
        assert_eq!(
            local_time("2027-01-01T00:30:00.000Z", -300),
            "2026-12-31 19:30"
        );
        assert_eq!(
            local_time("2028-02-28T23:59:00.000Z", 60),
            "2028-02-29 00:59"
        );
        assert_eq!(
            local_time("2026-03-01T00:00:00.000Z", -1),
            "2026-02-28 23:59"
        );
    }

    #[test]
    fn a_time_that_is_not_one_is_empty() {
        for raw in [
            "",
            "2026-10-02",
            "nonsense",
            "2026-10-02T25:00:00Z",
            "2026-10-02T12:60:00Z",
            "2026-13-02T12:00:00Z",
            "2026-10-02 12:12",
        ] {
            assert_eq!(local_time(raw, 120), "", "{raw}");
        }
    }
}
