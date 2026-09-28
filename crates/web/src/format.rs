//! Small display-formatting helpers shared by the pages.

/// The `YYYY-MM-DD` date portion of an RFC 3339 timestamp. Falls back to the
/// whole string when it is too short to slice, so a malformed timestamp
/// never panics the app.
pub fn date(timestamp: &str) -> &str {
    timestamp.get(..10).unwrap_or(timestamp)
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
}
