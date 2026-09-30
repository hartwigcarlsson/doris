use doris_company::domain::*;

#[test]
fn org_nr_accepts_common_spellings() {
    for raw in [
        "556016-0680",
        "5560160680",
        " 556016 0680 ",
        "16556016-0680",
        "165560160680",
    ] {
        let org_nr = OrgNr::parse(raw).unwrap();
        assert_eq!(org_nr.as_str(), "5560160680", "{raw:?}");
        assert_eq!(org_nr.formatted(), "556016-0680");
    }
}

#[test]
fn org_nr_accepts_a_personnummer_for_enskild_firma() {
    let org_nr = OrgNr::parse("19121212-1212").unwrap();
    assert_eq!(org_nr.as_str(), "1212121212");
    assert!(org_nr.is_personal_identity_number());
    assert!(
        !OrgNr::parse("556016-0680")
            .unwrap()
            .is_personal_identity_number()
    );
}

#[test]
fn org_nr_rejects_bad_check_digits_and_formats() {
    for raw in [
        "",
        "556016-0681",
        "5599999999",
        "55601606",
        "55601606800",
        "556016-068O",
        "185560160680",
    ] {
        assert_eq!(OrgNr::parse(raw), Err(DomainError::InvalidOrgNr), "{raw:?}");
    }
}

#[test]
fn org_nr_debug_output_hides_the_digits() {
    let org_nr = OrgNr::parse("556016-0680").unwrap();
    let debug_str = format!("{:?}", org_nr);
    assert!(
        !debug_str.contains("5560160680"),
        "debug output should not contain the digits"
    );
    assert_eq!(debug_str, "OrgNr(<redacted>)");
}
