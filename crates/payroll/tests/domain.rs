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

use jiff::civil::{Date, date};

const KR: i64 = 100;

fn fee(birth_year: i16, pay_date: Date, gross: i64) -> (u32, i64) {
    employer_fee(birth_year, pay_date, gross, 0)
}

#[test]
fn the_fee_depends_on_the_year_of_birth() {
    let may = date(2026, 5, 25);
    assert_eq!(fee(1937, may, 10_000 * KR), (0, 0));
    assert_eq!(fee(1938, may, 10_000 * KR), (OLD_AGE_RATE, 102_100));
    assert_eq!(fee(1958, may, 10_000 * KR), (OLD_AGE_RATE, 102_100));
    assert_eq!(fee(1959, may, 10_000 * KR), (FULL_RATE, 314_200));
    assert_eq!(fee(1980, may, 10_000 * KR), (FULL_RATE, 314_200));
    assert_eq!(fee(2002, may, 10_000 * KR), (FULL_RATE, 314_200));
    assert_eq!(fee(2003, may, 10_000 * KR), (YOUTH_RATE, 208_100));
    assert_eq!(fee(2007, may, 10_000 * KR), (YOUTH_RATE, 208_100));
    assert_eq!(fee(2008, may, 10_000 * KR), (FULL_RATE, 314_200));
}

#[test]
fn the_youth_reduction_runs_from_april_2026_to_september_2027() {
    assert_eq!(fee(2005, date(2026, 3, 31), 10_000 * KR).0, FULL_RATE);
    assert_eq!(fee(2005, date(2026, 4, 1), 10_000 * KR).0, YOUTH_RATE);
    assert_eq!(fee(2005, date(2027, 9, 30), 10_000 * KR).0, YOUTH_RATE);
    assert_eq!(fee(2005, date(2027, 10, 1), 10_000 * KR).0, FULL_RATE);
    // In 2027 the age band is born 2004-2008.
    assert_eq!(fee(2003, date(2027, 5, 25), 10_000 * KR).0, FULL_RATE);
    assert_eq!(fee(2008, date(2027, 5, 25), 10_000 * KR).0, YOUTH_RATE);
}

#[test]
fn the_youth_rate_covers_25000_kr_a_month() {
    let may = date(2026, 5, 25);
    // 25 000 × 20,81 % + 5 000 × 31,42 % = 5 202,50 + 1 571 = 6 773,50 kr.
    assert_eq!(fee(2005, may, 30_000 * KR), (YOUTH_RATE, 677_350));
    // 20 000 kr already paid this month: 5 000 at 20,81 %, 15 000 at 31,42 %.
    assert_eq!(
        employer_fee(2005, may, 20_000 * KR, 20_000 * KR),
        (YOUTH_RATE, 104_050 + 471_300)
    );
    // The cap already used up: all at 31,42 %.
    assert_eq!(employer_fee(2005, may, 1_000 * KR, 25_000 * KR), (YOUTH_RATE, 31_420));
}

#[test]
fn the_fee_rounds_half_an_ore_up() {
    let may = date(2026, 5, 25);
    // 25,00 kr × 31,42 % = 7,855 kr.
    assert_eq!(fee(1980, may, 2_500), (FULL_RATE, 786));
    // 24,99 kr × 31,42 % = 7,85186 kr.
    assert_eq!(fee(1980, may, 2_499), (FULL_RATE, 785));
}

use std::collections::HashSet;
use uuid::Uuid;

fn pin(raw: &str) -> PersonalIdentityNumber {
    PersonalIdentityNumber::parse(raw).unwrap()
}

fn name(raw: &str) -> EmployeeName {
    EmployeeName::parse(raw).unwrap()
}

fn hired(id: Uuid, personnummer: &str, monthly_salary: i64) -> PayrollEvent {
    PayrollEvent::EmployeeAdded {
        employee_id: id,
        name: name("Åsa Öberg"),
        personal_identity_number: pin(personnummer),
        monthly_salary,
        salary_account: SalaryAccount::DEFAULT,
    }
}

fn given(events: &[PayrollEvent]) -> Payroll {
    Payroll::from_events(events, HashSet::new())
}

#[test]
fn an_employee_is_added_once_per_personnummer() {
    let id = Uuid::new_v4();
    let cmd = |employee_id| AddEmployee {
        employee_id,
        name: name("Åsa Öberg"),
        personal_identity_number: pin("19800101-1231"),
        monthly_salary: 35_000 * KR,
        salary_account: SalaryAccount::DEFAULT,
    };

    assert_eq!(add_employee(&given(&[]), cmd(id)), Ok(vec![hired(id, "19800101-1231", 35_000 * KR)]));

    let existing = given(&[hired(id, "19800101-1231", 35_000 * KR)]);
    assert_eq!(add_employee(&existing, cmd(Uuid::new_v4())), Err(DomainError::DuplicateEmployee));
    // Also when the existing employee is inactive.
    let inactive = given(&[
        hired(id, "19800101-1231", 35_000 * KR),
        PayrollEvent::EmployeeDeactivated { employee_id: id },
    ]);
    assert_eq!(add_employee(&inactive, cmd(Uuid::new_v4())), Err(DomainError::DuplicateEmployee));
}

#[test]
fn a_monthly_salary_is_more_than_zero_and_at_most_the_ledgers_limit() {
    for bad in [0, -1, MAX_AMOUNT + 1] {
        let cmd = AddEmployee {
            employee_id: Uuid::new_v4(),
            name: name("Åsa Öberg"),
            personal_identity_number: pin("19800101-1231"),
            monthly_salary: bad,
            salary_account: SalaryAccount::DEFAULT,
        };
        assert_eq!(add_employee(&given(&[]), cmd), Err(DomainError::InvalidSalary), "{bad}");
    }
}

#[test]
fn an_active_employee_is_updated_and_deactivated() {
    let id = Uuid::new_v4();
    let payroll = given(&[hired(id, "19800101-1231", 35_000 * KR)]);
    let update = |employee_id, monthly_salary| UpdateEmployee {
        employee_id,
        name: name("Åsa Öberg"),
        monthly_salary,
        salary_account: SalaryAccount::parse(7220).unwrap(),
    };

    assert_eq!(
        update_employee(&payroll, update(id, 36_000 * KR)),
        Ok(vec![PayrollEvent::EmployeeUpdated {
            employee_id: id,
            name: name("Åsa Öberg"),
            monthly_salary: 36_000 * KR,
            salary_account: SalaryAccount::parse(7220).unwrap(),
        }])
    );
    assert_eq!(update_employee(&payroll, update(id, 0)), Err(DomainError::InvalidSalary));
    assert_eq!(
        update_employee(&payroll, update(Uuid::new_v4(), 36_000 * KR)),
        Err(DomainError::EmployeeNotFound)
    );
    // No change, no event.
    let same = UpdateEmployee {
        employee_id: id,
        name: name("Åsa Öberg"),
        monthly_salary: 35_000 * KR,
        salary_account: SalaryAccount::DEFAULT,
    };
    assert_eq!(update_employee(&payroll, same), Ok(vec![]));

    assert_eq!(
        deactivate_employee(&payroll, id),
        Ok(vec![PayrollEvent::EmployeeDeactivated { employee_id: id }])
    );
    assert_eq!(deactivate_employee(&payroll, Uuid::new_v4()), Err(DomainError::EmployeeNotFound));

    let inactive = given(&[
        hired(id, "19800101-1231", 35_000 * KR),
        PayrollEvent::EmployeeDeactivated { employee_id: id },
    ]);
    assert_eq!(deactivate_employee(&inactive, id), Ok(vec![]));
    assert_eq!(
        update_employee(&inactive, update(id, 36_000 * KR)),
        Err(DomainError::EmployeeInactive)
    );
    assert!(!inactive.employee(id).unwrap().active);
}
