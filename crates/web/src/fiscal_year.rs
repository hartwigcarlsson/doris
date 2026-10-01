//! The first räkenskapsår's end, derived from its start (BFL 3 kap.), so the
//! form only asks for the start. The server validates whatever is sent.

use crate::api::cpb;

/// The end of a 12-month räkenskapsår starting on `start` (`YYYY-MM-DD`,
/// the 1st of a month). Enskild firma and handelsbolag must follow the
/// calendar year, so theirs ends 31 December, which shortens a first year
/// that starts mid-year. `None` if `start` isn't the 1st of a month.
pub fn default_end(start: &str, legal_form: cpb::LegalForm) -> Option<String> {
    let (year, month) = match start.as_bytes() {
        [_, _, _, _, b'-', _, _, b'-', b'0', b'1'] => (
            start[..4].parse::<u32>().ok()?,
            start[5..7].parse::<u32>().ok()?,
        ),
        _ => return None,
    };
    if !(1..=12).contains(&month) {
        return None;
    }
    let calendar_year = matches!(
        legal_form,
        cpb::LegalForm::EnskildFirma
            | cpb::LegalForm::Handelsbolag
            | cpb::LegalForm::Kommanditbolag
    );
    // The last day of the month before `month`, one year on.
    let (year, month) = match (calendar_year, month) {
        (true, _) | (false, 1) => (year, 12),
        (false, _) => (year + 1, month - 1),
    };
    Some(format!(
        "{year:04}-{month:02}-{:02}",
        days_in_month(year, month)
    ))
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpb::LegalForm::*;

    #[test]
    fn a_year_starting_in_january_ends_31_december() {
        assert_eq!(
            default_end("2026-01-01", Aktiebolag).as_deref(),
            Some("2026-12-31")
        );
    }

    #[test]
    fn a_broken_year_ends_the_day_before_the_same_date_next_year() {
        assert_eq!(
            default_end("2026-05-01", Aktiebolag).as_deref(),
            Some("2027-04-30")
        );
        assert_eq!(
            default_end("2027-03-01", Stiftelse).as_deref(),
            Some("2028-02-29")
        );
        assert_eq!(
            default_end("2026-03-01", Unspecified).as_deref(),
            Some("2027-02-28")
        );
    }

    #[test]
    fn enskild_firma_and_handelsbolag_end_31_december_the_same_year() {
        for form in [EnskildFirma, Handelsbolag, Kommanditbolag] {
            assert_eq!(
                default_end("2026-05-01", form).as_deref(),
                Some("2026-12-31"),
                "{form:?}"
            );
        }
    }

    #[test]
    fn a_start_that_is_not_the_first_of_a_month_has_no_default_end() {
        for start in ["2026-05-02", "", "2026-5-1", "2026-13-01", "not a date"] {
            assert_eq!(default_end(start, Aktiebolag), None, "{start:?}");
        }
    }
}
