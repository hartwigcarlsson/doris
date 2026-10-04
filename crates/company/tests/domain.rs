use doris_company::domain::*;
use jiff::civil::Date;
use uuid::Uuid;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

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

#[test]
fn company_name_is_trimmed_and_1_to_200_characters() {
    assert_eq!(
        CompanyName::parse("  Exempel AB ").unwrap().as_str(),
        "Exempel AB"
    );
    assert!(CompanyName::parse(&"å".repeat(200)).is_ok());
    assert_eq!(
        CompanyName::parse("  "),
        Err(DomainError::InvalidCompanyName)
    );
    assert_eq!(
        CompanyName::parse(&"å".repeat(201)),
        Err(DomainError::InvalidCompanyName)
    );
}

#[test]
fn address_fields_are_trimmed_and_empty_ones_dropped() {
    let address = Address::parse(" Storgatan 1 ", "", " Stockholm").unwrap();
    assert_eq!(address.street.as_deref(), Some("Storgatan 1"));
    assert_eq!(address.postal_code, None);
    assert_eq!(address.city.as_deref(), Some("Stockholm"));
    assert_eq!(
        Address::parse(&"a".repeat(201), "", ""),
        Err(DomainError::InvalidAddress)
    );
}

#[test]
fn a_calendar_year_is_a_valid_first_fiscal_year() {
    let year = FiscalYear::first(d("2026-01-01"), d("2026-12-31"), LegalForm::Aktiebolag).unwrap();
    assert_eq!((year.start, year.end), (d("2026-01-01"), d("2026-12-31")));
}

#[test]
fn a_first_fiscal_year_may_be_short_or_extended_up_to_18_months() {
    for (start, end) in [
        ("2026-10-01", "2026-10-31"), // 1 month
        ("2026-07-01", "2027-12-31"), // 18 months
        ("2026-05-01", "2027-04-30"), // broken year
        ("2027-03-01", "2028-02-29"), // ends on a leap day
    ] {
        assert!(
            FiscalYear::first(d(start), d(end), LegalForm::Aktiebolag).is_ok(),
            "{start}–{end}"
        );
    }
}

#[test]
fn a_first_fiscal_year_must_follow_bfl_3_kap() {
    for (start, end) in [
        ("2026-01-02", "2026-12-31"), // not the first of a month
        ("2026-01-01", "2026-12-30"), // not the last of a month
        ("2026-07-01", "2028-01-31"), // 19 months
        ("2026-12-01", "2026-11-30"), // ends before it starts
        ("2027-03-01", "2028-02-28"), // 2028 is a leap year: not month end
    ] {
        assert_eq!(
            FiscalYear::first(d(start), d(end), LegalForm::Aktiebolag),
            Err(DomainError::InvalidFiscalYear),
            "{start}–{end}"
        );
    }
}

#[test]
fn enskild_firma_and_handelsbolag_must_use_the_calendar_year() {
    for form in [
        LegalForm::EnskildFirma,
        LegalForm::Handelsbolag,
        LegalForm::Kommanditbolag,
    ] {
        assert_eq!(
            FiscalYear::first(d("2026-05-01"), d("2027-04-30"), form),
            Err(DomainError::InvalidFiscalYear),
            "{form:?}"
        );
        // Starting mid-year is fine as long as the year ends 31 December.
        assert!(FiscalYear::first(d("2026-06-01"), d("2026-12-31"), form).is_ok());
    }
}

#[test]
fn later_fiscal_years_are_12_months_ending_in_the_same_month() {
    let first = FiscalYear::first(d("2026-07-01"), d("2027-12-31"), LegalForm::Aktiebolag).unwrap();
    let broken =
        FiscalYear::first(d("2026-05-01"), d("2027-04-30"), LegalForm::Aktiebolag).unwrap();

    assert_eq!(
        first.next(),
        FiscalYear {
            start: d("2028-01-01"),
            end: d("2028-12-31")
        }
    );
    assert_eq!(
        broken.next(),
        FiscalYear {
            start: d("2027-05-01"),
            end: d("2028-04-30")
        }
    );
    assert_eq!(first.containing(d("2026-09-30")), first);
    assert_eq!(first.containing(d("2026-01-15")), first); // before the company existed
    assert_eq!(broken.containing(d("2029-02-28")).start, d("2028-05-01"));
}

fn registered(creator: Uuid) -> (Company, Vec<CompanyEvent>) {
    let events = register_company(
        RegisterCompany {
            company_id: Uuid::new_v4(),
            org_nr: OrgNr::parse("556016-0680").unwrap(),
            name: CompanyName::parse("Exempel AB").unwrap(),
            legal_form: LegalForm::Aktiebolag,
            address: Address::default(),
            first_fiscal_year: FiscalYear::first(
                d("2026-01-01"),
                d("2026-12-31"),
                LegalForm::Aktiebolag,
            )
            .unwrap(),
            accounting_method: AccountingMethod::Invoice,
        },
        creator,
    );
    (Company::from_events(&events).unwrap(), events)
}

#[test]
fn registering_a_company_makes_the_creator_its_first_member() {
    let anna = Uuid::new_v4();
    let (company, events) = registered(anna);

    assert!(
        matches!(events[0], CompanyEvent::CompanyRegistered { created_by, .. } if created_by == anna)
    );
    assert_eq!(
        events[1],
        CompanyEvent::MemberAdded {
            user_id: anna,
            added_by: anna
        }
    );
    assert_eq!(company.members, vec![anna]);
    assert_eq!(company.name.as_str(), "Exempel AB");
}

#[test]
fn a_member_adds_another_user_once() {
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let (company, mut events) = registered(anna);

    let added = add_member(&company, anna, bo).unwrap();
    events.extend(added.clone());
    let company = Company::from_events(&events).unwrap();

    assert_eq!(
        added,
        vec![CompanyEvent::MemberAdded {
            user_id: bo,
            added_by: anna
        }]
    );
    assert!(company.is_member(bo));
    assert_eq!(add_member(&company, anna, bo).unwrap(), vec![]);
}

#[test]
fn a_non_member_cannot_add_members() {
    let (company, _) = registered(Uuid::new_v4());
    let stranger = Uuid::new_v4();
    assert_eq!(
        add_member(&company, stranger, stranger),
        Err(DomainError::NotMember)
    );
}

#[test]
fn events_round_trip_through_json_with_readable_dates() {
    let (_, events) = registered(Uuid::new_v4());
    let json = serde_json::to_value(&events[0]).unwrap();
    assert_eq!(json["type"], "CompanyRegistered");
    assert_eq!(json["org_nr"], "5560160680");
    assert_eq!(json["first_fiscal_year"]["start"], "2026-01-01");
    assert_eq!(json["accounting_method"], "invoice");
    assert_eq!(
        serde_json::from_value::<CompanyEvent>(json).unwrap(),
        events[0]
    );
}

#[test]
fn luhn_counts_from_the_right_for_any_length() {
    assert!(luhn("5560160680"));
    assert!(!luhn("5560160681"));
    // Seven and two digits: bankgiro 123-4566 and plusgiro 1-8.
    assert!(luhn("1234566"));
    assert!(luhn("18"));
    assert!(!luhn("1234567"));
}
