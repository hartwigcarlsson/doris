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

#[cfg(test)]
mod tests {
    use super::*;

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
