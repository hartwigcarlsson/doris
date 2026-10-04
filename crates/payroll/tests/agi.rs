use doris_payroll::agi::*;
use doris_payroll::domain::DomainError;
use jiff::civil::date;

#[test]
fn a_period_is_six_digits_year_and_month() {
    let p = Period::parse(" 202610 ").unwrap();
    assert_eq!(p.get(), 202610);
    assert_eq!(p.to_string(), "202610");
    assert_eq!(p.first_day(), date(2026, 10, 1));
    assert_eq!(Period::of(date(2026, 10, 25)), p);
    assert!(Period::parse("202612").is_ok());
    for bad in [
        "", "202613", "202600", "2026-10", "26010", "2026100", "18991", "189912", "2026１0",
    ] {
        assert_eq!(
            Period::parse(bad),
            Err(DomainError::InvalidPeriod),
            "{bad:?}"
        );
    }
    assert!(Period::parse("202609").unwrap() < p);
}

#[test]
fn a_contact_follows_skatteverkets_schema() {
    let ok = AgiContact::parse(" Anna Andersson ", " 070-123 45 67 ", " anna@example.se ").unwrap();
    assert_eq!(
        ok,
        AgiContact {
            name: "Anna Andersson".into(),
            phone: "070-123 45 67".into(),
            email: "anna@example.se".into()
        }
    );
    assert!(
        AgiContact::parse(
            &"å".repeat(50),
            &"1".repeat(20),
            "a.b-c+d'e_f@x-y.example.se"
        )
        .is_ok()
    );
    let refused = |n: &str, p: &str, e: &str| {
        AgiContact::parse(n, p, e) == Err(DomainError::InvalidAgiContact)
    };
    assert!(refused(&"å".repeat(51), "070", "a@b.se"), "name too long");
    assert!(refused("Anna", &"1".repeat(21), "a@b.se"), "phone too long");
    assert!(refused("", "070", "a@b.se"), "no name");
    assert!(refused("Anna", "   ", "a@b.se"), "blank phone");
    assert!(refused("Anna <AB>", "070", "a@b.se"), "angle brackets");
    for email in [
        "",
        "a@b",
        "a@b.",
        "@b.se",
        "a@.se",
        "a..b@c.se",
        "a@b..se",
        "a b@c.se",
        "a@b@c.se",
        "å@b.se",
        "a@b.s",
        "ab.se",
    ] {
        // "a@b.s" is valid by the pattern; only the length rule (5) refuses "a@b." etc.
        if email == "a@b.s" {
            assert!(AgiContact::parse("Anna", "070", email).is_ok(), "{email:?}");
        } else {
            assert!(refused("Anna", "070", email), "{email:?}");
        }
    }
}
