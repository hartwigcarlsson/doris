use doris_payroll::domain::*;
use doris_payroll::tax::{RowKind, TaxBasis, TaxSetting, TaxTable, TaxTableRow};

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
        "19800101-1232", // check digit
        "800101-1231",   // ten digits: the century is unknown
        "8001011231",
        "19800230-1235", // 30 February
        "19800192-1231", // samordningsnummer day 92
        "19800101+1231",
        "1980010l-1231",
        "19800101–1231", // en dash: not ASCII, never sliced
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
    assert_eq!(
        EmployeeName::parse("  Åsa Öberg ").unwrap().as_str(),
        "Åsa Öberg"
    );
    assert!(EmployeeName::parse(&"å".repeat(100)).is_ok());
    for bad in ["", "   ", &"å".repeat(101)] {
        assert_eq!(
            EmployeeName::parse(bad),
            Err(DomainError::InvalidEmployeeName)
        );
    }
}

#[test]
fn salary_accounts_are_7010_7210_or_7220() {
    for ok in [7010, 7210, 7220] {
        assert_eq!(SalaryAccount::parse(ok).unwrap().get(), ok);
    }
    assert_eq!(SalaryAccount::DEFAULT.get(), 7210);
    for bad in [0, 7211, 1930, 7510] {
        assert_eq!(
            SalaryAccount::parse(bad),
            Err(DomainError::InvalidSalaryAccount)
        );
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
    assert_eq!(
        employer_fee(2005, may, 1_000 * KR, 25_000 * KR),
        (YOUTH_RATE, 31_420)
    );
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
        tax: None,
    };

    assert_eq!(
        add_employee(&given(&[]), cmd(id)),
        Ok(vec![hired(id, "19800101-1231", 35_000 * KR)])
    );

    let existing = given(&[hired(id, "19800101-1231", 35_000 * KR)]);
    assert_eq!(
        add_employee(&existing, cmd(Uuid::new_v4())),
        Err(DomainError::DuplicateEmployee)
    );
    // Also when the existing employee is inactive.
    let inactive = given(&[
        hired(id, "19800101-1231", 35_000 * KR),
        PayrollEvent::EmployeeDeactivated { employee_id: id },
    ]);
    assert_eq!(
        add_employee(&inactive, cmd(Uuid::new_v4())),
        Err(DomainError::DuplicateEmployee)
    );
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
            tax: None,
        };
        assert_eq!(
            add_employee(&given(&[]), cmd),
            Err(DomainError::InvalidSalary),
            "{bad}"
        );
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
    assert_eq!(
        update_employee(&payroll, update(id, 0)),
        Err(DomainError::InvalidSalary)
    );
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
    assert_eq!(
        deactivate_employee(&payroll, Uuid::new_v4()),
        Err(DomainError::EmployeeNotFound)
    );

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

use doris_ledger::domain::VoucherLine;

/// Events so far, plus the ledger vouchers that have been corrected.
#[derive(Default)]
struct World {
    events: Vec<PayrollEvent>,
    reversed: HashSet<BookedVoucher>,
}

impl World {
    fn payroll(&self) -> Payroll {
        Payroll::from_events(&self.events, self.reversed.clone())
    }

    fn then(&mut self, event: PayrollEvent) {
        self.events.push(event);
    }

    fn hire(&mut self, personnummer: &str, monthly_salary: i64) -> Uuid {
        let id = Uuid::new_v4();
        self.then(hired(id, personnummer, monthly_salary));
        id
    }

    fn create(&mut self, draft: PayrollRunDraft) -> Uuid {
        let id = Uuid::new_v4();
        let event = create_payroll_run(&self.payroll(), id, draft).unwrap();
        self.then(event);
        id
    }

    fn finalize(&mut self, run: Uuid) {
        let event = finalize_payroll_run(&self.payroll(), run, None).unwrap();
        self.then(event);
    }

    fn book(&mut self, run: Uuid, number: u32) -> BookedVoucher {
        let voucher = BookedVoucher {
            fiscal_year_start: date(2026, 1, 1),
            number,
        };
        self.then(booked(run, voucher));
        voucher
    }

    fn status(&self, run: Uuid) -> PayrollRunStatus {
        let payroll = self.payroll();
        payroll.status(payroll.run(run).unwrap())
    }
}

fn draft(pay_date: Date, lines: &[(Uuid, i64, i64)]) -> PayrollRunDraft {
    PayrollRunDraft {
        pay_date,
        text: String::new(),
        lines: lines
            .iter()
            .map(|&(employee_id, gross, tax)| DraftLine {
                employee_id,
                gross,
                tax: Some(tax),
            })
            .collect(),
    }
}

fn vl(account: u32, debit: i64, credit: i64) -> VoucherLine {
    VoucherLine {
        account: doris_ledger::domain::AccountNumber::parse(account).unwrap(),
        debit,
        credit,
    }
}

#[test]
fn a_draft_needs_active_employees_once_each_with_a_salary_and_a_tax() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let gone = w.hire("19850709-9870", 30_000 * KR);
    w.then(PayrollEvent::EmployeeDeactivated { employee_id: gone });
    let p = w.payroll();
    let oct = date(2026, 10, 25);
    let check = |lines: &[(Uuid, i64, i64)]| validate_draft(&p, draft(oct, lines)).map(|_| ());

    assert_eq!(check(&[]), Err(DomainError::EmptyPayrollRun));
    assert_eq!(
        check(&[(asa, 100, 0), (asa, 100, 0)]),
        Err(DomainError::DuplicatePayrollRunLine)
    );
    assert_eq!(
        check(&[(Uuid::new_v4(), 100, 0)]),
        Err(DomainError::EmployeeNotFound)
    );
    assert_eq!(check(&[(gone, 100, 0)]), Err(DomainError::EmployeeInactive));
    assert_eq!(check(&[(asa, 0, 0)]), Err(DomainError::InvalidSalary));
    assert_eq!(
        check(&[(asa, MAX_AMOUNT + 1, 0)]),
        Err(DomainError::InvalidSalary)
    );
    assert_eq!(check(&[(asa, 100, -1)]), Err(DomainError::InvalidTax));
    assert_eq!(check(&[(asa, 100, 101)]), Err(DomainError::InvalidTax));
    assert_eq!(check(&[(asa, 100, 100)]), Ok(()));

    let mut long = draft(oct, &[(asa, 100, 0)]);
    long.text = "å".repeat(201);
    assert_eq!(validate_draft(&p, long), Err(DomainError::InvalidText));
}

#[test]
fn an_empty_text_becomes_the_month_and_a_future_pay_date_is_fine() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let p = w.payroll();

    let valid = validate_draft(&p, draft(date(2030, 1, 25), &[(asa, 100, 0)])).unwrap();
    assert_eq!(valid.text, "Lön januari 2030");

    let mut typed = draft(date(2026, 12, 23), &[(asa, 100, 0)]);
    typed.text = "  Julbonus  ".into();
    assert_eq!(validate_draft(&p, typed).unwrap().text, "Julbonus");
    let mut blank = draft(date(2026, 12, 23), &[(asa, 100, 0)]);
    blank.text = "   ".into();
    assert_eq!(validate_draft(&p, blank).unwrap().text, "Lön december 2026");
}

#[test]
fn lines_carry_the_account_fee_and_net_pay() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let ung = w.hire("20050615-1232", 20_000 * KR);
    w.then(PayrollEvent::EmployeeUpdated {
        employee_id: ung,
        name: name("Unga Ung"),
        monthly_salary: 20_000 * KR,
        salary_account: SalaryAccount::parse(7010).unwrap(),
    });
    let d = draft(
        date(2026, 10, 25),
        &[
            (asa, 35_000 * KR, 8_000 * KR),
            (ung, 20_000 * KR, 3_500 * KR),
        ],
    );

    let lines = compute_lines(&w.payroll(), None, &d, None).unwrap();

    assert_eq!(
        lines,
        vec![
            PayrollRunLine {
                employee_id: asa,
                salary_account: SalaryAccount::DEFAULT,
                gross: 35_000 * KR,
                tax: 8_000 * KR,
                fee_rate: FULL_RATE,
                fee: 1_099_700,
                net: 27_000 * KR,
                tax_basis: TaxBasis::Manual,
            },
            PayrollRunLine {
                employee_id: ung,
                salary_account: SalaryAccount::parse(7010).unwrap(),
                gross: 20_000 * KR,
                tax: 3_500 * KR,
                fee_rate: YOUTH_RATE,
                fee: 416_200,
                net: 16_500 * KR,
                tax_basis: TaxBasis::Manual,
            },
        ]
    );
}

#[test]
fn the_voucher_balances_grouped_by_account_without_zero_lines() {
    let line = |account: u32, gross: i64, tax: i64, fee: i64| PayrollRunLine {
        employee_id: Uuid::new_v4(),
        salary_account: SalaryAccount::parse(account).unwrap(),
        gross,
        tax,
        fee_rate: FULL_RATE,
        fee,
        net: gross - tax,
        tax_basis: TaxBasis::Manual,
    };
    let lines = [
        line(7210, 35_000 * KR, 8_000 * KR, 1_099_700),
        line(7010, 20_000 * KR, 0, 628_400),
        line(7210, 10_000 * KR, 2_000 * KR, 314_200),
    ];

    let voucher = voucher_lines(&lines);

    assert_eq!(
        voucher,
        vec![
            vl(7010, 20_000 * KR, 0),
            vl(7210, 45_000 * KR, 0),
            vl(2710, 0, 10_000 * KR),
            vl(1930, 0, 55_000 * KR),
            vl(7510, 2_042_300, 0),
            vl(2731, 0, 2_042_300),
        ]
    );
    let debit: i64 = voucher.iter().map(|l| l.debit).sum();
    let credit: i64 = voucher.iter().map(|l| l.credit).sum();
    assert_eq!(debit, credit);

    // No tax and no fee (born 1937): only salary and bank.
    let free = [line(7210, 1_000 * KR, 0, 0)];
    assert_eq!(
        voucher_lines(&free),
        vec![vl(7210, 1_000 * KR, 0), vl(1930, 0, 1_000 * KR)]
    );
}

#[test]
fn a_run_goes_open_finalized_booked_and_back_when_its_voucher_is_corrected() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let run = w.create(draft(date(2026, 10, 25), &[(asa, 35_000 * KR, 8_000 * KR)]));
    assert_eq!(w.status(run), PayrollRunStatus::Open);

    let mut changed = draft(date(2026, 10, 24), &[(asa, 36_000 * KR, 8_200 * KR)]);
    changed.text = "Lön okt".into();
    let event = update_payroll_run(&w.payroll(), run, changed).unwrap();
    w.then(event);
    assert_eq!(
        w.payroll().run(run).unwrap().draft.lines[0].gross,
        36_000 * KR
    );

    w.finalize(run);
    assert_eq!(w.status(run), PayrollRunStatus::Finalized);
    assert!(w.payroll().run(run).unwrap().lines.is_some());

    let event = reopen_payroll_run(&w.payroll(), run).unwrap();
    w.then(event);
    assert_eq!(w.status(run), PayrollRunStatus::Open);
    assert_eq!(w.payroll().run(run).unwrap().lines, None);

    w.finalize(run);
    let first = w.book(run, 7);
    assert_eq!(w.status(run), PayrollRunStatus::Booked(first));
    assert_eq!(unbook_payroll_run(&w.payroll(), run), Ok(first));

    // The rättelse, whether from the payroll page or the grundbok.
    w.reversed.insert(first);
    assert_eq!(w.status(run), PayrollRunStatus::Finalized);

    let second = w.book(run, 9);
    assert_eq!(w.status(run), PayrollRunStatus::Booked(second));
    assert_eq!(w.payroll().run(run).unwrap().bookings, vec![first, second]);
}

#[test]
fn each_step_needs_the_right_status() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let d = || draft(date(2026, 10, 1), &[(asa, 35_000 * KR, 8_000 * KR)]);
    let today = date(2026, 10, 4);
    let missing = Uuid::new_v4();
    let p = w.payroll();
    assert_eq!(
        update_payroll_run(&p, missing, d()),
        Err(DomainError::PayrollRunNotFound)
    );
    assert_eq!(
        finalize_payroll_run(&p, missing, None),
        Err(DomainError::PayrollRunNotFound)
    );
    assert_eq!(
        reopen_payroll_run(&p, missing),
        Err(DomainError::PayrollRunNotFound)
    );
    assert_eq!(
        book_payroll_run(&p, missing, today),
        Err(DomainError::PayrollRunNotFound)
    );
    assert_eq!(
        unbook_payroll_run(&p, missing),
        Err(DomainError::PayrollRunNotFound)
    );

    let run = w.create(d());
    let p = w.payroll();
    assert_eq!(
        reopen_payroll_run(&p, run),
        Err(DomainError::PayrollRunNotFinalized)
    );
    assert_eq!(
        book_payroll_run(&p, run, today),
        Err(DomainError::PayrollRunNotFinalized)
    );
    assert_eq!(
        unbook_payroll_run(&p, run),
        Err(DomainError::PayrollRunNotBooked)
    );

    w.finalize(run);
    let p = w.payroll();
    assert_eq!(
        update_payroll_run(&p, run, d()),
        Err(DomainError::PayrollRunNotOpen)
    );
    assert_eq!(
        finalize_payroll_run(&p, run, None),
        Err(DomainError::PayrollRunNotOpen)
    );
    assert_eq!(
        unbook_payroll_run(&p, run),
        Err(DomainError::PayrollRunNotBooked)
    );

    w.book(run, 1);
    let p = w.payroll();
    assert_eq!(
        update_payroll_run(&p, run, d()),
        Err(DomainError::PayrollRunBooked)
    );
    assert_eq!(
        finalize_payroll_run(&p, run, None),
        Err(DomainError::PayrollRunBooked)
    );
    assert_eq!(
        reopen_payroll_run(&p, run),
        Err(DomainError::PayrollRunBooked)
    );
    assert_eq!(
        book_payroll_run(&p, run, today),
        Err(DomainError::PayrollRunBooked)
    );
}

#[test]
fn a_run_is_booked_on_its_pay_date_with_the_locked_lines() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let mut d = draft(date(2026, 10, 25), &[(asa, 35_000 * KR, 8_000 * KR)]);
    d.text = "Lön oktober".into();
    let run = w.create(d);
    w.finalize(run);

    assert_eq!(
        book_payroll_run(&w.payroll(), run, date(2026, 10, 24)),
        Err(DomainError::PayrollRunNotDue)
    );

    // Moved to 7220 and deactivated after finalizing: the locked account
    // is booked, and nothing stops the booking.
    w.then(PayrollEvent::EmployeeUpdated {
        employee_id: asa,
        name: name("Åsa Öberg"),
        monthly_salary: 35_000 * KR,
        salary_account: SalaryAccount::parse(7220).unwrap(),
    });
    w.then(PayrollEvent::EmployeeDeactivated { employee_id: asa });

    let voucher = book_payroll_run(&w.payroll(), run, date(2026, 10, 25)).unwrap();
    assert_eq!(voucher.date, date(2026, 10, 25));
    assert_eq!(voucher.text, "Lön oktober");
    assert_eq!(
        voucher.lines,
        vec![
            vl(7210, 35_000 * KR, 0),
            vl(2710, 0, 8_000 * KR),
            vl(1930, 0, 27_000 * KR),
            vl(7510, 1_099_700, 0),
            vl(2731, 0, 1_099_700),
        ]
    );
}

#[test]
fn an_open_run_with_a_deactivated_employee_cannot_be_finalized() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let run = w.create(draft(date(2026, 10, 25), &[(asa, 35_000 * KR, 8_000 * KR)]));
    w.then(PayrollEvent::EmployeeDeactivated { employee_id: asa });

    assert_eq!(
        finalize_payroll_run(&w.payroll(), run, None),
        Err(DomainError::EmployeeInactive)
    );
}

#[test]
fn the_youth_cap_counts_only_booked_runs_in_the_same_month() {
    let mut w = World::default();
    let ung = w.hire("20050615-1232", 20_000 * KR);
    let oct = |day| date(2026, 10, day);
    let other = w.create(draft(oct(10), &[(ung, 20_000 * KR, 0)]));
    w.finalize(other);
    let d = draft(oct(25), &[(ung, 20_000 * KR, 0)]);
    // Finalized but not booked: not counted.
    assert_eq!(
        compute_lines(&w.payroll(), None, &d, None).unwrap()[0].fee,
        416_200
    );

    let voucher = w.book(other, 1);
    // 5 000 kr left under the cap: 1 040,50 + 4 713 = 5 753,50 kr.
    assert_eq!(
        compute_lines(&w.payroll(), None, &d, None).unwrap()[0].fee,
        575_350
    );
    // Another month is not counted.
    let nov = draft(date(2026, 11, 25), &[(ung, 20_000 * KR, 0)]);
    assert_eq!(
        compute_lines(&w.payroll(), None, &nov, None).unwrap()[0].fee,
        416_200
    );

    w.reversed.insert(voucher);
    assert_eq!(
        compute_lines(&w.payroll(), None, &d, None).unwrap()[0].fee,
        416_200
    );
}

#[test]
fn a_booking_that_would_change_the_fee_is_outdated() {
    let mut w = World::default();
    let ung = w.hire("20050615-1232", 20_000 * KR);
    let late = w.create(draft(date(2026, 10, 3), &[(ung, 20_000 * KR, 0)]));
    w.finalize(late); // Fee 4 162 kr: nothing booked in October yet.
    let early = w.create(draft(date(2026, 10, 1), &[(ung, 20_000 * KR, 0)]));
    w.finalize(early);
    w.book(early, 1);

    assert_eq!(
        book_payroll_run(&w.payroll(), late, date(2026, 10, 4)),
        Err(DomainError::PayrollRunOutdated)
    );

    // Reopened and finalized again, the fee is recomputed and it books.
    let event = reopen_payroll_run(&w.payroll(), late).unwrap();
    w.then(event);
    w.finalize(late);
    assert!(book_payroll_run(&w.payroll(), late, date(2026, 10, 4)).is_ok());
}

/// Every table 29–42 with one amount band (1–80 000 kr: `kronor` in every
/// column) and one open 40 % band: enough for the domain, which only looks
/// rows up.
fn flat_table(year: i16, kronor: i64) -> TaxTable {
    let rows = (29..=42u8)
        .flat_map(|table| {
            [
                TaxTableRow {
                    table,
                    kind: RowKind::Amount,
                    from: 1,
                    to: Some(80_000),
                    columns: [kronor; 6],
                },
                TaxTableRow {
                    table,
                    kind: RowKind::Percent,
                    from: 80_001,
                    to: None,
                    columns: [40; 6],
                },
            ]
        })
        .collect();
    TaxTable::validate(year, rows).unwrap()
}

fn computed(employee_id: Uuid, gross: i64) -> DraftLine {
    DraftLine {
        employee_id,
        gross,
        tax: None,
    }
}

#[test]
fn an_employee_is_added_with_a_tax_setting_and_can_change_it() {
    let id = Uuid::new_v4();
    let t33 = TaxSetting::table(33, 1).unwrap();
    let cmd = AddEmployee {
        employee_id: id,
        name: name("Åsa Öberg"),
        personal_identity_number: pin("19800101-1231"),
        monthly_salary: 35_000 * KR,
        salary_account: SalaryAccount::DEFAULT,
        tax: Some(t33),
    };
    assert_eq!(
        add_employee(&given(&[]), cmd),
        Ok(vec![
            hired(id, "19800101-1231", 35_000 * KR),
            PayrollEvent::EmployeeTaxChanged {
                employee_id: id,
                tax: t33
            },
        ])
    );

    let p = given(&[
        hired(id, "19800101-1231", 35_000 * KR),
        PayrollEvent::EmployeeTaxChanged {
            employee_id: id,
            tax: t33,
        },
    ]);
    assert_eq!(p.employee(id).unwrap().tax, Some(t33));
    assert_eq!(set_employee_tax(&p, id, t33), Ok(vec![]));
    let thirty = TaxSetting::percent(30).unwrap();
    assert_eq!(
        set_employee_tax(&p, id, thirty),
        Ok(vec![PayrollEvent::EmployeeTaxChanged {
            employee_id: id,
            tax: thirty
        }])
    );
    assert_eq!(
        set_employee_tax(&p, Uuid::new_v4(), thirty),
        Err(DomainError::EmployeeNotFound)
    );
    let gone = given(&[
        hired(id, "19800101-1231", 35_000 * KR),
        PayrollEvent::EmployeeDeactivated { employee_id: id },
    ]);
    assert_eq!(
        set_employee_tax(&gone, id, thirty),
        Err(DomainError::EmployeeInactive)
    );
    assert_eq!(gone.employee(id).unwrap().tax, None);
}

#[test]
fn a_blank_tax_is_computed_from_the_setting_and_a_typed_one_is_manual() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let bo = w.hire("19850709-9870", 30_000 * KR);
    let ung = w.hire("20050615-1232", 20_000 * KR);
    w.then(PayrollEvent::EmployeeTaxChanged {
        employee_id: asa,
        tax: TaxSetting::table(33, 1).unwrap(),
    });
    w.then(PayrollEvent::EmployeeTaxChanged {
        employee_id: bo,
        tax: TaxSetting::percent(30).unwrap(),
    });
    let oct = date(2026, 10, 25);
    let table = flat_table(2026, 7_000);
    let d = PayrollRunDraft {
        pay_date: oct,
        text: String::new(),
        lines: vec![
            computed(asa, 35_000 * KR),
            computed(bo, 30_000 * KR),
            DraftLine {
                employee_id: ung,
                gross: 20_000 * KR,
                tax: Some(3_500 * KR),
            },
        ],
    };

    let lines = compute_lines(&w.payroll(), None, &d, Some(&table)).unwrap();

    let taxes: Vec<_> = lines.iter().map(|l| (l.tax, l.tax_basis, l.net)).collect();
    assert_eq!(
        taxes,
        vec![
            (
                7_000 * KR,
                TaxBasis::Table {
                    year: 2026,
                    table: 33,
                    column: 1
                },
                28_000 * KR
            ),
            (9_000 * KR, TaxBasis::Percent { percent: 30 }, 21_000 * KR),
            (3_500 * KR, TaxBasis::Manual, 16_500 * KR),
        ]
    );
}

#[test]
fn a_blank_tax_without_a_setting_or_a_table_is_refused() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let bo = w.hire("19850709-9870", 30_000 * KR);
    w.then(PayrollEvent::EmployeeTaxChanged {
        employee_id: bo,
        tax: TaxSetting::table(33, 1).unwrap(),
    });
    let oct = date(2026, 10, 25);
    let one = |line| PayrollRunDraft {
        pay_date: oct,
        text: String::new(),
        lines: vec![line],
    };

    // A blank tax is a valid draft; it is computed when the lines are.
    assert!(validate_draft(&w.payroll(), one(computed(asa, 100))).is_ok());
    assert_eq!(
        compute_lines(&w.payroll(), None, &one(computed(asa, 35_000 * KR)), None),
        Err(DomainError::TaxRequired)
    );
    assert_eq!(
        compute_lines(&w.payroll(), None, &one(computed(bo, 30_000 * KR)), None),
        Err(DomainError::TaxTableMissing(2026))
    );
    // Last year's table is not this year's.
    assert_eq!(
        compute_lines(
            &w.payroll(),
            None,
            &one(computed(bo, 30_000 * KR)),
            Some(&flat_table(2025, 1))
        ),
        Err(DomainError::TaxTableMissing(2026))
    );
    // A typed tax keeps its old checks.
    assert_eq!(
        validate_draft(
            &w.payroll(),
            one(DraftLine {
                employee_id: asa,
                gross: 100,
                tax: Some(101)
            })
        ),
        Err(DomainError::InvalidTax)
    );
}

#[test]
fn finalizing_locks_the_computed_tax_and_booking_keeps_it() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    w.then(PayrollEvent::EmployeeTaxChanged {
        employee_id: asa,
        tax: TaxSetting::table(33, 1).unwrap(),
    });
    let d = PayrollRunDraft {
        pay_date: date(2026, 10, 25),
        text: String::new(),
        lines: vec![computed(asa, 35_000 * KR)],
    };
    let run = Uuid::new_v4();
    let event = create_payroll_run(&w.payroll(), run, d).unwrap();
    w.then(event);
    assert_eq!(
        finalize_payroll_run(&w.payroll(), run, None),
        Err(DomainError::TaxTableMissing(2026))
    );
    let event = finalize_payroll_run(&w.payroll(), run, Some(&flat_table(2026, 7_000))).unwrap();
    w.then(event);
    // A later change of setting does not touch the locked line.
    w.then(PayrollEvent::EmployeeTaxChanged {
        employee_id: asa,
        tax: TaxSetting::percent(50).unwrap(),
    });

    let voucher = book_payroll_run(&w.payroll(), run, date(2026, 10, 25)).unwrap();

    let locked = w.payroll().run(run).unwrap().lines.clone().unwrap();
    assert_eq!(
        (locked[0].tax, locked[0].tax_basis),
        (
            7_000 * KR,
            TaxBasis::Table {
                year: 2026,
                table: 33,
                column: 1
            }
        )
    );
    assert!(
        voucher
            .lines
            .iter()
            .any(|l| l.account.get() == 2710 && l.credit == 7_000 * KR)
    );
}

#[test]
fn events_from_before_tax_settings_read_as_manual() {
    let line: PayrollRunLine = serde_json::from_str(
        r#"{"employee_id":"7d0e1a2b-0000-4000-8000-000000000001","salary_account":7210,
            "gross":3500000,"tax":800000,"fee_rate":3142,"fee":1099700,"net":2700000}"#,
    )
    .unwrap();
    assert_eq!(line.tax_basis, TaxBasis::Manual);
    let draft: DraftLine = serde_json::from_str(
        r#"{"employee_id":"7d0e1a2b-0000-4000-8000-000000000001","gross":3500000,"tax":800000}"#,
    )
    .unwrap();
    assert_eq!(draft.tax, Some(800_000));
}

#[test]
fn a_salary_splits_into_fee_bases() {
    let oct = date(2026, 10, 25);
    assert_eq!(
        fee_bases(1980, oct, 35_000 * KR, 0),
        [(FULL_RATE, 35_000 * KR), (FULL_RATE, 0)]
    );
    assert_eq!(
        fee_bases(1950, oct, 20_000 * KR, 0),
        [(OLD_AGE_RATE, 20_000 * KR), (FULL_RATE, 0)]
    );
    assert_eq!(
        fee_bases(1937, oct, 20_000 * KR, 0),
        [(0, 20_000 * KR), (FULL_RATE, 0)]
    );
    assert_eq!(
        fee_bases(2005, oct, 30_000 * KR, 0),
        [(YOUTH_RATE, 25_000 * KR), (FULL_RATE, 5_000 * KR)]
    );
    assert_eq!(
        fee_bases(2005, oct, 20_000 * KR, 20_000 * KR),
        [(YOUTH_RATE, 5_000 * KR), (FULL_RATE, 15_000 * KR)]
    );
}
