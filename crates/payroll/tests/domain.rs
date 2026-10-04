use doris_payroll::domain::*;

#[test]
fn a_personnummer_has_twelve_digits_a_date_and_a_check_digit() {
    let pin = PersonalIdentityNumber::parse(" 19800101-1231 ").unwrap();
    assert_eq!(pin.as_str(), "198001011231");
    assert_eq!(pin.formatted(), "19800101-1231");
    assert_eq!(pin.birth_year(), 1980);
    assert_eq!(PersonalIdentityNumber::parse("198001011231").unwrap(), pin);
    // A samordningsnummer: day + 60.
    assert!(PersonalIdentityNumber::parse("19800161-1238").is_ok());
}

#[test]
fn anything_else_is_not_a_personnummer() {
    for bad in [
        "",
        "19800101-1232",   // check digit
        "800101-1231",     // ten digits: the century is unknown
        "8001011231",
        "19800230-1235",   // 30 February
        "19800192-1231",   // samordningsnummer day 92
        "19800101+1231",
        "1980010l-1231",
        "19800101–1231",   // en dash: not ASCII, never sliced
        "١٩٨٠٠١٠١١٢٣١",
        "1980-01-01-1231",
    ] {
        assert_eq!(
            PersonalIdentityNumber::parse(bad),
            Err(DomainError::InvalidPersonalIdentityNumber),
            "{bad:?}"
        );
    }
}

#[test]
fn employee_names_are_trimmed_and_1_to_100_characters() {
    assert_eq!(EmployeeName::parse("  Åsa Öberg ").unwrap().as_str(), "Åsa Öberg");
    assert!(EmployeeName::parse(&"å".repeat(100)).is_ok());
    for bad in ["", "   ", &"å".repeat(101)] {
        assert_eq!(EmployeeName::parse(bad), Err(DomainError::InvalidEmployeeName));
    }
}

#[test]
fn salary_accounts_are_7010_7210_or_7220() {
    for ok in [7010, 7210, 7220] {
        assert_eq!(SalaryAccount::parse(ok).unwrap().get(), ok);
    }
    assert_eq!(SalaryAccount::DEFAULT.get(), 7210);
    for bad in [0, 7211, 1930, 7510] {
        assert_eq!(SalaryAccount::parse(bad), Err(DomainError::InvalidSalaryAccount));
    }
}
