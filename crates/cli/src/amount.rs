//! Kronor in, öre inside, exact strings out.

/// Öre from kronor typed as `1250`, `1250.5` or `1250,50`: no sign, no
/// grouping, at most two decimals.
pub fn parse_kronor(raw: &str) -> Option<i64> {
    let (whole, fraction) = raw.split_once(['.', ',']).unwrap_or((raw, ""));
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || (raw.len() > whole.len() && !digits(fraction)) || fraction.len() > 2 {
        return None;
    }
    let ore: i64 = format!("{fraction:0<2}").parse().ok()?;
    whole
        .parse::<i64>()
        .ok()?
        .checked_mul(100)?
        .checked_add(ore)
}

/// Öre as an exact decimal string in kronor, for JSON: `"1250.00"`.
pub fn kronor(ore: i64) -> String {
    let sign = if ore < 0 { "-" } else { "" };
    let ore = ore.unsigned_abs();
    format!("{sign}{}.{:02}", ore / 100, ore % 100)
}

/// Öre as the web shows kronor, with spaces between thousands: `"1 250,00"`.
pub fn display(ore: i64) -> String {
    let sign = if ore < 0 { "-" } else { "" };
    let ore = ore.unsigned_abs();
    let whole = (ore / 100).to_string();
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    format!("{sign}{grouped},{:02}", ore % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts() {
        assert_eq!(parse_kronor("1250"), Some(125_000));
        assert_eq!(parse_kronor("1250.5"), Some(125_050));
        assert_eq!(parse_kronor("1250,50"), Some(125_050));
        assert_eq!(parse_kronor("0.01"), Some(1));
        for bad in [
            "", "-80", "+80", "1250.505", "1 250", "12,5,0", "abc", ".5", "5.",
        ] {
            assert_eq!(parse_kronor(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn kronor_are_written_exactly() {
        assert_eq!(kronor(125_000), "1250.00");
        assert_eq!(kronor(1), "0.01");
        assert_eq!(kronor(-8_000), "-80.00");
        assert_eq!(display(125_050), "1 250,50");
        assert_eq!(display(-100_000_000), "-1 000 000,00");
    }
}
